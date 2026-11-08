use rustune::{
    backend::{cdp::read_message, validate_api_path},
    config::{Config, Paths},
    ipc::{MAX_FRAME, read_frame},
    model::{Quality, quality_supported},
};
#[test]
fn cdp_frames_require_terminator_and_bound() {
    assert!(read_message(&mut std::io::Cursor::new(b"{}".to_vec())).is_err());
    assert!(read_message(&mut std::io::Cursor::new(vec![b'a'; MAX_FRAME + 1])).is_err());
    assert_eq!(
        read_message(&mut std::io::Cursor::new(b"{}\0".to_vec())).unwrap(),
        b"{}"
    );
}
#[tokio::test]
async fn ipc_rejects_oversized_length_before_allocation() {
    let bytes = ((MAX_FRAME + 1) as u32).to_be_bytes();
    let result = read_frame(&mut &bytes[..]).await;
    assert!(result.unwrap_err().to_string().contains("limit"));
}
#[test]
fn pagination_cannot_send_credentials_to_another_host() {
    for bad in [
        "https://evil.test/",
        "//evil.test/",
        "/v1/me/../tokens",
        "/v1/me?secret=1",
        "/v1/me\r\nX: y",
    ] {
        assert!(validate_api_path(bad).is_err(), "{bad}");
    }
    assert!(validate_api_path("/v1/me/library/songs?offset=25&limit=25").is_ok());
}
#[test]
fn unsupported_quality_is_never_advertised() {
    assert!(quality_supported(Quality::Auto));
    assert!(!quality_supported(Quality::Lossless));
    assert!(!quality_supported(Quality::HiResLossless));
}

#[test]
fn quality_target_is_never_presented_as_a_measured_source_format() {
    let unknown: rustune::model::Playback = serde_json::from_value(serde_json::json!({})).unwrap();
    assert_eq!(unknown.quality_label(), "Source quality unknown");
    let reported: rustune::model::Playback =
        serde_json::from_value(serde_json::json!({"target_bitrate_kbps":256})).unwrap();
    assert_eq!(
        reported.quality_label(),
        "MusicKit target 256 kb/s · source quality unknown"
    );
}

#[tokio::test]
async fn default_backend_does_not_contact_or_start_a_browser() {
    use rustune::backend::musickit::MusicKit;
    let d = tempfile::tempdir().unwrap();
    let paths = Paths {
        config: d.path().join("config"),
        data: d.path().join("data"),
        cache: d.path().join("cache"),
        runtime: d.path().join("run"),
    };
    paths.init().unwrap();
    let mut backend = MusicKit::new(Config::default(), paths);
    let error = backend.start(false).await.unwrap_err().to_string();
    assert!(error.contains("Native Linux playback"), "{error}");
    assert_eq!(backend.pid(), None);
}
#[test]
fn token_permissions_and_state_writes_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let paths = Paths {
        config: d.path().join("config"),
        data: d.path().join("data"),
        cache: d.path().join("cache"),
        runtime: d.path().join("run"),
    };
    paths.init().unwrap();
    Config::default().save(&paths).unwrap();
    assert_eq!(
        std::fs::metadata(paths.config.join("config.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let token = paths.config.join("developer-token");
    std::fs::write(&token, "not-a-developer-token").unwrap();
    assert!(paths.token().is_err());
}

#[tokio::test]
async fn failed_browser_start_releases_the_local_origin_for_retry() {
    use rustune::{backend::musickit::MusicKit, config::BackendKind};
    let d = tempfile::tempdir().unwrap();
    let paths = Paths {
        config: d.path().join("config"),
        data: d.path().join("data"),
        cache: d.path().join("cache"),
        runtime: d.path().join("run"),
    };
    paths.init().unwrap();
    rustune::config::private_write(
        &paths.config.join("developer-token"),
        b"aaaaaaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbbbbbb.cccccccccccccccccccc",
    )
    .unwrap();
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let config = Config {
        backend: BackendKind::MusicKit,
        browser: Some("/bin/false".into()),
        bridge_port: port,
        ..Config::default()
    };
    let mut backend = MusicKit::new(config, paths);
    assert!(backend.start(false).await.is_err());
    assert!(
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).is_ok(),
        "failed initialization must release the local origin"
    );
}
