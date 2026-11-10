use super::*;
use crate::{
    ipc::{QueueAction, queue_action},
    model::PlaybackPhase,
    queue::QueueContext,
};
async fn commit(e: &mut Engine, action: QueueAction) -> Result<()> {
    e.handle(queue_action(e.queue.revision, action)).await?;
    if let Some(done) = e.disk_events.recv().await {
        e.disk_complete(done).await;
    }
    Ok(())
}
#[tokio::test]
async fn failed_disk_commit_leaves_session_untouched_and_retry_is_idempotent() {
    let (_dir, mut e) = super::tests::engine();
    let old = e.queue.revision;
    let request = Request::QueueAction {
        key: "same-key".into(),
        revision: old,
        action: QueueAction::Append {
            tracks: vec![Track {
                id: "42".into(),
                kind: "songs".into(),
                ..Track::default()
            }],
            context: QueueContext::single(),
        },
    };
    std::fs::create_dir(e.paths.data.join("state.json")).unwrap();
    e.handle(request.clone()).await.unwrap();
    let done = e.disk_events.recv().await.unwrap();
    e.disk_complete(done).await;
    assert_eq!(e.queue.revision, old);
    assert_eq!(e.queue.tracks.len(), 3);
    std::fs::remove_dir(e.paths.data.join("state.json")).unwrap();
    e.handle(request.clone()).await.unwrap();
    let done = e.disk_events.recv().await.unwrap();
    e.disk_complete(done).await;
    assert_eq!(e.queue.tracks.len(), 4);
    e.handle(request).await.unwrap();
    assert_eq!(e.queue.tracks.len(), 4);
    assert!(e.pending.is_none());
}
#[tokio::test]
async fn queue_commit_and_audio_failure_are_distinct_and_undo_restores_without_autoplay() {
    let (_dir, mut e) = super::tests::engine();
    commit(
        &mut e,
        QueueAction::PlayContext {
            tracks: vec![Track {
                id: "42".into(),
                kind: "songs".into(),
                ..Track::default()
            }],
            context: QueueContext::single(),
            start: 0,
            shuffle: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(e.queue.tracks.len(), 1);
    assert_eq!(e.playback.phase, PlaybackPhase::Failed);
    assert!(!e.playback.playing);
    commit(&mut e, QueueAction::Undo).await.unwrap();
    assert_eq!(e.queue.tracks.len(), 3);
    assert_eq!(e.playback.phase, PlaybackPhase::Stopped);
}
#[tokio::test]
async fn append_and_clear_do_not_change_pause_position_or_repeat() {
    let (_dir, mut e) = super::tests::engine();
    e.playback.position = 17.0;
    e.playback.phase = PlaybackPhase::Paused;
    e.queue.repeat = Repeat::All;
    commit(
        &mut e,
        QueueAction::Append {
            tracks: vec![Track {
                id: "42".into(),
                kind: "songs".into(),
                ..Track::default()
            }],
            context: QueueContext::single(),
        },
    )
    .await
    .unwrap();
    commit(&mut e, QueueAction::ClearUpcoming).await.unwrap();
    assert_eq!(e.queue.tracks.len(), 1);
    assert_eq!(e.playback.position, 17.0);
    assert_eq!(e.playback.phase, PlaybackPhase::Paused);
    assert_eq!(e.queue.repeat, Repeat::All);
    assert_eq!(e.queue.candidate(false), None);
}

#[tokio::test]
async fn controlled_worker_preserves_pause_thresholds_reports_failure_and_rejects_old_events() {
    use crate::backend::testing::Controlled;
    let (_dir, mut e) = super::tests::engine();
    let driver = std::sync::Arc::new(Controlled::default());
    e.factory = driver.clone();
    e.queue.clear();
    for id in ["1", "2", "3"] {
        e.queue
            .push(Track {
                id: id.into(),
                kind: "songs".into(),
                ..Track::default()
            })
            .unwrap();
    }
    e.queue.current = Some(0);
    e.handle(Request::Play).await.unwrap();
    let generation = e.audio_generation;
    driver
        .emit(Event::Ready {
            duration: 60.0,
            sample_rate: 44100,
            channels: 2,
        })
        .await;
    let (g, event) = e.events.recv().await.unwrap();
    e.native_event(g, event).await;
    driver
        .emit(Event::State {
            playing: true,
            position: 5.0,
            encrypted_packets: 1,
            decoded_frames: 1,
            nonzero_samples: 1,
        })
        .await;
    let (g, event) = e.events.recv().await.unwrap();
    e.native_event(g, event).await;
    assert_eq!(e.playback.phase, PlaybackPhase::Playing);
    e.handle(Request::Pause).await.unwrap();
    e.handle(Request::Next).await.unwrap();
    let done = e.disk_events.recv().await.unwrap();
    e.disk_complete(done).await;
    assert_eq!(e.queue.current_track().unwrap().id, "2");
    assert!(driver.starts().last().unwrap().1);
    e.playback.position = 4.0;
    e.handle(Request::Previous).await.unwrap();
    assert_eq!(e.queue.current_track().unwrap().id, "2");
    assert!(driver.commands().iter().any(|v| v.contains("Seek(0.0)")));
    e.playback.position = 3.0;
    e.handle(Request::Previous).await.unwrap();
    let done = e.disk_events.recv().await.unwrap();
    e.disk_complete(done).await;
    assert_eq!(e.queue.current_track().unwrap().id, "1");
    assert!(driver.starts().last().unwrap().1);
    e.native_event(generation, Event::Error("old error".into()))
        .await;
    assert!(e.audio_error.is_none());
    e.native_event(
        e.audio_generation,
        Event::Error("Synthetic audio failure".into()),
    )
    .await;
    assert_eq!(e.playback.phase, PlaybackPhase::Failed);
    assert_eq!(e.audio_error.as_deref(), Some("Synthetic audio failure"));
    e.handle(Request::Play).await.unwrap();
    assert!(e.audio_error.is_none());
}

async fn delayed_album(
    e: &mut Engine,
) -> (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    e.catalog = std::sync::Arc::new(Catalog::with_origin(&origin));
    let (release, go) = tokio::sync::oneshot::channel();
    let (entered, started) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 8192];
        assert!(socket.read(&mut bytes).await.unwrap() > 0);
        let _ = entered.send(());
        let _ = go.await;
        let body=serde_json::json!({"results":[{"wrapperType":"collection","collectionType":"Album","collectionId":42,"collectionName":"Album","trackCount":1},{"kind":"song","trackId":99,"trackName":"Song"}]}).to_string();
        let _ = socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await;
    });
    (release, started, task)
}
fn album_request(e: &Engine, key: &str) -> Request {
    Request::Album {
        key: key.into(),
        revision: e.queue.revision,
        album: crate::catalog::album::AlbumRef {
            id: "42".into(),
            provider: "itunes".into(),
            storefront: "pl".into(),
            title: "Album".into(),
        },
        action: crate::ipc::AlbumAction::Append,
    }
}
#[tokio::test]
async fn slow_album_allows_pause_volume_and_cancellation_before_commit() {
    let (_dir, mut e) = super::tests::engine();
    e.queue.clear();
    e.queue
        .push(Track {
            id: "1".into(),
            kind: "songs".into(),
            ..Track::default()
        })
        .unwrap();
    e.queue.current = Some(0);
    let driver = std::sync::Arc::new(crate::backend::testing::Controlled::default());
    e.factory = driver.clone();
    e.handle(Request::Play).await.unwrap();
    let (release, started, http) = delayed_album(&mut e).await;
    let revision = e.queue.revision;
    e.handle(album_request(&e, "cancel-me")).await.unwrap();
    started.await.unwrap();
    tokio::time::timeout(
        std::time::Duration::from_millis(100),
        e.handle(Request::Pause),
    )
    .await
    .unwrap()
    .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_millis(100),
        e.handle(Request::Volume(0.2)),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(driver.commands().iter().any(|c| c == "Pause(true)"));
    assert_eq!(e.playback.volume, 0.2);
    e.handle(Request::CancelOperation("cancel-me".into()))
        .await
        .unwrap();
    assert_eq!(e.queue.revision, revision);
    assert_eq!(e.queue.tracks.len(), 1);
    assert!(e.pending.is_none());
    release.send(()).unwrap();
    http.await.unwrap();
    assert_eq!(e.operation.as_ref().unwrap().phase, "cancelled");
}
#[tokio::test]
async fn late_album_does_not_overwrite_another_clients_edit_and_retry_survives_restart() {
    let (_dir, mut e) = super::tests::engine();
    let (release, started, http) = delayed_album(&mut e).await;
    e.handle(album_request(&e, "late-album")).await.unwrap();
    started.await.unwrap();
    commit(
        &mut e,
        QueueAction::Append {
            tracks: vec![Track {
                id: "4".into(),
                kind: "songs".into(),
                ..Track::default()
            }],
            context: QueueContext::single(),
        },
    )
    .await
    .unwrap();
    release.send(()).unwrap();
    http.await.unwrap();
    loop {
        let event = e.album_events.recv().await.unwrap();
        let done = matches!(event, operations::AlbumEvent::Done(..));
        e.album_event(event).await;
        if done {
            break;
        }
    }
    assert_eq!(e.queue.tracks.len(), 4);
    assert!(e.pending.is_none());
    assert_eq!(e.operation.as_ref().unwrap().phase, "failed");
    let request = Request::QueueAction {
        key: "durable-key".into(),
        revision: e.queue.revision,
        action: QueueAction::Append {
            tracks: vec![Track {
                id: "5".into(),
                kind: "songs".into(),
                ..Track::default()
            }],
            context: QueueContext::single(),
        },
    };
    e.handle(request.clone()).await.unwrap();
    let done = e.disk_events.recv().await.unwrap();
    e.disk_complete(done).await;
    let mut restored = Engine::new(e.config.clone(), e.paths.clone()).unwrap();
    restored.handle(request).await.unwrap();
    assert!(restored.pending.is_none());
    assert_eq!(restored.queue.tracks.len(), 5);
    assert_eq!(restored.playback.phase, PlaybackPhase::Stopped);
}
#[tokio::test]
async fn remove_current_hands_pause_to_successor_and_last_removal_stops() {
    let (_dir, mut e) = super::tests::engine();
    e.queue.clear();
    for id in ["1", "2"] {
        e.queue
            .push(Track {
                id: id.into(),
                kind: "songs".into(),
                ..Track::default()
            })
            .unwrap();
    }
    e.queue.current = Some(0);
    let driver = std::sync::Arc::new(crate::backend::testing::Controlled::default());
    e.factory = driver.clone();
    e.handle(Request::Play).await.unwrap();
    e.handle(Request::Pause).await.unwrap();
    let current = e.queue.current_entry_id().unwrap();
    commit(&mut e, QueueAction::RemoveAndAdvance { entry_id: current })
        .await
        .unwrap();
    assert_eq!(e.queue.current_track().unwrap().id, "2");
    assert!(driver.starts().last().unwrap().1);
    let current = e.queue.current_entry_id().unwrap();
    commit(&mut e, QueueAction::RemoveAndAdvance { entry_id: current })
        .await
        .unwrap();
    assert!(e.audio.is_none());
    assert_eq!(e.queue.current, None);
    assert_eq!(e.playback.phase, PlaybackPhase::Stopped);
}

