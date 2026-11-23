//! Bound each connection phase independently so long-lived work cannot occupy control slots.
use super::{ClientRegistration, Reply, Sender, Status, send};
use crate::{
    catalog::Catalog,
    ipc::{Request, read_limited_frame, write_frame, write_payload},
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
async fn respond_private(
    socket: &mut UnixStream,
    catalog: &Catalog,
    generation: Option<u64>,
    reply: Reply,
) -> Result<()> {
    let payload = serde_json::to_vec(&reply)?;
    let lock = catalog.private_publication().and_then(|lock| {
        anyhow::ensure!(
            generation.is_some() && catalog.account_generation() == generation,
            "Account credentials changed; restart the library search"
        );
        Ok(lock)
    });
    match lock {
        Ok(_lock) => timeout(WRITE_TIMEOUT, write_payload(socket, &payload)).await?,
        Err(error) => respond(socket, Reply::result(Err(error))).await,
    }
}
// Status snapshots may wait in the watch channel or the engine's reply queue.
async fn respond_status(
    socket: &mut UnixStream,
    catalog: &Catalog,
    mut reply: Reply,
) -> Result<()> {
    if reply
        .data
        .get("playlist_write")
        .is_some_and(serde_json::Value::is_object)
    {
        let generation = reply.data["playlist_write"]["account_generation"].as_u64();
        let payload = serde_json::to_vec(&reply)?;
        if let Ok(_lock) = catalog.private_publication()
            && generation.is_some()
            && catalog.account_generation() == generation
            && reply.data["account_generation"].as_u64() == generation
        {
            return timeout(WRITE_TIMEOUT, write_payload(socket, &payload)).await?;
        }
        reply.data["playlist_write"] = serde_json::Value::Null;
        reply.data["account_generation"] = serde_json::to_value(catalog.account_generation())?;
    }
    respond(socket, reply).await
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
                    respond_status(
                        &mut socket,
                        &self.catalog,
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
                let private = crate::catalog::playlist::private_kind(&kind);
                let browse = async {
                    let page = self
                        .catalog
                        .browse(
                            &kind,
                            &term,
                            next.as_deref(),
                            &status.catalog_country,
                            status.profile.limits(),
                        )
                        .await?;
                    if !private && kind.starts_with("search:") && !term.trim().is_empty() {
                        let _ = send(&self.tx, Request::RecordSearch(term.trim().into())).await;
                    }
                    Ok(page)
                };
                let mut input = [0];
                let page = tokio::select! {
                    biased;
                    _ = socket.read(&mut input) => return Ok(()),
                    result = browse => result,
                };
                if private && let Ok(page) = page {
                    let generation = page.account_generation;
                    let reply = Reply::result(Ok(serde_json::to_value(page)?));
                    return respond_private(&mut socket, &self.catalog, generation, reply).await;
                }
                page.and_then(|page| Ok(serde_json::to_value(page)?))
            }
            other => send(&self.tx, other).await,
        };
        respond_status(&mut socket, &self.catalog, Reply::result(result)).await
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

    #[tokio::test]
    async fn token_write_waits_for_private_publication_and_old_reply_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let paths = crate::catalog::playlist_tests::paths(directory.path());
        let catalog = Arc::new(Catalog::new(&paths).unwrap());
        let generation = catalog.account_generation();
        let (mut server, mut client) = UnixStream::pair().unwrap();
        let sender = tokio::spawn({
            let catalog = catalog.clone();
            async move {
                let reply =
                    Reply::result(Ok(serde_json::Value::String("x".repeat(3 * 1024 * 1024))));
                respond_private(&mut server, &catalog, generation, reply).await
            }
        });
        let size = timeout(Duration::from_secs(1), client.read_u32())
            .await
            .unwrap()
            .unwrap();
        assert!(size > 3 * 1024 * 1024);
        // The header proves the responder acquired the lock and started publishing.
        let lockfile = std::fs::File::open(paths.config.join("music-user-token.lock")).unwrap();
        let Err((_, denied)) =
            nix::fcntl::Flock::lock(lockfile, nix::fcntl::FlockArg::LockExclusiveNonblock)
        else {
            panic!("an account mutation must not overtake a partially written private reply");
        };
        assert_eq!(denied, nix::errno::Errno::EWOULDBLOCK);
        assert_eq!(catalog.account_generation(), generation);
        drop(client);
        assert!(sender.await.unwrap().is_err());
        crate::auth::save_user_token(
            &paths,
            &crate::auth::SecretToken::parse("replacement").unwrap(),
        )
        .unwrap();
        assert_ne!(catalog.account_generation(), generation);

        let (mut server, mut client) = UnixStream::pair().unwrap();
        let reply = Reply::result(Ok(serde_json::json!("old private value")));
        respond_private(&mut server, &catalog, generation, reply)
            .await
            .unwrap();
        let response = crate::ipc::read_frame(&mut client).await.unwrap();
        let reply: Reply = serde_json::from_slice(&response).unwrap();
        assert!(
            reply
                .value()
                .unwrap_err()
                .to_string()
                .contains("credentials changed")
        );
    }

    #[tokio::test]
    async fn delayed_status_and_subscription_snapshot_hide_a_previous_accounts_write() {
        let directory = tempfile::tempdir().unwrap();
        let paths = crate::catalog::playlist_tests::paths(directory.path());
        let catalog = Catalog::new(&paths).unwrap();
        let old = catalog.account_generation();
        let snapshot = serde_json::json!({
            "account_generation":old,
            "playlist_write":{
                "account_generation":old,
                "phase":"completed",
                "result":{"playlist_id":"p.from-old-account","tracks":2}
            }
        });
        let catalog = &catalog;
        let publish = |data| async move {
            let (mut socket, mut peer) = UnixStream::pair().unwrap();
            respond_status(&mut socket, catalog, Reply::result(Ok(data)))
                .await
                .unwrap();
            let body = crate::ipc::read_frame(&mut peer).await.unwrap();
            serde_json::from_slice::<Reply>(&body)
                .unwrap()
                .value()
                .unwrap()
        };
        assert_eq!(
            publish(snapshot.clone()).await["playlist_write"]["result"]["tracks"],
            2
        );
        crate::auth::save_user_token(&paths, &crate::auth::SecretToken::parse("next").unwrap())
            .unwrap();
        let safe = publish(snapshot).await;
        assert!(safe["playlist_write"].is_null());
        assert_eq!(
            safe["account_generation"].as_u64(),
            catalog.account_generation()
        );
        assert!(!safe.to_string().contains("p.from-old-account"));
    }
}
