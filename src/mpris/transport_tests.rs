//! Real D-Bus methods and signals backed by deterministic helper events.
use super::*;
use crate::{
    backend::{testing::Controlled, worker::Event},
    config::{Config, Paths},
    ipc,
    model::Track,
    service,
};
use futures_util::StreamExt;
use std::{process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::AsyncBufReadExt,
    time::{sleep, timeout},
};

#[tokio::test]
async fn seek_signals_and_navigation_on_a_private_bus() {
    timeout(Duration::from_secs(10), exercise_transport())
        .await
        .expect("private bus transport timed out");
}

async fn exercise_transport() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths {
        config: dir.path().join("cfg"),
        cache: dir.path().join("cache"),
        data: dir.path().join("data"),
        runtime: dir.path().join("run"),
    };
    let mut bus = tokio::process::Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address"])
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let address = tokio::io::BufReader::new(bus.stdout.take().unwrap())
        .lines()
        .next_line()
        .await
        .unwrap()
        .unwrap();
    let client = zbus::connection::Builder::address(address.as_str())
        .build()
        .await
        .unwrap();
    let driver = Arc::new(Controlled::default());
    let daemon = tokio::spawn(service::run_with(
        Config::default(),
        paths.clone(),
        driver.clone(),
        None,
        Some(address),
    ));
    while !paths.socket().exists() {
        sleep(Duration::from_millis(5)).await;
    }
    for (id, play) in [("7", true), ("8", false)] {
        ipc::request(
            &paths,
            &Request::Enqueue {
                track: Track {
                    id: id.into(),
                    kind: "songs".into(),
                    ..Track::default()
                },
                next: false,
                play,
            },
        )
        .await
        .unwrap();
    }
    let player = zbus::Proxy::new(
        &client,
        "org.mpris.MediaPlayer2.rustune",
        PATH,
        "org.mpris.MediaPlayer2.Player",
    )
    .await
    .unwrap();
    let props = zbus::Proxy::new(
        &client,
        "org.mpris.MediaPlayer2.rustune",
        PATH,
        "org.freedesktop.DBus.Properties",
    )
    .await
    .unwrap();
    let mut changed = props.receive_signal("PropertiesChanged").await.unwrap();
    player.set_property("Volume", 0.7001f64).await.unwrap();
    player.set_property("Volume", 0.7001f64).await.unwrap();
    let mut volume_events = Vec::new();
    while let Ok(Some(message)) = timeout(Duration::from_millis(75), changed.next()).await {
        let (_, values, _): (String, HashMap<String, OwnedValue>, Vec<String>) =
            message.body().deserialize().unwrap();
        if let Some(v) = values.get("Volume") {
            volume_events.push(f64::try_from(v.clone()).unwrap());
        }
        assert!(!values.contains_key("CanControl") && !values.contains_key("Position"));
    }
    assert_eq!(volume_events, vec![0.7001]);
    let mut signals = player.receive_signal("Seeked").await.unwrap();
    driver
        .emit(Event::Ready {
            duration: 60.0,
            sample_rate: 44100,
            channels: 2,
        })
        .await;
    let state = |playing, position| Event::State {
        playing,
        position,
        encrypted_packets: 10,
        decoded_frames: 10,
        nonzero_samples: 20,
    };
    driver.emit(state(true, 10.0)).await;
    loop {
        if ipc::request(&paths, &Request::Status).await.unwrap()["playback"]["position"] == 10.0 {
            break;
        }
    }
    for (id, offset, position) in [(1, 1_000_000i64, 11.0), (2, -1_000_000, 10.0)] {
        if id == 2 {
            player.call::<_, _, ()>("Pause", &()).await.unwrap();
            driver.emit(state(false, 11.0)).await;
        }
        player.call::<_, _, ()>("Seek", &offset).await.unwrap();
        assert!(
            timeout(Duration::from_millis(50), signals.next())
                .await
                .is_err(),
            "no signal before application"
        );
        driver
            .emit(Event::SeekApplied {
                request_id: id,
                position,
            })
            .await;
        let signal = signals.next().await.unwrap();
        assert_eq!(
            signal.body().deserialize::<i64>().unwrap(),
            (position * 1_000_000.0) as i64
        );
    }
    assert_eq!(
        player
            .get_property::<String>("PlaybackStatus")
            .await
            .unwrap(),
        "Paused"
    );
    let status: Status =
        serde_json::from_value(ipc::request(&paths, &Request::Status).await.unwrap()).unwrap();
    let metadata: HashMap<String, OwnedValue> = player.get_property("Metadata").await.unwrap();
    assert_eq!(
        i64::try_from(metadata["mpris:length"].clone()).unwrap(),
        60_000_000
    );
    let track = track_path(&status);
    for position in [-1, 60_000_001] {
        player
            .call::<_, _, ()>("SetPosition", &(track.clone(), position as i64))
            .await
            .unwrap();
    }
    player
        .call::<_, _, ()>("SetPosition", &(track.clone(), 12_000_000i64))
        .await
        .unwrap();
    driver
        .emit(Event::SeekApplied {
            request_id: 3,
            position: 12.0,
        })
        .await;
    assert_eq!(
        signals
            .next()
            .await
            .unwrap()
            .body()
            .deserialize::<i64>()
            .unwrap(),
        12_000_000
    );
    // Beyond EOF has exactly the navigation semantics of Next, including paused preparation.
    player
        .call::<_, _, ()>("Seek", &65_000_000i64)
        .await
        .unwrap();
    assert_eq!(driver.starts().last().unwrap(), &("8".into(), true));
    driver
        .emit(Event::Ready {
            duration: 60.0,
            sample_rate: 44100,
            channels: 2,
        })
        .await;
    driver.emit(state(false, 8.0)).await;
    assert_eq!(
        player
            .get_property::<String>("PlaybackStatus")
            .await
            .unwrap(),
        "Paused"
    );
    let commands = driver.commands();
    player
        .call::<_, _, ()>("SetPosition", &(track, 5_000_000i64))
        .await
        .unwrap();
    assert_eq!(
        driver.commands(),
        commands,
        "old track path must be ignored"
    );
    player.call::<_, _, ()>("Previous", &()).await.unwrap();
    assert_eq!(driver.starts().last().unwrap(), &("7".into(), true));
    player.call::<_, _, ()>("Next", &()).await.unwrap();
    assert_eq!(driver.starts().last().unwrap(), &("8".into(), true));
    player.call::<_, _, ()>("Previous", &()).await.unwrap();
    assert_eq!(driver.starts().last().unwrap(), &("7".into(), true));
    player.call::<_, _, ()>("Stop", &()).await.unwrap();
    player.call::<_, _, ()>("Next", &()).await.unwrap();
    let stopped = ipc::request(&paths, &Request::Status).await.unwrap();
    assert!(stopped["audio_pid"].is_null());
    assert_eq!(stopped["playback"]["track"]["id"], "8");
    player.call::<_, _, ()>("Play", &()).await.unwrap();
    assert_eq!(driver.starts().last().unwrap(), &("8".into(), false));
    assert_eq!(
        ipc::request(&paths, &Request::Status).await.unwrap()["playback"]["position"],
        0.0
    );
    ipc::request(&paths, &Request::Shutdown).await.unwrap();
    daemon.await.unwrap().unwrap();
}
