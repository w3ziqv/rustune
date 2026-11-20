//! Bound each connection phase independently so long-lived work cannot occupy control slots.
use super::{ClientRegistration, Reply, Sender, Status, send};
use crate::{
    catalog::Catalog,
    ipc::{Request, read_limited_frame, write_frame},
};
use anyhow::Result;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    net::{UnixListener, UnixStream},
    sync::{OwnedSemaphorePermit, Semaphore, oneshot, watch},
    task::JoinSet,
    time::timeout,
};

const REQUEST_LIMIT: usize = 64 * 1024;
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct Clients {
    tx: Sender,
    updates: watch::Receiver<Status>,
    catalog: Arc<Catalog>,
    registry: Arc<Mutex<BTreeMap<u32, usize>>>,
    browse: Arc<Semaphore>,
    subscribers: Arc<Semaphore>,
    controls: Arc<Semaphore>,
}
pub(super) async fn serve(
    listener: UnixListener,
    uid: u32,
    tx: Sender,
    updates: watch::Receiver<Status>,
    catalog: Arc<Catalog>,
    registry: Arc<Mutex<BTreeMap<u32, usize>>>,
    mut quit: oneshot::Receiver<()>,
) {
    let readers = Arc::new(Semaphore::new(16));
    let clients = Clients {
        tx,
        updates,
        catalog,
        registry,
        browse: Arc::new(Semaphore::new(4)),
        subscribers: Arc::new(Semaphore::new(8)),
        controls: Arc::new(Semaphore::new(16)),
    };
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            _=&mut quit=>break,
            accepted=async {
                let slot=readers.clone().acquire_owned().await.expect("open reader pool");
                listener.accept().await.map(|(socket,_)|(socket,slot))
            }=>{
                let Ok((socket,slot))=accepted else {break};
                let Ok(peer)=socket.peer_cred() else {continue};
                if peer.uid()!=uid {continue}
                let clients=clients.clone();
                tasks.spawn(clients.handle(socket,peer.pid().and_then(|p|u32::try_from(p).ok()),slot));
            }
            _=tasks.join_next(),if !tasks.is_empty()=>{}
        }
    }
    let _ = timeout(Duration::from_secs(1), async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
}
async fn respond(socket: &mut UnixStream, reply: Reply) -> Result<()> {
    timeout(WRITE_TIMEOUT, write_frame(socket, &reply)).await??;
    Ok(())
}
impl Clients {
    async fn handle(
        mut self,
        mut socket: UnixStream,
        pid: Option<u32>,
        reader: OwnedSemaphorePermit,
    ) -> Result<()> {
        let bytes = timeout(
            Duration::from_secs(5),
            read_limited_frame(&mut socket, REQUEST_LIMIT),
        )
        .await??;
        let request: Request = serde_json::from_slice(&bytes)?;
        drop(bytes);
        let pool = match request {
            Request::Subscribe => &self.subscribers,
            Request::Browse { .. } => &self.browse,
            _ => &self.controls,
        };
        let Ok(_slot) = pool.clone().try_acquire_owned() else {
            return respond(
                &mut socket,
                Reply::result(Err(anyhow::anyhow!("Service busy; retry this request"))),
            )
            .await;
        };
        drop(reader);
        let result = match request {
            Request::Subscribe => {
                let _registration = ClientRegistration::new(self.registry, pid);
                loop {
                    let status = self.updates.borrow_and_update().clone();
                    respond(
                        &mut socket,
                        Reply::result(Ok(serde_json::to_value(status)?)),
                    )
                    .await?;
                    let mut byte = [0; 1];
                    tokio::select! {
                        changed=self.updates.changed()=>if changed.is_err(){break},
                        _=socket.read(&mut byte)=>break,
                    }
                }
                return Ok(());
            }
            Request::Browse { kind, term, next } => {
                let status = self.updates.borrow().clone();
                let page = self
                    .catalog
                    .browse(
                        &kind,
                        &term,
                        next.as_deref(),
                        &status.catalog_country,
                        status.profile.limits(),
                    )
                    .await;
                if page.is_ok() && kind.starts_with("search:") && !term.trim().is_empty() {
                    let _ = send(&self.tx, Request::RecordSearch(term.trim().into())).await;
                }
                page.and_then(|p| Ok(serde_json::to_value(p)?))
            }
            other => send(&self.tx, other).await,
        };
        respond(&mut socket, Reply::result(result)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn write_deadline_releases_a_blocked_responder() {
        let (mut server, _stalled_client) = UnixStream::pair().unwrap();
        let reply = Reply::result(Ok(serde_json::Value::String("x".repeat(3 * 1024 * 1024))));
        let error = timeout(Duration::from_secs(4), respond(&mut server, reply))
            .await
            .expect("write has its own deadline")
            .unwrap_err();
        assert!(error.is::<tokio::time::error::Elapsed>());
    }
}
