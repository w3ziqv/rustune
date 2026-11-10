use super::*;
use crate::{catalog::playlist::PlaylistRef, ipc::ContextAction};
use serde_json::json;
#[tokio::test]
async fn playlist_play_starts_second_duplicate_by_source_position() {
    let (_dir, mut e) = super::tests::engine();
    let driver = Arc::new(crate::backend::testing::Controlled::default());
    e.factory = driver.clone();
    let mut bodies = vec![json!({"data":[{"id":"pl"}]})];
    for page in 0..2 {
        let data=(page*25..((page+1)*25).min(45)).map(|i|json!({"id":format!("i.{i}"),"type":"library-songs","attributes":{"name":format!("Song {i}"),"playParams":{"catalogId":if i==26{"26".to_string()}else{(i+1).to_string()}}}})).collect::<Vec<_>>();
        let mut body = json!({"data":data});
        if page == 0 {
            body["next"] = json!("ignored-url");
        }
        bodies.push(body);
    }
    let (_catalog_dir, catalog, http) = crate::catalog::playlist_tests::fixture(bodies).await;
    e.catalog = Arc::new(catalog);
    e.handle(Request::Playlist {
        key: "play-duplicate".into(),
        revision: e.queue.revision,
        playlist: PlaylistRef {
            id: "p.fixture".into(),
            storefront: "pl".into(),
            title: "Fixture".into(),
            can_edit: true,
        },
        action: ContextAction::PlayFromPosition {
            source_index: 26,
            shuffle: false,
        },
    })
    .await
    .unwrap();
    assert!(driver.starts().is_empty());
    loop {
        let event = e.album_events.recv().await.unwrap();
        let done = matches!(event, operations::AlbumEvent::Done(..));
        e.album_event(event).await;
        if done {
            break;
        }
    }
    let done = e.disk_events.recv().await.unwrap();
    e.disk_complete(done).await;
    assert_eq!(e.queue.tracks.len(), 45);
    assert_eq!(e.queue.current, Some(26));
    assert_eq!(e.queue.current_track().unwrap().id, "26");
    assert_eq!(driver.starts().len(), 1);
    http.await.unwrap();
}
use crate::{catalog::playlist_write::WriteAction, ipc::PlaylistWriteSource};
fn create(key: &str, source: PlaylistWriteSource) -> Request {
    Request::PlaylistWrite {
        key: key.into(),
        source,
        action: WriteAction::Create {
            name: "Synthetic".into(),
            description: String::new(),
        },
    }
}
async fn written(e: &mut Engine) -> playlist_write::PlaylistWriteStatus {
    let result = tokio::time::timeout(Duration::from_secs(3), e.playlist_write_events.recv())
        .await
        .unwrap()
        .unwrap();
    e.playlist_write_task.take().unwrap().await.unwrap();
    e.playlist_write_status = Some(result.clone());
    result
}
#[tokio::test]
async fn account_write_snapshot_preserves_duplicates_and_replays_after_queue_changes_and_restart() {
    let (_dir, mut e) = super::tests::engine();
    e.queue = Queue::default();
    for id in ["1", "1", "2"] {
        e.queue
            .push(Track {
                id: id.into(),
                kind: "songs".into(),
                ..Default::default()
            })
            .unwrap();
    }
    let (_cd, c, http) = crate::catalog::playlist_write_tests::server(vec![
        (200, Some(json!({"data":[{"id":"pl"}]}))),
        (
            201,
            Some(json!({"data":[{"id":"p.created","type":"library-playlists"}]})),
        ),
    ])
    .await;
    e.catalog = Arc::new(c);
    let queue = serde_json::to_value(&e.queue).unwrap();
    let request = create(
        "saved",
        PlaylistWriteSource::Queue {
            revision: e.queue.revision,
        },
    );
    e.handle(request.clone()).await.unwrap();
    // Snapshot captured synchronously; subsequent edits do not change exported IDs.
    e.queue.clear();
    let done = written(&mut e).await;
    assert_eq!(done.phase, "completed");
    assert_eq!(done.tracks, 3);
    let requests = http.await.unwrap();
    let tracks = &requests[1]["body"]["relationships"]["tracks"]["data"];
    assert_eq!(tracks.as_array().unwrap().len(), 3);
    assert_eq!(tracks[0]["id"], tracks[1]["id"]);
    assert_eq!(queue["tracks"].as_array().unwrap().len(), 3);
    let mut restored = Engine::new(Config::default(), e.paths.clone()).unwrap();
    restored.catalog = e.catalog.clone();
    restored.handle(request).await.unwrap();
    let done = written(&mut restored).await;
    assert_eq!(done.phase, "completed");
    assert!(done.message.contains("Already confirmed"));
    assert!(restored.queue.tracks.is_empty());
}
#[tokio::test]
async fn uncertain_write_cannot_retry_or_resend_same_key() {
    let (_dir, mut e) = super::tests::engine();
    let (_cd, c, http) = crate::catalog::playlist_write_tests::server(vec![
        (200, Some(json!({"data":[{"id":"pl"}]}))),
        (200, None),
    ])
    .await;
    e.catalog = Arc::new(c);
    let request = create("lost", PlaylistWriteSource::Empty);
    e.handle(request.clone()).await.unwrap();
    let done = written(&mut e).await;
    assert_eq!(done.phase, "uncertain");
    assert!(!done.can_retry);
    assert!(e.handle(Request::RetryPlaylistWrite).await.is_err());
    e.handle(request).await.unwrap();
    assert_eq!(written(&mut e).await.phase, "uncertain");
    assert_eq!(http.await.unwrap().len(), 2);
}
#[tokio::test]
async fn stale_and_oversized_exports_and_bad_journal_never_reach_http() {
    let (_dir, mut e) = super::tests::engine();
    let (_cd, c, http) = crate::catalog::playlist_write_tests::server(vec![]).await;
    e.catalog = Arc::new(c);
    e.handle(create(
        "stale",
        PlaylistWriteSource::Queue {
            revision: e.queue.revision + 1,
        },
    ))
    .await
    .unwrap();
    assert_eq!(written(&mut e).await.phase, "rejected");
    e.queue = Queue::default();
    for _ in 0..1001 {
        e.queue
            .push(Track {
                id: "1".into(),
                kind: "songs".into(),
                ..Default::default()
            })
            .unwrap();
    }
    e.handle(create(
        "large",
        PlaylistWriteSource::Queue {
            revision: e.queue.revision,
        },
    ))
    .await
    .unwrap();
    let result = written(&mut e).await;
    assert!(result.message.contains("1000"));
    private_write(&e.paths.data.join("playlist-writes.json"), b"broken").unwrap();
    e.handle(create("bad-disk", PlaylistWriteSource::Empty))
        .await
        .unwrap();
    assert_eq!(written(&mut e).await.phase, "rejected");
    assert_eq!(
        fs::read(e.paths.data.join("playlist-writes.json")).unwrap(),
        b"broken"
    );
    assert!(http.await.unwrap().is_empty());
}
#[tokio::test]
async fn held_writer_lock_blocks_another_actor_without_touching_account() {
    let (_dir, mut e) = super::tests::engine();
    let (_cd, c, http) = crate::catalog::playlist_write_tests::server(vec![]).await;
    e.catalog = Arc::new(c);
    let _lock = playlist_journal::acquire(&e.paths).unwrap();
    e.handle(create("locked", PlaylistWriteSource::Empty))
        .await
        .unwrap();
    let done = written(&mut e).await;
    assert_eq!(done.phase, "rejected");
    assert!(done.message.contains("writer"));
    assert!(http.await.unwrap().is_empty());
}
#[tokio::test]
async fn pending_account_response_does_not_block_pause_volume_or_status() {
    let (_dir, mut e) = super::tests::engine();
    e.queue = Queue::default();
    e.queue
        .push(Track {
            id: "1".into(),
            kind: "songs".into(),
            ..Default::default()
        })
        .unwrap();
    e.queue.current = Some(0);
    let driver = Arc::new(crate::backend::testing::Controlled::default());
    e.factory = driver.clone();
    e.handle(Request::Play).await.unwrap();
    let reached = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (_cd, c, http) = crate::catalog::playlist_write_tests::server_gated(
        vec![
            (200, Some(json!({"data":[{"id":"pl"}]}))),
            (
                201,
                Some(json!({"data":[{"id":"p.created","type":"library-playlists"}]})),
            ),
        ],
        Some((reached.clone(), release.clone())),
    )
    .await;
    e.catalog = Arc::new(c);
    let before = serde_json::to_value(&e.queue).unwrap();
    e.handle(create("slow", PlaylistWriteSource::Empty))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), reached.notified())
        .await
        .unwrap();
    for request in [Request::Pause, Request::Volume(0.25), Request::Status] {
        tokio::time::timeout(Duration::from_millis(300), e.handle(request))
            .await
            .unwrap()
            .unwrap();
    }
    assert_eq!(e.playback.volume, 0.25);
    assert!(e.desired_paused);
    assert_eq!(serde_json::to_value(&e.queue).unwrap(), before);
    assert_eq!(driver.starts().len(), 1);
    release.notify_one();
    assert_eq!(written(&mut e).await.phase, "completed");
    assert_eq!(http.await.unwrap().len(), 2);
    e.stop_audio().await;
}
