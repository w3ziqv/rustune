//! Real UI intents → Unix IPC → service → controlled worker, no Apple account/CDM.
use super::*;
use crate::{
    backend::{testing::Controlled, worker::Event as AudioEvent},
    catalog::Catalog,
};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
fn key(app: &mut App, code: KeyCode) -> Option<Request> {
    app.key(crossterm::event::KeyEvent::new(code, KeyModifiers::NONE))
        .unwrap()
        .1
}
async fn status(paths: &Paths) -> Status {
    serde_json::from_value(ipc::request(paths, &Request::Status).await.unwrap()).unwrap()
}
async fn wait(paths: &Paths, predicate: impl Fn(&Status) -> bool) -> Status {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let s = status(paths).await;
            if predicate(&s) {
                return s;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("service reached expected state")
}
#[tokio::test]
async fn search_open_one_album_action_and_every_eof_crosses_all_45_tracks() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths {
        config: dir.path().join("config"),
        data: dir.path().join("data"),
        cache: dir.path().join("cache"),
        runtime: dir.path().join("run"),
    };
    paths.init().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let http = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 8192];
            let n = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            let album = serde_json::json!({"wrapperType":"collection","collectionType":"Album","collectionId":42,"collectionName":"Synthetic Album","trackCount":45});
            let body=if request.contains("/search?"){serde_json::json!({"results":[album]})}else{let mut results=vec![album];results.extend((1..=45).map(|i|serde_json::json!({"kind":"song","trackId":i,"trackName":format!("Song {i}"),"discNumber":if i<=25 {1}else{2},"trackNumber":if i<=25{i}else{i-25},"collectionId":42})));serde_json::json!({"results":results})}.to_string();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    let mut bus = tokio::process::Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut address = String::new();
    tokio::io::BufReader::new(bus.stdout.take().unwrap())
        .read_line(&mut address)
        .await
        .unwrap();
    let address = address.trim().to_owned();
    let factory = Arc::new(Controlled::default());
    let catalog = Arc::new(Catalog::with_origin(&origin));
    let service = tokio::spawn(crate::service::run_with(
        Config::default(),
        paths.clone(),
        factory.clone(),
        Some(catalog),
        Some(address.clone()),
    ));
    for _ in 0..100 {
        if paths.socket().exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let mut app = App::new(Config::default());
    app.screen = 2;
    app.sidebar = false;
    app.kind = "search:albums".into();
    app.query = "Synthetic".into();
    app.on_status(status(&paths).await);
    let search = app.page_request();
    app.accept(&search, ipc::request(&paths, &search).await.unwrap())
        .unwrap();
    assert_eq!(app.items[0].id, "42");
    // Open leaves audio alone and initially displays only 25 of 45 songs.
    let open = key(&mut app, KeyCode::Enter).unwrap();
    app.accept(&open, ipc::request(&paths, &open).await.unwrap())
        .unwrap();
    assert_eq!(app.items.len(), 25);
    assert!(factory.starts().is_empty());
    let mut monitor = tokio::process::Command::new("busctl")
        .arg(format!("--address={address}"))
        .args(["monitor", "org.mpris.MediaPlayer2.rustune"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let output = monitor.stdout.take().unwrap();
    let signals = tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(output).lines();
        let mut seen = std::collections::HashSet::new();
        while let Ok(Some(line)) = lines.next_line().await {
            for property in [
                "PlaybackStatus",
                "Metadata",
                "CanGoNext",
                "CanPause",
                "CanSeek",
            ] {
                if line.contains(property) {
                    seen.insert(property);
                }
            }
        }
        seen
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    key(&mut app, KeyCode::Char('a'));
    let play = key(&mut app, KeyCode::Enter).unwrap();
    ipc::request(&paths, &play).await.unwrap();
    wait(&paths, |s| {
        s.queue_len == 45 && s.playback.track.as_ref().is_some_and(|t| t.id == "1")
    })
    .await;
    for id in 1..=45 {
        let s = wait(&paths, |s| {
            s.playback
                .track
                .as_ref()
                .is_some_and(|t| t.id == id.to_string())
        })
        .await;
        assert_eq!(s.queue_len, 45);
        factory
            .emit(AudioEvent::Ready {
                duration: 60.0,
                sample_rate: 44100,
                channels: 2,
            })
            .await;
        factory
            .emit(AudioEvent::State {
                playing: true,
                position: 1.0,
                encrypted_packets: 1,
                decoded_frames: 1,
                nonzero_samples: 1,
            })
            .await;
        wait(&paths, |s| s.playback.playing).await;
        factory
            .emit(AudioEvent::Ended {
                position: 60.0,
                encrypted_packets: 60,
                decoded_frames: 60,
                nonzero_samples: 60,
            })
            .await;
    }
    wait(&paths, |s| {
        s.playback.phase == crate::model::PlaybackPhase::Stopped
    })
    .await;
    assert_eq!(factory.starts().len(), 45);
    let property = tokio::process::Command::new("busctl")
        .arg(format!("--address={address}"))
        .args([
            "get-property",
            "org.mpris.MediaPlayer2.rustune",
            "/org/mpris/MediaPlayer2",
            "org.mpris.MediaPlayer2.Player",
            "PlaybackStatus",
        ])
        .output()
        .await
        .unwrap();
    assert!(property.status.success());
    assert!(String::from_utf8_lossy(&property.stdout).contains("Stopped"));
    monitor.kill().await.unwrap();
    let _ = monitor.wait().await;
    let seen = signals.await.unwrap();
    for property in [
        "PlaybackStatus",
        "Metadata",
        "CanGoNext",
        "CanPause",
        "CanSeek",
    ] {
        assert!(seen.contains(property), "Missing MPRIS signal {property}");
    }
    ipc::request(&paths, &play).await.unwrap();
    assert_eq!(status(&paths).await.queue_len, 45);
    assert_eq!(
        factory.starts().len(),
        45,
        "Retry of committed operation does not replay album"
    );
    ipc::request(&paths, &Request::Shutdown).await.unwrap();
    service.await.unwrap().unwrap();
    http.abort();
    bus.kill().await.unwrap();
    let _ = bus.wait().await;
}
#[tokio::test]
async fn playlist_ui_ipc_full_context_private_save_and_every_eof() {
    use crate::ipc::PlaylistWriteSource;
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths {
        config: dir.path().join("c"),
        data: dir.path().join("d"),
        cache: dir.path().join("a"),
        runtime: dir.path().join("r"),
    };
    paths.init().unwrap();
    let region = serde_json::json!({"data":[{"id":"pl"}]});
    let list = serde_json::json!({"data":[{"id":"p.fixture","type":"library-playlists","attributes":{"name":"Synthetic","canEdit":true}}]});
    let page = |offset: usize| {
        let records=(offset..(offset+25).min(45)).map(|i|serde_json::json!({"id":format!("i.{i}"),"type":"library-songs","attributes":{"name":format!("Song {i}"),"playParams":{"catalogId":if i==26{"26".to_string()}else{(i+1).to_string()}}}})).collect::<Vec<_>>();
        let mut body = serde_json::json!({"data":records,"meta":{"total":45}});
        if offset == 0 {
            body["next"] = serde_json::json!("untrusted-url-not-followed");
        }
        body
    };
    let (_cd, catalog, http) = crate::catalog::playlist_write_tests::server(vec![
        (200, Some(region.clone())),
        (200, Some(list)),
        (200, Some(page(0))),
        (200, Some(page(0))),
        (200, Some(page(25))),
        (200, Some(region)),
        (
            201,
            Some(serde_json::json!({"data":[{"id":"p.created","type":"library-playlists"}]})),
        ),
    ])
    .await;
    let mut bus = tokio::process::Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut address = String::new();
    tokio::io::BufReader::new(bus.stdout.take().unwrap())
        .read_line(&mut address)
        .await
        .unwrap();
    let factory = Arc::new(Controlled::default());
    let service = tokio::spawn(crate::service::run_with(
        Config::default(),
        paths.clone(),
        factory.clone(),
        Some(Arc::new(catalog)),
        Some(address.trim().into()),
    ));
    for _ in 0..100 {
        if paths.socket().exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let mut app = App::new(Config::default());
    app.on_status(status(&paths).await);
    let browse = app.go(6).unwrap();
    app.accept(&browse, ipc::request(&paths, &browse).await.unwrap())
        .unwrap();
    assert_eq!(app.items[0].can_edit, Some(true));
    let open = key(&mut app, KeyCode::Enter).unwrap();
    app.accept(&open, ipc::request(&paths, &open).await.unwrap())
        .unwrap();
    assert_eq!(app.items.len(), 25);
    assert!(factory.starts().is_empty());
    key(&mut app, KeyCode::Char('a'));
    let play = key(&mut app, KeyCode::Enter).unwrap();
    ipc::request(&paths, &play).await.unwrap();
    let s = wait(&paths, |s| s.queue_len == 45).await;
    let revision = s.queue_revision;
    app.on_status(s);
    key(&mut app, KeyCode::Char('S'));
    app.playlist_paste("Synthetic export");
    assert!(key(&mut app, KeyCode::Enter).is_none());
    let save = key(&mut app, KeyCode::Enter).unwrap();
    assert!(matches!(
        save,
        Request::PlaylistWrite {
            source: PlaylistWriteSource::Queue { .. },
            ..
        }
    ));
    ipc::request(&paths, &save).await.unwrap();
    // Account I/O is independent of the engine's transport/status handling.
    tokio::time::timeout(
        Duration::from_millis(300),
        ipc::request(&paths, &Request::Volume(0.4)),
    )
    .await
    .unwrap()
    .unwrap();
    let s = wait(&paths, |s| {
        s.playlist_write
            .as_ref()
            .is_some_and(|w| w.phase == "completed")
    })
    .await;
    assert_eq!(s.queue_revision, revision);
    assert_eq!(s.queue_len, 45);
    assert_eq!(factory.starts().len(), 1);
    let requests = http.await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r["request"].as_str().unwrap().starts_with("POST"))
            .count(),
        1
    );
    let exported = &requests[6]["body"]["relationships"]["tracks"]["data"];
    assert_eq!(exported.as_array().unwrap().len(), 45);
    assert_eq!(exported[25]["id"], exported[26]["id"]);
    for index in 0..45 {
        let expected = if index == 26 {
            "26".to_string()
        } else {
            (index + 1).to_string()
        };
        wait(&paths, |s| {
            s.current == Some(index) && s.playback.track.as_ref().is_some_and(|t| t.id == expected)
        })
        .await;
        factory
            .emit(AudioEvent::Ended {
                position: 60.0,
                encrypted_packets: 1,
                decoded_frames: 1,
                nonzero_samples: 1,
            })
            .await;
    }
    wait(&paths, |s| {
        s.playback.phase == crate::model::PlaybackPhase::Stopped
    })
    .await;
    assert_eq!(factory.starts().len(), 45);
    ipc::request(&paths, &save).await.unwrap();
    wait(&paths, |s| {
        s.playlist_write
            .as_ref()
            .is_some_and(|w| w.phase == "completed")
    })
    .await;
    assert_eq!(factory.starts().len(), 45);
    ipc::request(&paths, &Request::Shutdown).await.unwrap();
    service.await.unwrap().unwrap();
    bus.kill().await.unwrap();
    let _ = bus.wait().await;
}
