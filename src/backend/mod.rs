pub mod cdp;
pub mod musickit;
use crate::model::Page;
use anyhow::Result;
pub fn validate_api_path(path: &str) -> Result<()> {
    anyhow::ensure!(
        path.len() <= 4096
            && path.is_ascii()
            && !path.bytes().any(|b| b <= 32 || b == 127)
            && !path.contains("..")
            && !path.contains('\\'),
        "invalid API path"
    );
    let (base, query) = path.split_once('?').unwrap_or((path, ""));
    anyhow::ensure!(
        !base.contains('%')
            && (base.starts_with("/v1/me/library/")
                || base.starts_with("/v1/me/recent/")
                || base.starts_with("/v1/catalog/")),
        "unsupported API path"
    );
    for param in query.split('&').filter(|p| !p.is_empty()) {
        let (key, _) = param
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("invalid pagination query"))?;
        anyhow::ensure!(
            ["offset", "limit", "l", "include", "extend", "term", "types"].contains(&key),
            "unsupported pagination query"
        );
    }
    Ok(())
}
pub trait PlaybackBackend {
    fn snapshot(&mut self) -> impl std::future::Future<Output = Result<crate::model::Playback>>;
    fn execute(
        &mut self,
        op: &str,
        args: serde_json::Value,
    ) -> impl std::future::Future<Output = Result<serde_json::Value>>;
}
pub trait MusicService {
    fn browse(
        &mut self,
        kind: &str,
        term: &str,
        next: Option<&str>,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Page>>;
}
pub mod firefox;
