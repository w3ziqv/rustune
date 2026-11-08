//! Small CDP transport over Chromium's inherited pipes. No debugging port.
use crate::ipc::MAX_FRAME;
use anyhow::{Context, Result, bail, ensure};
use command_fds::{CommandFdExt, FdMapping};
use serde_json::{Value, json};
use std::{
    io::BufRead,
    os::unix::{net::UnixStream as StdStream, process::CommandExt},
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

pub fn read_message(reader: &mut impl BufRead) -> Result<Vec<u8>> {
    let mut b = Vec::new();
    std::io::Read::take(reader, MAX_FRAME as u64 + 1).read_until(0, &mut b)?;
    ensure!(
        b.len() <= MAX_FRAME && b.last() == Some(&0),
        "invalid CDP frame or size limit"
    );
    b.pop();
    Ok(b)
}

pub struct Cdp {
    child: Child,
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    id: u64,
    session: Option<String>,
}
impl Cdp {
    pub async fn launch(binary: &Path, profile: &Path, headless: bool) -> Result<Self> {
        let (writer, child_read) = StdStream::pair()?;
        let (child_write, reader) = StdStream::pair()?;
        let mut command = Command::new(binary);
        command
            .args([
                "--remote-debugging-pipe",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-background-networking",
                "--disable-sync",
            ])
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        if headless {
            command.arg("--headless=new");
        }
        command.fd_mappings(vec![
            FdMapping {
                parent_fd: child_read.into(),
                child_fd: 3,
            },
            FdMapping {
                parent_fd: child_write.into(),
                child_fd: 4,
            },
        ])?;
        let child = command.spawn().context("could not launch Chromium")?;
        drop(command);
        reader.set_nonblocking(true)?;
        writer.set_nonblocking(true)?;
        let mut c = Self {
            child,
            reader: BufReader::new(UnixStream::from_std(reader)?),
            writer: UnixStream::from_std(writer)?,
            id: 0,
            session: None,
        };
        c.call("Browser.getVersion", json!({}), false).await?;
        Ok(c)
    }
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
    pub async fn call(&mut self, method: &str, params: Value, in_page: bool) -> Result<Value> {
        self.id += 1;
        let id = self.id;
        let mut req = json!({"id":id,"method":method,"params":params});
        if in_page {
            req["sessionId"] = json!(self.session.as_ref().context("no browser page attached")?);
        }
        let mut bytes = serde_json::to_vec(&req)?;
        ensure!(bytes.len() < MAX_FRAME, "CDP request exceeds size limit");
        bytes.push(0);
        let io = async {
            self.writer.write_all(&bytes).await?;
            loop {
                let mut buf = Vec::with_capacity(4096);
                (&mut self.reader)
                    .take(MAX_FRAME as u64 + 1)
                    .read_until(0, &mut buf)
                    .await?;
                ensure!(
                    buf.len() <= MAX_FRAME && buf.last() == Some(&0),
                    "browser disconnected or CDP size limit"
                );
                buf.pop();
                let response: Value =
                    serde_json::from_slice(&buf).context("invalid browser response")?;
                if response["id"].as_u64() == Some(id) {
                    ensure!(
                        response.get("error").is_none(),
                        "browser rejected CDP command"
                    );
                    return Ok(response["result"].clone());
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(20), io)
            .await
            .context("browser command timed out; reconnect the backend")?
    }
    pub async fn attach(&mut self, url: &str) -> Result<()> {
        let targets = self.call("Target.getTargets", json!({}), false).await?;
        let target = targets["targetInfos"]
            .as_array()
            .and_then(|a| a.iter().find(|v| v["type"] == "page"))
            .and_then(|v| v["targetId"].as_str())
            .context("no Chromium page")?;
        let r = self
            .call(
                "Target.attachToTarget",
                json!({"targetId":target,"flatten":true}),
                false,
            )
            .await?;
        self.session = Some(
            r["sessionId"]
                .as_str()
                .context("no CDP session")?
                .to_owned(),
        );
        self.call("Page.navigate", json!({"url":url}), true).await?;
        Ok(())
    }
    pub async fn eval(&mut self, expression: &str) -> Result<Value> {
        let r=self.call("Runtime.evaluate",json!({"expression":expression,"awaitPromise":true,"returnByValue":true,"userGesture":true}),true).await?;
        if r.get("exceptionDetails").is_some() {
            bail!(
                "MusicKit operation failed (check authorization, subscription, network and Widevine)"
            );
        }
        Ok(r["result"]["value"].clone())
    }
    pub async fn minimize(&mut self) -> Result<()> {
        let r = self
            .call("Browser.getWindowForTarget", json!({}), true)
            .await?;
        self.call(
            "Browser.setWindowBounds",
            json!({"windowId":r["windowId"],"bounds":{"windowState":"minimized"}}),
            false,
        )
        .await?;
        Ok(())
    }
    pub async fn close(&mut self) {
        let _ = self.call("Browser.close", json!({}), false).await;
        for _ in 0..40 {
            if !self.alive() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}
impl Drop for Cdp {
    fn drop(&mut self) {
        if self.alive() {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(self.child.id() as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
