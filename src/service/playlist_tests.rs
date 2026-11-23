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
        account_generation: e.catalog.account_generation(),
        action: ContextAction::PlayFromPosition {
            source_index: 26,
            expected_song: Some("26".into()),
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
fn create(e: &Engine, key: &str, source: PlaylistWriteSource) -> Request {
    Request::PlaylistWrite {
        account_generation: e.catalog.account_generation(),
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
        &e,
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
    let request = create(&e, "lost", PlaylistWriteSource::Empty);
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
        &e,
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
        &e,
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
    e.handle(create(&e, "bad-disk", PlaylistWriteSource::Empty))
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
    e.handle(create(&e, "locked", PlaylistWriteSource::Empty))
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
    e.handle(create(&e, "slow", PlaylistWriteSource::Empty))
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

fn fixture_playlist_page(
    ids: impl IntoIterator<Item = usize>,
    total: usize,
    more: bool,
) -> serde_json::Value {
    let data = ids
        .into_iter()
        .map(|id| {
            json!({
                "id":format!("i.{id}"), "type":"library-songs",
                "attributes":{"name":format!("Song {id}"),"playParams":{"catalogId":id.to_string()}}
            })
        })
        .collect::<Vec<_>>();
    let mut page = json!({"data":data,"meta":{"total":total}});
    if more {
        page["next"] = json!("ignored-url");
    }
    page
}

fn play_at(e: &Engine, key: &str, index: usize, id: &str) -> Request {
    Request::Playlist {
        key: key.into(),
        revision: e.queue.revision,
        playlist: PlaylistRef {
            id: "p.fixture".into(),
            storefront: "pl".into(),
            title: "Fixture".into(),
            can_edit: true,
        },
        account_generation: e.catalog.account_generation(),
        action: ContextAction::PlayFromPosition {
            source_index: index,
            expected_song: Some(id.into()),
            shuffle: false,
        },
    }
}

async fn prepared_event(e: &mut Engine) -> operations::AlbumEvent {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let event = e.album_events.recv().await.unwrap();
            if matches!(event, operations::AlbumEvent::Done(..)) {
                return event;
            }
            e.album_event(event).await;
        }
    })
    .await
    .expect("playlist preparation completed")
}

async fn advance_budget_by(seconds: u64) {
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(seconds)).await;
    tokio::time::resume();
}

#[tokio::test]
async fn a_450_track_playlist_restarts_after_budget_wait_and_uses_the_new_order() {
    let (_dir, mut e) = super::tests::engine();
    let driver = Arc::new(crate::backend::testing::Controlled::default());
    e.factory = driver.clone();
    e.queue = Queue::default();
    e.queue
        .push(Track {
            id: "9".into(),
            kind: "songs".into(),
            ..Track::default()
        })
        .unwrap();
    e.queue.select(0);
    e.start_audio(0).await.unwrap();
    let page =
        |offset: usize| fixture_playlist_page(offset + 1..=offset + 25, 450, offset + 25 < 450);
    let mut bodies = vec![json!({"data":[{"id":"pl"}]}), page(0), page(25)];
    bodies.extend((0..425).step_by(25).map(page));
    bodies.extend((0..450).step_by(25).map(|offset| {
        let ids = (offset + 1..=offset + 25).map(|id| match id {
            38 => 37, // A true duplicate that must survive the restart.
            425 => 426,
            426 => 425,
            _ => id,
        });
        fixture_playlist_page(ids, 450, offset + 25 < 450)
    }));
    let (_catalog_dir, catalog, http) = crate::catalog::playlist_tests::fixture(bodies).await;
    e.catalog = Arc::new(catalog);
    let kind = "open:library-playlists:p.fixture";
    let limits = e.config.performance.limits();
    let first = e
        .catalog
        .browse(kind, "", None, "pl", limits)
        .await
        .unwrap();
    e.catalog
        .browse(kind, "", first.next.as_deref(), "pl", limits)
        .await
        .unwrap();
    advance_budget_by(31).await;

    e.handle(play_at(&e, "start-after-prefetch", 20, "21"))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        while e.operation.as_ref().unwrap().loaded < 425 {
            let event = e.album_events.recv().await.unwrap();
            assert!(matches!(event, operations::AlbumEvent::Progress(..)));
            e.album_event(event).await;
        }
    })
    .await
    .unwrap();
    assert!(
        !http.is_finished(),
        "21st HTTP must wait for the rolling limit"
    );
    tokio::task::yield_now().await;
    let started = std::time::Instant::now();
    e.handle(Request::Pause).await.unwrap();
    let status: Status = serde_json::from_value(e.handle(Request::Status).await.unwrap()).unwrap();
    assert_eq!(status.operation.unwrap().phase, "preparing");
    assert!(started.elapsed() < Duration::from_millis(100));
    assert_eq!(
        driver.starts().len(),
        1,
        "waiting leaves the current audio helper alone"
    );

    // Only the three initial calls have expired. The old prefix cannot resume here.
    advance_budget_by(30).await;
    let event = tokio::time::timeout(Duration::from_millis(50), e.album_events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event, operations::AlbumEvent::Progress(_, _, 0)));
    e.album_event(event).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), e.album_events.recv())
            .await
            .is_err()
    );
    assert_eq!(e.operation.as_ref().unwrap().loaded, 0);
    advance_budget_by(31).await;
    let event = prepared_event(&mut e).await;
    e.album_event(event).await;
    let done = tokio::time::timeout(Duration::from_secs(2), e.disk_events.recv())
        .await
        .unwrap()
        .unwrap();
    e.disk_complete(done).await;
    assert_eq!(e.queue.tracks.len(), 450);
    assert_eq!(e.queue.current, Some(20));
    let ids: Vec<_> = e
        .queue
        .tracks
        .iter()
        .map(|entry| entry.track.id.as_str())
        .collect();
    assert_eq!(&ids[423..427], &["424", "426", "425", "427"]);
    assert_eq!(&ids[36..39], &["37", "37", "39"]);
    let requests = tokio::time::timeout(Duration::from_secs(2), http)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        requests.len(),
        38,
        "two prefetched pages, 17 discarded pages, 18 fresh pages and a storefront"
    );
    for (page, request) in requests.iter().skip(20).enumerate() {
        let offset = page * 25;
        assert!(request.contains(&format!("offset={offset}&")), "{request}");
    }
}

