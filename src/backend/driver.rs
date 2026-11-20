//! Injectable native transport boundary. Session state stays in the service.
use super::{
    controller::Controller,
    worker::{Control, Event},
};
use crate::config::Paths;
use anyhow::Result;
use std::{future::Future, os::unix::fs::MetadataExt, path::Path, pin::Pin, sync::Mutex};
use tokio::sync::mpsc;
pub type AudioFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub trait AudioDriver: Send {
    fn pid(&self) -> Option<u32>;
    fn send(&mut self, control: Control) -> AudioFuture<'_, Result<()>>;
    fn stop(self: Box<Self>) -> AudioFuture<'static, ()>;
}
pub trait DriverFactory: Send + Sync {
    fn refresh(&self) -> AudioFuture<'_, ()> {
        Box::pin(async {})
    }
    fn unavailable_reason(&self) -> &'static str {
        super::NATIVE_UNAVAILABLE
    }
    fn available(&self, paths: &Paths) -> bool;
    fn start(
        &self,
        id: String,
        volume: f64,
        paused: bool,
        generation: u64,
        events: mpsc::Sender<(u64, Event)>,
    ) -> AudioFuture<'_, Result<Box<dyn AudioDriver>>>;
}
#[derive(Default)]
pub struct NativeFactory {
    check: Mutex<HelperCheck>,
}
#[derive(Default)]
struct HelperCheck {
    file: Option<(u64, u64, i64, i64)>,
    compatible: bool,
}
impl NativeFactory {
    fn protocol_ready(&self) -> bool {
        self.check.lock().expect("helper check").compatible
    }
    async fn check_helper(&self, executable: Option<&Path>) {
        let file = executable
            .and_then(|path| path.metadata().ok())
            .map(|m| (m.dev(), m.ino(), m.ctime(), m.ctime_nsec()));
        if file == self.check.lock().expect("helper check").file {
            return;
        }
        let compatible = if let Some(exe) = executable {
            matches!(
                super::controller::probe_protocol(exe).await,
                Ok(rustune_core::protocol::PROTOCOL_VERSION)
            )
        } else {
            false
        };
        *self.check.lock().expect("helper check") = HelperCheck { file, compatible };
    }
}
impl DriverFactory for NativeFactory {
    fn refresh(&self) -> AudioFuture<'_, ()> {
        Box::pin(async {
            let path = std::env::current_exe()
                .ok()
                .and_then(|p| super::worker::audio_executable(&p).ok());
            self.check_helper(path.as_deref()).await;
        })
    }
    fn unavailable_reason(&self) -> &'static str {
        let check = self.check.lock().expect("helper check");
        if check.file.is_some() && !check.compatible {
            super::controller::PAIR_ERROR
        } else {
            super::NATIVE_UNAVAILABLE
        }
    }
    fn available(&self, paths: &Paths) -> bool {
        self.protocol_ready() && super::worker::available(paths)
    }
    fn start(
        &self,
        id: String,
        volume: f64,
        paused: bool,
        generation: u64,
        events: mpsc::Sender<(u64, Event)>,
    ) -> AudioFuture<'_, Result<Box<dyn AudioDriver>>> {
        Box::pin(async move {
            Ok(
                Box::new(Controller::start_paused(id, volume, paused, generation, events).await?)
                    as Box<dyn AudioDriver>,
            )
        })
    }
}
impl AudioDriver for Controller {
    fn pid(&self) -> Option<u32> {
        Controller::pid(self)
    }
    fn send(&mut self, control: Control) -> AudioFuture<'_, Result<()>> {
        Box::pin(Controller::send(self, control))
    }
    fn stop(self: Box<Self>) -> AudioFuture<'static, ()> {
        Box::pin(Controller::stop(*self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn an_executable_without_hello_cannot_advertise_native_playback() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("rustune-audio");
        std::fs::copy("/bin/true", &exe).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
        let factory = NativeFactory::default();
        factory.check_helper(Some(&exe)).await;
        assert!(!factory.protocol_ready());
        assert!(factory.unavailable_reason().contains("replace both"));
    }
}
