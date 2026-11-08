use rustune::{
    config::{Config, Paths},
    ipc::{MAX_FRAME, read_frame},
    model::{Quality, quality_supported},
};
#[tokio::test]
async fn ipc_rejects_oversized_length_before_allocation() {
    let bytes = ((MAX_FRAME + 1) as u32).to_be_bytes();
    let result = read_frame(&mut &bytes[..]).await;
    assert!(result.unwrap_err().to_string().contains("limit"));
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
        "Reported target 256 kb/s · source quality unknown"
    );
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
}
