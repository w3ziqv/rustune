//! Coalesce identical reads without retaining work after the last caller leaves.
use anyhow::Result;
use futures_util::future::{BoxFuture, FutureExt, Shared, WeakShared};
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    hash::Hash,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

type Work<V> = BoxFuture<'static, std::result::Result<V, Arc<anyhow::Error>>>;

pub(super) struct Reads<K, V>(Mutex<HashMap<K, WeakShared<Work<V>>>>);

impl<K: Eq + Hash, V: Clone + Send + Sync + 'static> Reads<K, V> {
    pub fn new() -> Self {
        Self(Mutex::new(HashMap::new()))
    }

    pub async fn read(
        &self,
        key: K,
        work: impl Future<Output = Result<V>> + Send + 'static,
    ) -> Result<V> {
        self.join(key, work)
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))
    }

    fn join(
        &self,
        key: K,
        work: impl Future<Output = Result<V>> + Send + 'static,
    ) -> Shared<Work<V>> {
        let mut reads = self.0.lock().expect("catalog reads");
        if let Some(pending) = reads.get(&key).and_then(WeakShared::upgrade) {
            return pending;
        }
        reads.retain(|_, pending| pending.upgrade().is_some());
        let pending = work.map(|result| result.map_err(Arc::new)).boxed().shared();
        reads.insert(key, pending.downgrade().expect("new shared read"));
        pending
    }
}

#[derive(Default)]
pub(super) struct Budget {
    calls: VecDeque<Instant>,
    blocked_until: Option<Instant>,
}

impl Budget {
    pub fn reserve(&mut self, provider: &str) -> Result<()> {
        let now = Instant::now();
        if let Some(until) = self.blocked_until.filter(|until| *until > now) {
            anyhow::bail!(
                "{provider} rate limit; retry in {} seconds",
                until.duration_since(now).as_secs() + 1
            );
        }
        while self
            .calls
            .front()
            .is_some_and(|time| now.duration_since(*time) >= Duration::from_secs(60))
        {
            self.calls.pop_front();
        }
        anyhow::ensure!(
            self.calls.len() < 20,
            "{provider} request limit reached; retry in a minute"
        );
        self.calls.push_back(now);
        Ok(())
    }

    pub fn block(&mut self, seconds: u64) {
        let until = Instant::now() + Duration::from_secs(seconds);
        self.blocked_until = Some(
            self.blocked_until
                .map_or(until, |current| current.max(until)),
        );
    }
}