#[tokio::test]
async fn reordered_page_boundary_reads_fresh_and_checks_the_original_occurrence() {
    let (_dir, mut e) = super::tests::engine();
    let old_first = fixture_playlist_page(1..=25, 50, true);
    let first = fixture_playlist_page((1..=24).chain([26]), 50, true);
    let second = fixture_playlist_page([25].into_iter().chain(27..=50), 50, false);
    let (_catalog_dir, catalog, http) = crate::catalog::playlist_tests::fixture(vec![
        json!({"data":[{"id":"pl"}]}),
        old_first,
        first.clone(),
        second.clone(),
        first,
        second,
    ])
    .await;
    e.catalog = Arc::new(catalog);
    let view = e
        .catalog
        .browse(
            "open:library-playlists:p.fixture",
            "",
            None,
            "pl",
            e.config.performance.limits(),
        )
        .await
        .unwrap();
    assert_eq!(view.items[24].id, "25");

    e.handle(play_at(&e, "moved-selection", 24, "25"))
        .await
        .unwrap();
    let event = prepared_event(&mut e).await;
    e.album_event(event).await;
    assert!(
        e.operation
            .as_ref()
            .unwrap()
            .message
            .contains("occurrence changed")
    );
    assert!(e.pending.is_none());
    assert_eq!(e.queue.tracks.len(), 3);

    e.handle(play_at(&e, "reopened-selection", 24, "26"))
        .await
        .unwrap();
    let event = prepared_event(&mut e).await;
    e.album_event(event).await;
    let done = tokio::time::timeout(Duration::from_secs(2), e.disk_events.recv())
        .await
        .unwrap()
        .unwrap();
    e.disk_complete(done).await;
    let ids: Vec<_> = e
        .queue
        .tracks
        .iter()
        .map(|entry| entry.track.id.as_str())
        .collect();
    assert_eq!(ids.len(), 50);
    assert_eq!(&ids[23..27], &["24", "26", "25", "27"]);
    assert_eq!(e.queue.current, Some(24));
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), http)
            .await
            .unwrap()
            .unwrap()
            .len(),
        6
    );
}

