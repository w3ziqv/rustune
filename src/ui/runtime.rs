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
