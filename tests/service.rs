use rustune::{
    config::{Config, Paths, PerformanceMode},
    ipc::{self, Request},
    model::Track,
};
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    time::Duration,
};
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[tokio::test]
async fn real_service_persists_queue_pushes_updates_and_obeys_mpris() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths {
        config: dir.path().join("config/rustune"),
        data: dir.path().join("data/rustune"),
        cache: dir.path().join("cache/rustune"),
        runtime: dir.path().join("run/rustune"),
    };
    paths.init().unwrap();
    let config = Config::default();
    config.save(&paths).unwrap();
    let mut bus = Process(
        Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut address = String::new();
    BufReader::new(bus.0.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    let address = address.trim();
    let spawn = || {
        Process(
            Command::new(env!("CARGO_BIN_EXE_rustune"))
                .arg("daemon")
                .env("XDG_CONFIG_HOME", dir.path().join("config"))
                .env("XDG_DATA_HOME", dir.path().join("data"))
                .env("XDG_CACHE_HOME", dir.path().join("cache"))
                .env("XDG_RUNTIME_DIR", dir.path().join("run"))
                .env("DBUS_SESSION_BUS_ADDRESS", address)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        )
    };
    let mut daemon = spawn();
    for _ in 0..100 {
        if ipc::request(&paths, &Request::Status).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let status = ipc::request(&paths, &Request::Status).await.unwrap();
    assert_eq!(
        status["playback"]["volume"], 0.7,
        "idle volume must match config"
    );
    assert_eq!(
        status["browser_pid"],
        serde_json::Value::Null,
        "idle must not launch Chromium"
    );
    let mut stream = tokio::net::UnixStream::connect(paths.socket())
        .await
        .unwrap();
    ipc::write_frame(&mut stream, &Request::Subscribe)
        .await
        .unwrap();
    ipc::read_frame(&mut stream).await.unwrap();
    ipc::request(&paths, &Request::Profile(PerformanceMode::Minimal))
        .await
        .unwrap();
    let update = tokio::time::timeout(Duration::from_secs(2), ipc::read_frame(&mut stream))
        .await
        .unwrap()
        .unwrap();
    assert!(String::from_utf8(update).unwrap().contains("minimal"));
    drop(stream);
    for request in [
        Request::Artwork(false),
        Request::Lyrics(false),
        Request::CatalogCountry("US".into()),
    ] {
        ipc::request(&paths, &request).await.unwrap();
    }
    let updated = Config::load(&paths).unwrap();
    assert!(!updated.artwork && !updated.lyrics);
    assert_eq!(updated.catalog_country, "US");
    assert_eq!(
        ipc::request(&paths, &Request::Status).await.unwrap()["catalog_country"],
        "US"
    );
    for request in [
        Request::Artwork(true),
        Request::Lyrics(true),
        Request::CatalogCountry("PL".into()),
    ] {
        ipc::request(&paths, &request).await.unwrap();
    }
    for id in ["a", "b", "c"] {
        let track = Track {
            id: id.into(),
            kind: "songs".into(),
            title: id.into(),
            ..Track::default()
        };
        ipc::request(
            &paths,
            &Request::Enqueue {
                track,
                next: false,
                play: false,
            },
        )
        .await
        .unwrap();
    }
    ipc::request(&paths, &Request::Move { from: 2, to: 0 })
        .await
        .unwrap();
    let page = ipc::request(&paths, &Request::QueuePage { offset: 0 })
        .await
        .unwrap();
    assert_eq!(page["items"][0]["id"], "c");
    assert!(
        ipc::request(&paths, &Request::Play).await.is_err(),
        "missing native engine must fail clearly"
    );
    assert_eq!(
        ipc::request(&paths, &Request::Status).await.unwrap()["browser_pid"],
        serde_json::Value::Null
    );
    let get = Command::new("busctl")
        .arg(format!("--address={address}"))
        .args([
            "get-property",
            "org.mpris.MediaPlayer2.rustune",
            "/org/mpris/MediaPlayer2",
            "org.mpris.MediaPlayer2.Player",
            "Volume",
        ])
        .output()
        .unwrap();
    assert!(
        get.status.success(),
        "{}",
        String::from_utf8_lossy(&get.stderr)
    );
    assert!(String::from_utf8_lossy(&get.stdout).contains("0.7"));
    let result = tokio::process::Command::new("busctl")
        .arg(format!("--address={address}"))
        .args([
            "set-property",
            "org.mpris.MediaPlayer2.rustune",
            "/org/mpris/MediaPlayer2",
            "org.mpris.MediaPlayer2.Player",
            "Shuffle",
            "b",
            "true",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        ipc::request(&paths, &Request::Status).await.unwrap()["shuffle"],
        true
    );
    ipc::request(&paths, &Request::Stop).await.unwrap();
    for _ in 0..100 {
        if daemon.0.try_wait().unwrap().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let _daemon = spawn();
    for _ in 0..100 {
        if ipc::request(&paths, &Request::Status).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let restored = ipc::request(&paths, &Request::QueuePage { offset: 0 })
        .await
        .unwrap();
    assert_eq!(restored["items"][0]["id"], "c");
    assert_eq!(restored["total"], 3);
    ipc::request(&paths, &Request::Stop).await.unwrap();
}