#[tokio::test]
async fn eof_during_replacement_does_not_skip_new_first_track_and_stop_cancels_pending_audio() {
    let (_dir, mut e) = super::tests::engine();
    let driver = std::sync::Arc::new(crate::backend::testing::Controlled::default());
    e.factory = driver.clone();
    e.queue.clear();
    e.queue
        .push(Track {
            id: "1".into(),
            kind: "songs".into(),
            ..Track::default()
        })
        .unwrap();
    e.queue.current = Some(0);
    e.handle(Request::Play).await.unwrap();
    let replacement = || QueueAction::PlayContext {
        tracks: vec![
            Track {
                id: "2".into(),
                kind: "songs".into(),
                ..Track::default()
            },
            Track {
                id: "3".into(),
                kind: "songs".into(),
                ..Track::default()
            },
        ],
        context: QueueContext::single(),
        start: 0,
        shuffle: false,
    };
    e.handle(crate::ipc::queue_action(e.queue.revision, replacement()))
        .await
        .unwrap();
    e.native_event(
        e.audio_generation,
        Event::Ended {
            position: 60.0,
            encrypted_packets: 1,
            decoded_frames: 1,
            nonzero_samples: 1,
        },
    )
    .await;
    let done = e.disk_events.recv().await.unwrap();
    e.disk_complete(done).await;
    assert!(e.pending.is_none());
    assert_eq!(e.queue.current_track().unwrap().id, "2");
    e.handle(crate::ipc::queue_action(e.queue.revision, replacement()))
        .await
        .unwrap();
    e.handle(Request::StopAudio).await.unwrap();
    let done = e.disk_events.recv().await.unwrap();
    e.disk_complete(done).await;
    assert!(e.audio.is_none());
    assert_eq!(e.playback.phase, PlaybackPhase::Stopped);
}

