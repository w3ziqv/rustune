//! Official MusicKit JS host. Credentials travel only on the inherited CDP pipe.
use crate::{
    backend::{MusicService, PlaybackBackend, cdp::Cdp, validate_api_path},
    config::{BackendKind, Config, Paths},
    model::{Page, Playback},
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};

pub const NATIVE_UNAVAILABLE: &str = "Native Linux playback is not implemented: a Music User Token authenticates the account but does not supply a DRM playback engine. No browser fallback was started.";

pub struct MusicKit {
    pub browser: Option<Cdp>,
    pub config: Config,
    pub paths: Paths,
    pub login_mode: bool,
    web: bool,
    firefox_pid: Option<u32>,
    document: Option<tokio::task::JoinHandle<()>>,
}
impl MusicKit {
    pub fn new(config: Config, paths: Paths) -> Self {
        Self {
            browser: None,
            config,
            paths,
            login_mode: false,
            web: false,
            firefox_pid: None,
            document: None,
        }
    }
    pub fn binary(&self) -> Option<PathBuf> {
        browser_binary(&self.config)
    }
    pub fn pid(&self) -> Option<u32> {
        self.firefox_pid
            .or_else(|| self.browser.as_ref().map(Cdp::pid))
    }
    pub async fn start(&mut self, login: bool) -> Result<()> {
        let result = self.start_inner(login).await;
        if result.is_err() {
            self.close().await;
        }
        result
    }
    async fn start_inner(&mut self, login: bool) -> Result<()> {
        ensure!(
            self.config.backend != BackendKind::Disabled,
            NATIVE_UNAVAILABLE
        );
        if self.config.backend == BackendKind::Firefox {
            if self.firefox_pid.is_none() || login {
                let (pid, _) = super::firefox::request(
                    &self.paths,
                    if login { "login" } else { "snapshot" },
                    json!({}),
                )
                .await?;
                self.firefox_pid = Some(pid);
            }
            return Ok(());
        }
        if self.browser.as_mut().is_some_and(Cdp::alive) {
            if !login || self.login_mode {
                return Ok(());
            }
            self.close().await;
        }
        self.browser = None;
        let token = match self.config.backend {
            BackendKind::Firefox | BackendKind::Disabled => unreachable!("handled above"),
            BackendKind::MusicKit => Some(self.paths.token()?),
            BackendKind::AppleWeb => None,
            BackendKind::Auto => self.paths.token().ok(),
        };
        self.web = token.is_none();
        let url = if self.web {
            "https://music.apple.com/".to_owned()
        } else {
            self.document = Some(serve_document(self.config.bridge_port).await?);
            format!("http://127.0.0.1:{}/", self.config.bridge_port)
        };
        let binary = self
            .binary()
            .context("no Chromium browser found; configure browser in config.toml")?;
        let mut c = Cdp::launch(
            &binary,
            &self.paths.data.join("browser"),
            !login && self.config.headless,
        )
        .await?;
        c.attach(&url).await?;
        let mut ready = false;
        for _ in 0..200 {
            let check = if self.web {
                "typeof MusicKit !== 'undefined' && !!MusicKit.getInstance()"
            } else {
                "typeof MusicKit !== 'undefined'"
            };
            if c.eval(check).await?.as_bool() == Some(true) {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        ensure!(
            ready,
            "MusicKit library did not load; check network access to js-cdn.music.apple.com"
        );
        let instance = if let Some(token) = token {
            format!(
                "await MusicKit.configure({})",
                serde_json::to_string(
                    &json!({"developerToken":token,"app":{"name":"Rustune","build":env!("CARGO_PKG_VERSION")}})
                )?
            )
        } else {
            "MusicKit.getInstance()".to_owned()
        };
        let script = include_str!("musickit.js").replace("__RUSTUNE_INSTANCE__", &instance);
        c.eval(&script).await?;
        c.eval(&format!(
            "MusicKit.getInstance().volume = {}",
            self.config.volume
        ))
        .await?;
        self.browser = Some(c);
        self.login_mode = login;
        if !login && !self.config.headless {
            self.browser
                .as_mut()
                .context("browser unavailable")?
                .minimize()
                .await?;
        }
        if login {
            // Do not await the authorize promise: Apple ID and 2FA need human interaction.
            self.browser.as_mut().context("browser unavailable")?.eval("void MusicKit.getInstance().authorize().catch(() => { window.rustuneAuthFailed = true; })").await?;
        }
        Ok(())
    }
    pub async fn close(&mut self) {
        self.firefox_pid = None; // Firefox belongs to the user; never close or kill it.
        if let Some(mut c) = self.browser.take() {
            c.close().await;
        }
        self.login_mode = false;
        if let Some(document) = self.document.take() {
            document.abort();
            let _ = document.await;
        }
    }
    pub async fn finish_login(&mut self) -> Result<()> {
        if self.config.headless {
            self.close().await;
        } else {
            if let Some(c) = self.browser.as_mut() {
                c.minimize().await?;
            }
            self.login_mode = false;
        }
        Ok(())
    }
    pub fn name(&self) -> &'static str {
        if self.config.backend == BackendKind::Disabled {
            "No playback engine (native Linux playback unavailable)"
        } else if self.config.backend == BackendKind::Firefox {
            "Firefox (shared existing session) / MusicKit JS"
        } else if self.web {
            "Apple web player / MusicKit JS / Chromium"
        } else {
            "MusicKit JS local host / Chromium"
        }
    }
}
impl Drop for MusicKit {
    fn drop(&mut self) {
        if let Some(document) = self.document.take() {
            document.abort();
        }
    }
}
impl PlaybackBackend for MusicKit {
    async fn execute(&mut self, op: &str, args: Value) -> Result<Value> {
        self.start(false).await?;
        let r = if self.config.backend == BackendKind::Firefox {
            match super::firefox::request(
                &self.paths,
                "dispatch",
                json!({"action":op,"parameters":args}),
            )
            .await
            {
                Ok((pid, value)) => {
                    self.firefox_pid = Some(pid);
                    Ok(value)
                }
                Err(e) => Err(e),
            }
        } else {
            let expression = format!(
                "window.rustune.dispatch({}, {})",
                serde_json::to_string(op)?,
                serde_json::to_string(&args)?
            );
            self.browser
                .as_mut()
                .context("browser unavailable")?
                .eval(&expression)
                .await
        };
        if r.is_err() {
            self.close().await;
        }
        let v = r?;
        if let Some(code) = v.get("error").and_then(Value::as_str) {
            match code {
                "unauthorized" => {
                    bail!("authorize the configured legacy backend: rustune login --browser")
                }
                "preview" => bail!(
                    "full-song playback unavailable; previews are disabled (check subscription/DRM)"
                ),
                "network" => bail!(
                    "MusicKit request failed; check network, subscription, authorization and token expiry"
                ),
                _ => bail!("MusicKit could not perform this operation"),
            }
        }
        Ok(v)
    }
    async fn snapshot(&mut self) -> Result<Playback> {
        if self.config.backend == BackendKind::Firefox {
            let (pid, value) = super::firefox::request(&self.paths, "snapshot", json!({})).await?;
            self.firefox_pid = Some(pid);
            return Ok(serde_json::from_value(value)?);
        }
        let Some(c) = self.browser.as_mut() else {
            return Ok(Playback::default());
        };
        let v = c.eval("window.rustune.snapshot()").await?;
        serde_json::from_value(v).context("invalid playback state from MusicKit")
    }
}
impl MusicService for MusicKit {
    async fn browse(
        &mut self,
        kind: &str,
        term: &str,
        next: Option<&str>,
        limit: usize,
    ) -> Result<Page> {
        ensure!(term.len() <= 512, "search too long (512 bytes maximum)");
        if let Some(path) = next {
            validate_api_path(path)?;
        }
        let r = self
            .execute(
                "browse",
                json!({"kind":kind,"term":term,"next":next,"limit":limit.min(50)}),
            )
            .await?;
        let page: Page = serde_json::from_value(r).context("invalid MusicKit catalog response")?;
        ensure!(page.items.len() <= 100, "MusicKit page exceeds item limit");
        if let Some(path) = &page.next {
            validate_api_path(path)?;
        }
        Ok(page)
    }
}
pub fn browser_binary(config: &Config) -> Option<PathBuf> {
    if let Some(p) = &config.browser {
        return p.is_file().then(|| p.clone());
    }
    let path = std::env::var_os("PATH")?;
    for name in [
        "chromium",
        "google-chrome-stable",
        "google-chrome",
        "chromium-browser",
    ] {
        for dir in std::env::split_paths(&path) {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Serves only a static document. No tokens, APIs or controls are exposed on HTTP.
/// Host validation prevents DNS rebinding; cross-origin framing is refused.
pub async fn serve_document(port: u16) -> Result<tokio::task::JoinHandle<()>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .context("MusicKit local origin port is occupied; choose bridge_port in config.toml")?;
    Ok(tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            // One bounded request at a time; no per-connection background task growth.
            let mut b = [0u8; 4096];
            let Ok(Ok(n)) = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut b)).await
            else {
                continue;
            };
            let req = String::from_utf8_lossy(&b[..n]);
            let valid = req.starts_with("GET / HTTP/1.1\r\n")
                && req
                    .lines()
                    .any(|s| s.eq_ignore_ascii_case(&format!("Host: 127.0.0.1:{port}")));
            let body = if valid {
                "<!doctype html><html><head><meta charset='utf-8'><title>Rustune MusicKit authorization</title><script src='https://js-cdn.music.apple.com/musickit/v3/musickit.js'></script></head><body><p>Complete Apple Music authorization, then return to Rustune.</p></body></html>"
            } else {
                "Not found"
            };
            let reply = format!(
                "HTTP/1.1 {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Frame-Options: DENY\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{}",
                if valid { "200 OK" } else { "404 Not Found" },
                body.len(),
                body
            );
            let _ =
                tokio::time::timeout(Duration::from_secs(2), stream.write_all(reply.as_bytes()))
                    .await;
        }
    }))
}
