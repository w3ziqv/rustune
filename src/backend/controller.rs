//! Owned worker lifecycle and bounded private pipe messages.
use super::worker::{Control, Event};
use crate::ipc::write_frame;
use anyhow::{Result, anyhow, ensure};
use rustune_core::protocol::PROTOCOL_VERSION;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    process::{Child, ChildStdin, ChildStdout},
    sync::mpsc,
    task::JoinHandle,
};

pub(super) const PAIR_ERROR: &str = "Native audio protocol negotiation failed; replace both rustune and rustune-audio with a matching pair";

async fn read_event(stdout: &mut ChildStdout) -> Result<Event> {
    use tokio::io::AsyncReadExt;
    let size = stdout.read_u32().await? as usize;
    ensure!((1..=4096).contains(&size), "Invalid audio event size");
    let mut body = vec![0; size];
    stdout.read_exact(&mut body).await?;
    Ok::<Event, anyhow::Error>(serde_json::from_slice(&body)?)
}

async fn connect(executable: &Path) -> Result<(Child, ChildStdin, ChildStdout, u32)> {
    let mut child = tokio::process::Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .process_group(0)
        .spawn()
        .map_err(|_| anyhow!("Cannot start native audio worker"))?;

    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");
    let hello = async {
        write_frame(
            &mut stdin,
            &Control::Hello {
                version: PROTOCOL_VERSION,
            },
        )
        .await?;
        match read_event(&mut stdout).await? {
            Event::Hello { version } => Ok(version),
            _ => Err(anyhow!(PAIR_ERROR)),
        }
    };
    match tokio::time::timeout(Duration::from_secs(1), hello).await {
        Ok(Ok(version)) => Ok((child, stdin, stdout, version)),
        _ => {
            let _ = child.kill().await;
            Err(anyhow!(PAIR_ERROR))
        }
    }
}

pub async fn probe_protocol(executable: &Path) -> Result<u32> {
    let (mut child, _stdin, _stdout, version) = connect(executable).await?;
    let _ = child.kill().await;
    Ok(version)
}

pub struct Controller {
    child: Child,
    stdin: ChildStdin,
    reader: JoinHandle<()>,
}
impl Controller {
    pub async fn start(
        id: String,
        volume: f64,
        generation: u64,
        events: mpsc::Sender<(u64, Event)>,
    ) -> Result<Self> {
        Self::start_paused(id, volume, false, generation, events).await
    }
    pub async fn start_paused(
        id: String,
        volume: f64,
        paused: bool,
        generation: u64,
        events: mpsc::Sender<(u64, Event)>,
    ) -> Result<Self> {
        let command = Control::Prepare { id, volume, paused };
        command.validate()?;
        let executable = super::worker::audio_executable(&std::env::current_exe()?)?;
        Self::prepare(&executable, command, generation, events).await
    }
    async fn prepare(
        executable: &Path,
        command: Control,
        generation: u64,
        events: mpsc::Sender<(u64, Event)>,
    ) -> Result<Self> {
        let (mut child, mut stdin, mut stdout, version) = connect(executable).await?;
        if version != PROTOCOL_VERSION {
            let _ = child.kill().await;
            anyhow::bail!(PAIR_ERROR);
        }
        if write_frame(&mut stdin, &command).await.is_err() {
            let _ = child.kill().await;
            anyhow::bail!("Cannot prepare native audio worker");
        }
        let reader = tokio::spawn(async move {
            let mut terminal = false;
            loop {
                let event = read_event(&mut stdout).await;
                let Ok(event) = event else {
                    break;
                };
                terminal |= matches!(event, Event::Ended { .. } | Event::Error(_));
                if events.send((generation, event)).await.is_err() {
                    return;
                }
            }
            if !terminal {
                let _ = events
                    .send((
                        generation,
                        Event::Error("Native audio worker stopped unexpectedly".into()),
                    ))
                    .await;
            }
        });
        Ok(Self {
            child,
            stdin,
            reader,
        })
    }
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }
    pub async fn send(&mut self, command: Control) -> Result<()> {
        command.validate()?;
        tokio::time::timeout(
            Duration::from_secs(1),
            write_frame(&mut self.stdin, &command),
        )
        .await
        .map_err(|_| anyhow!("Native audio command timed out"))?
        .map_err(|_| anyhow!("Native audio worker is no longer available"))
    }
    pub async fn stop(mut self) {
        let _ = self.send(Control::Stop).await;
        if tokio::time::timeout(Duration::from_millis(300), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
            let _ = self.child.wait().await;
        }
        self.reader.abort();
        let _ = self.reader.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn incompatible_or_silent_helper_is_reaped_before_prepare() {
        use std::os::unix::fs::PermissionsExt;
        for silent in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let exe = dir.path().join("rustune-audio");
            let pid = dir.path().join("pid");
            let script = format!(
                r#"#!/usr/bin/python3
import json,os,struct,sys,time
from pathlib import Path
Path(__file__).with_name("pid").write_text(str(os.getpid()))
pipe=sys.stdin.buffer
size=struct.unpack(">I",pipe.read(4))[0]
hello=json.loads(pipe.read(size))
if hello["command"]!="hello":
    Path(__file__).with_name("prepared").touch()
if {reply}:
    data=b'{{"event":"hello","data":{{"version":1}}}}'
    sys.stdout.buffer.write(struct.pack(">I",len(data))+data)
    sys.stdout.buffer.flush()
if pipe.read(1):
    Path(__file__).with_name("prepared").touch()
time.sleep(20)
"#,
                reply = if silent { "False" } else { "True" }
            );
            std::fs::write(&exe, script).unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
            let (events, _) = mpsc::channel(2);
            let cmd = Control::Prepare {
                id: "12".into(),
                volume: 0.0,
                paused: true,
            };
            let error = tokio::time::timeout(
                Duration::from_secs(3),
                Controller::prepare(&exe, cmd, 1, events),
            )
            .await
            .unwrap()
            .err()
            .unwrap();
            assert!(
                error
                    .to_string()
                    .contains("replace both rustune and rustune-audio")
            );
            assert!(!dir.path().join("prepared").exists());
            assert!(
                !Path::new("/proc")
                    .join(std::fs::read_to_string(pid).unwrap())
                    .exists()
            );
        }
    }
    #[tokio::test]
    async fn invalid_start_is_rejected_before_any_child_is_spawned() {
        let (events, _) = mpsc::channel(1);
        assert!(
            Controller::start("../invalid".into(), 0.7, 1, events)
                .await
                .is_err()
        );
    }
}