#[tokio::test]
async fn blocked_state_writer_does_not_block_pause_volume_or_status() {
    use nix::{sys::stat::Mode, unistd::mkfifo};
    let (_dir, mut e) = super::tests::engine();
    e.queue.clear();
    e.queue
        .push(Track {
            id: "1".into(),
            kind: "songs".into(),
            ..Track::default()
        })
        .unwrap();
    e.queue.current = Some(0);
    e.factory = std::sync::Arc::new(crate::backend::testing::Controlled::default());
    e.handle(Request::Play).await.unwrap();
    let file = e.paths.data.join("state.json");
    mkfifo(&file, Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
    e.handle(crate::ipc::queue_action(
        e.queue.revision,
        QueueAction::Append {
            tracks: vec![Track {
                id: "2".into(),
                kind: "songs".into(),
                ..Track::default()
            }],
            context: QueueContext::single(),
        },
    ))
    .await
    .unwrap();
    for request in [Request::Pause, Request::Volume(0.3), Request::Status] {
        tokio::time::timeout(std::time::Duration::from_millis(100), e.handle(request))
            .await
            .unwrap()
            .unwrap();
    }
    let writer = tokio::task::spawn_blocking(move || {
        std::fs::write(file, b"invalid state").unwrap();
    });
    let done = e.disk_events.recv().await.unwrap();
    e.disk_complete(done).await;
    writer.await.unwrap();
    assert_eq!(e.queue.tracks.len(), 1);
    assert_eq!(e.playback.volume, 0.3);
    assert_eq!(e.operation.as_ref().unwrap().phase, "failed");
}

#[tokio::test]
async fn stale_duplicate_edit_cannot_target_a_different_occurrence() {
    let (_dir, mut e) = super::tests::engine();
    e.queue.clear();
    for _ in 0..2 {
        e.queue
            .push(Track {
                id: "1".into(),
                kind: "songs".into(),
                ..Track::default()
            })
            .unwrap();
    }
    let revision = e.queue.revision;
    let first = e.queue.tracks[0].entry_id;
    let second = e.queue.tracks[1].entry_id;
    commit(
        &mut e,
        QueueAction::MoveBy {
            entry_id: first,
            delta: 1,
        },
    )
    .await
    .unwrap();
    let stale = Request::QueueAction {
        key: "stale-delete".into(),
        revision,
        action: QueueAction::RemoveAndAdvance { entry_id: first },
    };
    assert!(e.handle(stale).await.is_err());
    assert_eq!(e.queue.tracks[0].entry_id, second);
    assert!(
        e.handle(Request::Remove(0)).await.is_err(),
        "Unchecked positional IPC must not edit a stale row"
    );
}

#[tokio::test]
async fn removal_after_stop_selects_successor_without_autoplay_and_stop_clears_old_error() {
    let (_dir, mut e) = super::tests::engine();
    e.queue.clear();
    for id in ["1", "2", "3"] {
        e.queue
            .push(Track {
                id: id.into(),
                kind: "songs".into(),
                ..Track::default()
            })
            .unwrap();
    }
    e.queue.current = Some(0);
    let driver = std::sync::Arc::new(crate::backend::testing::Controlled::default());
    e.factory = driver.clone();
    e.handle(Request::Play).await.unwrap();
    e.handle(Request::StopAudio).await.unwrap();
    let started = driver.starts().len();
    let id = e.queue.current_entry_id().unwrap();
    commit(&mut e, QueueAction::RemoveAndAdvance { entry_id: id })
        .await
        .unwrap();
    assert_eq!(e.queue.current_track().unwrap().id, "2");
    assert_eq!(driver.starts().len(), started);
    assert_eq!(e.playback.phase, PlaybackPhase::Stopped);
    e.fail_audio("Synthetic failure".into()).await;
    e.handle(Request::StopAudio).await.unwrap();
    assert!(e.audio_error.is_none());
}

#[tokio::test]
async fn renamed_but_unsynced_state_is_adopted_without_audio_and_locks_edits() {
    let (_dir, mut e) = super::tests::engine();
    let request = crate::ipc::queue_action(
        e.queue.revision,
        QueueAction::Append {
            tracks: vec![Track {
                id: "4".into(),
                kind: "songs".into(),
                ..Track::default()
            }],
            context: QueueContext::single(),
        },
    );
    e.handle(request.clone()).await.unwrap();
    let mut done = e.disk_events.recv().await.unwrap();
    done.result = Err(persistence::DurabilityUncertain.into());
    e.disk_complete(done).await;
    assert_eq!(e.queue.tracks.len(), 4);
    assert!(e.recovery_required);
    assert!(e.audio.is_none());
    assert_eq!(e.playback.phase, PlaybackPhase::Stopped);
    e.handle(request).await.unwrap();
    assert!(e.pending.is_none());
    assert!(
        e.handle(crate::ipc::queue_action(
            e.queue.revision,
            QueueAction::ClearUpcoming
        ))
        .await
        .is_err()
    );
    assert_eq!(e.queue.tracks.len(), 4);
    assert!(
        e.handle(Request::RecoverState).await.is_err(),
        "A valid installed state must be reconciled on restart, not renamed as corrupt"
    );
    let restored = Engine::new(e.config.clone(), e.paths.clone()).unwrap();
    assert_eq!(restored.queue.tracks.len(), 4);
    assert!(!restored.recovery_required);
}

#[tokio::test]
async fn failed_batch_can_retry_with_same_key_without_duplicate_commit() {
    let (_dir, mut e) = super::tests::engine();
    std::fs::create_dir(e.paths.data.join("state.json")).unwrap();
    e.handle(crate::ipc::queue_action(
        e.queue.revision,
        QueueAction::Append {
            tracks: vec![Track {
                id: "4".into(),
                kind: "songs".into(),
                ..Track::default()
            }],
            context: QueueContext::single(),
        },
    ))
    .await
    .unwrap();
    let done = e.disk_events.recv().await.unwrap();
    e.disk_complete(done).await;
    std::fs::remove_dir(e.paths.data.join("state.json")).unwrap();
    e.handle(Request::RetryOperation).await.unwrap();
    let done = e.disk_events.recv().await.unwrap();
    e.disk_complete(done).await;
    assert_eq!(e.queue.tracks.len(), 4);
    e.handle(Request::RetryOperation).await.unwrap();
    assert!(e.pending.is_none());
    assert_eq!(e.queue.tracks.len(), 4);
}

#[tokio::test]
async fn cancelled_album_completion_cannot_commit_into_retry_with_same_key() {
    let (_dir, mut e) = super::tests::engine();
    let (release, started, http) = delayed_album(&mut e).await;
    let request = album_request(&e, "cancel-retry");
    e.handle(request.clone()).await.unwrap();
    started.await.unwrap();
    let old = e.album_generation;
    e.handle(Request::CancelOperation("cancel-retry".into()))
        .await
        .unwrap();
    e.handle(request).await.unwrap();
    e.album_event(operations::AlbumEvent::Done(
        old,
        "cancel-retry".into(),
        Ok(crate::catalog::album::PreparedAlbum {
            tracks: vec![Track {
                id: "99".into(),
                kind: "songs".into(),
                ..Track::default()
            }],
            skipped: Vec::new(),
            context: QueueContext::single(),
        }),
    ))
    .await;
    assert_eq!(e.queue.tracks.len(), 3);
    assert!(e.pending.is_none());
    assert!(e.preparing.is_some());
    e.handle(Request::CancelOperation("cancel-retry".into()))
        .await
        .unwrap();
    release.send(()).unwrap();
    http.await.unwrap();
}

#[tokio::test]
async fn disabling_queue_restore_keeps_identity_namespace_and_invalid_state_recovery() {
    let (_dir, mut e) = super::tests::engine();
    commit(
        &mut e,
        QueueAction::Append {
            tracks: vec![Track {
                id: "4".into(),
                kind: "songs".into(),
                ..Track::default()
            }],
            context: QueueContext::single(),
        },
    )
    .await
    .unwrap();
    let last = e.queue.next_entry_id;
    let revision = e.queue.revision;
    let mut config = e.config.clone();
    config.restore_queue = false;
    let mut empty = Engine::new(config.clone(), e.paths.clone()).unwrap();
    assert!(empty.queue.tracks.is_empty());
    assert!(empty.queue.revision > revision);
    empty
        .queue
        .push(Track {
            id: "5".into(),
            kind: "songs".into(),
            ..Track::default()
        })
        .unwrap();
    assert!(empty.queue.tracks[0].entry_id > last);
    crate::config::private_write(&e.paths.data.join("state.json"), b"invalid original").unwrap();
    let mut invalid = Engine::new(config, e.paths.clone()).unwrap();
    assert!(invalid.recovery_required);
    invalid.handle(Request::RecoverState).await.unwrap();
    assert!(!invalid.recovery_required);
}

#[tokio::test]
async fn complete_album_exceeding_session_limit_preserves_old_queue_and_audio() {
    let (_dir, mut e) = super::tests::engine();
    for (i, entry) in e.queue.tracks.iter_mut().enumerate() {
        entry.track.id = (i + 1).to_string();
    }
    e.config.max_queue = 3;
    let driver = std::sync::Arc::new(crate::backend::testing::Controlled::default());
    e.factory = driver.clone();
    e.handle(Request::Play).await.unwrap();
    e.handle(Request::Pause).await.unwrap();
    let before = e.queue.revision;
    let generation = e.audio_generation;
    let (release, started, http) = delayed_album(&mut e).await;
    e.handle(album_request(&e, "over-limit")).await.unwrap();
    started.await.unwrap();
    release.send(()).unwrap();
    http.await.unwrap();
    loop {
        let event = e.album_events.recv().await.unwrap();
        let done = matches!(event, operations::AlbumEvent::Done(..));
        e.album_event(event).await;
        if done {
            break;
        }
    }
    assert!(e.pending.is_none());
    assert_eq!(e.queue.tracks.len(), 3);
    assert_eq!(e.queue.revision, before);
    assert_eq!(e.audio_generation, generation);
    assert!(e.audio.is_some());
    assert!(e.desired_paused);
    assert_eq!(driver.starts().len(), 1);
    assert_eq!(e.operation.as_ref().unwrap().phase, "failed");
}
