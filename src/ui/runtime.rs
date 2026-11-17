//! Bounded independent metadata, queue mutation and immediate transport lanes.
use super::*;
fn mutation(request: &Request) -> bool {
    matches!(
        request,
        Request::QueueAction { .. }
            | Request::Album { .. }
            | Request::Playlist { .. }
            | Request::PlaylistWrite { .. }
            | Request::RetryPlaylistWrite
            | Request::Enqueue { .. }
            | Request::PlayQueued { .. }
            | Request::QueueEdit { .. }
            | Request::PlayIndex(_)
            | Request::Remove(_)
            | Request::Move { .. }
            | Request::Clear
            | Request::Shuffle
            | Request::Repeat
            | Request::Next
            | Request::Previous
            | Request::RecordSearch(_)
    )
}
pub(super) async fn requests(
    paths: Paths,
    mut work: mpsc::Receiver<Request>,
    done: mpsc::Sender<(Request, Result<serde_json::Value>)>,
) {
    let (meta, mut metadata) = tokio::sync::watch::channel::<Option<Request>>(None);
    let (edits, mut edit_rx) = mpsc::channel::<Request>(16);
    let (transport, mut transport_rx) = mpsc::channel::<Request>(16);
    let mut tasks = tokio::task::JoinSet::new();
    let metadata_paths = paths.clone();
    let metadata_done = done.clone();
    tasks.spawn(async move {
        while metadata.changed().await.is_ok() {
            let Some(request) = metadata.borrow_and_update().clone() else {
                continue;
            };
            let result = tokio::select! {
                biased;
                changed=metadata.changed()=>{if changed.is_err(){break;}continue;}
                result=ipc::request(&metadata_paths,&request)=>result,
            };
            if metadata.has_changed().unwrap_or(true) {
                continue;
            }
            if metadata_done.send((request, result)).await.is_err() {
                break;
            }
        }
    });
    let edit_paths = paths.clone();
    let edit_done = done.clone();
    tasks.spawn(async move {
        while let Some(request) = edit_rx.recv().await {
            let result = ipc::request(&edit_paths, &request).await;
            if edit_done.send((request, result)).await.is_err() {
                break;
            }
        }
    });
    let control_done = done.clone();
    tasks.spawn(async move {
        while let Some(request) = transport_rx.recv().await {
            let result = ipc::request(&paths, &request).await;
            if control_done.send((request, result)).await.is_err() {
                break;
            }
        }
    });
    while let Some(request) = work.recv().await {
        if matches!(request, Request::Browse { .. }) {
            meta.send_replace(Some(request));
            continue;
        }
        let result = if mutation(&request) {
            edits.try_send(request)
        } else {
            transport.try_send(request)
        };
        if let Err(error) = result {
            let _ = done
                .send((
                    error.into_inner(),
                    Err(anyhow::anyhow!(
                        "Command lane is full; retry after pending commands"
                    )),
                ))
                .await;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn slow_metadata_does_not_delay_transport() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            config: dir.path().join("c"),
            data: dir.path().join("d"),
            cache: dir.path().join("a"),
            runtime: dir.path().join("r"),
        };
        paths.init().unwrap();
        let listener = tokio::net::UnixListener::bind(paths.socket()).unwrap();
        let (jobs, work) = mpsc::channel(8);
        let (done, _responses) = mpsc::channel(8);
        let worker = tokio::spawn(requests(paths, work, done));
        jobs.send(Request::Browse {
            kind: "search:albums".into(),
            term: "Album".into(),
            next: None,
        })
        .await
        .unwrap();
        let (mut slow, _) = listener.accept().await.unwrap();
        ipc::read_frame(&mut slow).await.unwrap();
        jobs.send(Request::Toggle).await.unwrap();
        let (mut fast, _) =
            tokio::time::timeout(std::time::Duration::from_millis(300), listener.accept())
                .await
                .expect("Toggle must reach service while catalog is pending")
                .unwrap();
        let request: Request =
            serde_json::from_slice(&ipc::read_frame(&mut fast).await.unwrap()).unwrap();
        assert!(matches!(request, Request::Toggle));
        worker.abort();
    }
}

