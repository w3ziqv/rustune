//! Owned worker lifecycle and bounded private pipe messages.
use super::worker::{Control, Event};
use crate::ipc::write_frame;
use anyhow::{Result, anyhow, ensure};
use std::{process::Stdio, time::Duration};
use tokio::{
    process::{Child, ChildStdin},
    sync::mpsc,
    task::JoinHandle,
};

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
        let command = Control::Start { id, volume };
        command.validate()?;
        let executable = super::worker::audio_executable(&std::env::current_exe()?)?;
        let mut child = tokio::process::Command::new(executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .process_group(0)
            .spawn()
            .map_err(|_| anyhow!("Cannot start native audio worker"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Missing private audio command pipe"))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Missing private audio event pipe"))?;
        write_frame(&mut stdin, &command).await?;
        let reader = tokio::spawn(async move {
            let mut terminal = false;
            loop {
                let event = async {
                    use tokio::io::AsyncReadExt;
                    let size = stdout.read_u32().await? as usize;
                    ensure!((1..=4096).contains(&size), "Invalid audio event size");
                    let mut body = vec![0; size];
                    stdout.read_exact(&mut body).await?;
                    Ok::<Event, anyhow::Error>(serde_json::from_slice(&body)?)
                }
                .await;
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
    async fn invalid_start_is_rejected_before_any_child_is_spawned() {
        let (events, _) = mpsc::channel(1);
        assert!(
            Controller::start("../invalid".into(), 0.7, 1, events)
                .await
                .is_err()
        );
    }
}