#[tokio::test]
async fn account_rotation_rejects_queue_work_before_start_and_after_download() {
    let (_dir, mut e) = super::tests::engine();
    let (account_dir, catalog, http) = crate::catalog::playlist_tests::fixture(vec![
        json!({"data":[{"id":"pl"}]}),
        fixture_playlist_page([1], 1, false),
    ])
    .await;
    let paths = crate::config::Paths {
        config: account_dir.path().join("c"),
        data: account_dir.path().join("d"),
        cache: account_dir.path().join("a"),
        runtime: account_dir.path().join("r"),
    };
    e.catalog = Arc::new(catalog);
    let stale = play_at(&e, "stale", 0, "1");
    let change = || {
        crate::auth::save_user_token(
            &paths,
            &crate::auth::SecretToken::parse("fixture-user").unwrap(),
        )
        .unwrap()
    };
    change();
    assert!(
        e.handle(stale)
            .await
            .unwrap_err()
            .to_string()
            .contains("credentials changed")
    );
    assert!(e.preparing.is_none());

    e.handle(play_at(&e, "prepared-then-changed", 0, "1"))
        .await
        .unwrap();
    let event = prepared_event(&mut e).await;
    assert!(matches!(&event, operations::AlbumEvent::Done(_, _, Ok(_))));
    change();
    e.album_event(event).await;
    assert!(
        e.operation
            .as_ref()
            .unwrap()
            .message
            .contains("credentials changed")
    );
    assert!(e.pending.is_none());
    assert_eq!(e.queue.tracks.len(), 3);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), http)
            .await
            .unwrap()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn account_rotation_cancels_a_playlist_waiting_for_local_budget() {
    let (_dir, mut e) = super::tests::engine();
    let mut responses = vec![json!({"data":[{"id":"pl"}]})];
    responses.extend((0..19).map(|_| json!({"data":[]})));
    let (directory, catalog, http) = crate::catalog::playlist_tests::fixture(responses).await;
    let paths = crate::config::Paths {
        config: directory.path().join("c"),
        data: directory.path().join("d"),
        cache: directory.path().join("a"),
        runtime: directory.path().join("r"),
    };
    e.catalog = Arc::new(catalog);
    for _ in 0..19 {
        e.catalog
            .browse("playlists", "", None, "pl", e.config.performance.limits())
            .await
            .unwrap();
    }
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), http)
            .await
            .unwrap()
            .unwrap()
            .len(),
        20
    );
    e.handle(play_at(&e, "wait-then-rotate", 0, "1"))
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(1), e.album_events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event, operations::AlbumEvent::Progress(_, _, 0)));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), e.album_events.recv())
            .await
            .is_err()
    );
    let token = crate::auth::SecretToken::parse("new-fixture-token").unwrap();
    crate::auth::save_user_token(&paths, &token).unwrap();
    advance_budget_by(1).await;
    let event = prepared_event(&mut e).await;
    e.album_event(event).await;
    assert!(
        e.operation
            .as_ref()
            .unwrap()
            .message
            .contains("credentials changed")
    );
    assert!(e.pending.is_none());
    assert_eq!(e.queue.tracks.len(), 3);
}

#[tokio::test]
async fn retry_playlist_write_uses_the_previous_operations_generation() {
    let (_dir, mut e) = super::tests::engine();
    let folder = tempfile::tempdir().unwrap();
    let paths = crate::catalog::playlist_tests::paths(folder.path());
    e.catalog = Arc::new(Catalog::new(&paths).unwrap());
    let generation = e.catalog.account_generation();
    e.playlist_write_status = Some(playlist_write::PlaylistWriteStatus {
        id: "retry-previous".into(),
        phase: "rejected".into(),
        can_retry: true,
        account_generation: generation,
        ..Default::default()
    });
    e.playlist_write_retry = Some(create(&e, "retry-previous", PlaylistWriteSource::Empty));
    crate::auth::save_user_token(
        &paths,
        &crate::auth::SecretToken::parse("new-fixture-token").unwrap(),
    )
    .unwrap();
    let error = e.handle(Request::RetryPlaylistWrite).await.unwrap_err();
    assert!(error.to_string().contains("credentials changed"));
    assert!(e.playlist_write_task.is_none());
}

#[tokio::test]
async fn a_confirmed_write_cannot_start_after_the_account_changes() {
    let (_dir, mut e) = super::tests::engine();
    let folder = tempfile::tempdir().unwrap();
    let paths = crate::catalog::playlist_tests::paths(folder.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    e.catalog = Arc::new(Catalog::with_music_origin(
        paths.clone(),
        format!("http://{}", listener.local_addr().unwrap()),
    ));
    let confirmed = create(
        &e,
        "confirmed-under-old-account",
        PlaylistWriteSource::Empty,
    );
    crate::auth::save_user_token(
        &paths,
        &crate::auth::SecretToken::parse("new-fixture-token").unwrap(),
    )
    .unwrap();
    let error = e.handle(confirmed).await.unwrap_err();
    assert!(error.to_string().contains("credentials changed"));
    assert!(e.playlist_write_task.is_none());
    assert!(e.playlist_write_status.is_none());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
}