pub(super) async fn subscribe(paths: Paths, updates: mpsc::Sender<Result<Status>>) {
    let mut delay = std::time::Duration::from_millis(500);
    loop {
        let result = async {
            let mut stream = tokio::net::UnixStream::connect(paths.socket()).await?;
            ipc::write_frame(&mut stream, &Request::Subscribe).await?;
            loop {
                let b = ipc::read_frame(&mut stream).await?;
                let value = serde_json::from_slice::<Reply>(&b)?.value()?;
                let status = serde_json::from_value(value)?;
                if updates.send(Ok(status)).await.is_err() {
                    return Ok::<(), anyhow::Error>(());
                }
                delay = std::time::Duration::from_millis(500);
            }
        }
        .await;
        if result.is_ok() || updates.is_closed() {
            break;
        }
        if updates
            .send(Err(anyhow::anyhow!("Service disconnected; reconnecting…")))
            .await
            .is_err()
        {
            break;
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(std::time::Duration::from_secs(8));
    }
}

pub(super) fn redraw_needed(old: Option<&Status>, new: &Status, equalizer: bool) -> bool {
    let Some(old) = old else {
        return true;
    };
    let playback = |s: &Status| {
        (
            s.playback.phase,
            s.playback.playing,
            (s.playback.position * 5.0) as u64,
            s.playback.duration,
            s.playback.volume,
            s.playback.authorized,
        )
    };
    playback(old) != playback(new)
        || old.playback.track != new.playback.track
        || old.playback.source_codec != new.playback.source_codec
        || old.playback.source_sample_rate_hz != new.playback.source_sample_rate_hz
        || old.playback.source_channels != new.playback.source_channels
        || old.playback.target_bitrate_kbps != new.playback.target_bitrate_kbps
        || old.queue_revision != new.queue_revision
        || old.queue_len != new.queue_len
        || old.current_entry_id != new.current_entry_id
        || old.next_entry_id != new.next_entry_id
        || old.can_next != new.can_next
        || old.can_previous != new.can_previous
        || old.start_available != new.start_available
        || old.playback_available != new.playback_available
        || old.profile != new.profile
        || old.repeat != new.repeat
        || old.shuffle != new.shuffle
        || old.pid != new.pid
        || old.audio_error != new.audio_error
        || old.config_error != new.config_error
        || old.message != new.message
        || old.sleep_remaining != new.sleep_remaining
        || old.appearance != new.appearance
        || old.equalizer != new.equalizer
        || old.equalizer_applied != new.equalizer_applied
        || old.operation != new.operation
        || old.playlist_write != new.playlist_write
        || old.catalog_country != new.catalog_country
        || (equalizer
            && (old.equalizer_input_rms != new.equalizer_input_rms
                || old.equalizer_output_rms != new.equalizer_output_rms
                || old.equalizer_headroom_db != new.equalizer_headroom_db))
}

#[cfg(test)]
mod reconnect_tests {
    use super::*;
    #[tokio::test]
    async fn subscription_reconnects_without_replaying_commands() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            config: dir.path().join("c"),
            data: dir.path().join("d"),
            cache: dir.path().join("a"),
            runtime: dir.path().join("r"),
        };
        paths.init().unwrap();
        let listener = tokio::net::UnixListener::bind(paths.socket()).unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let task = tokio::spawn(subscribe(paths, tx));
        for _ in 0..2 {
            let (mut stream, _) =
                tokio::time::timeout(std::time::Duration::from_secs(2), listener.accept())
                    .await
                    .expect("must reconnect")
                    .unwrap();
            let request: Request =
                serde_json::from_slice(&ipc::read_frame(&mut stream).await.unwrap()).unwrap();
            assert!(matches!(request, Request::Subscribe));
            drop(stream);
            assert!(rx.recv().await.unwrap().is_err());
        }
        task.abort();
    }
}
