//! Deterministic test transport; it cannot authorize Apple or produce audio.
use super::{
    driver::*,
    worker::{Control, Event},
};
use crate::config::Paths;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
#[derive(Default, Clone)]
pub(crate) struct Controlled {
    state: Arc<Mutex<State>>,
}
#[derive(Default)]
struct State {
    starts: Vec<(String, bool)>,
    commands: Vec<String>,
    events: Option<(u64, mpsc::Sender<(u64, Event)>)>,
}
impl Controlled {
    pub fn starts(&self) -> Vec<(String, bool)> {
        self.state.lock().unwrap().starts.clone()
    }
    pub fn commands(&self) -> Vec<String> {
        self.state.lock().unwrap().commands.clone()
    }
    pub async fn emit(&self, event: Event) {
        let (g, tx) = self
            .state
            .lock()
            .unwrap()
            .events
            .clone()
            .expect("test worker started");
        tx.send((g, event)).await.unwrap();
    }
}
impl DriverFactory for Controlled {
    fn available(&self, _: &Paths) -> bool {
        true
    }
    fn start(
        &self,
        id: String,
        _: f64,
        paused: bool,
        generation: u64,
        events: mpsc::Sender<(u64, Event)>,
    ) -> AudioFuture<'_, anyhow::Result<Box<dyn AudioDriver>>> {
        let mut state = self.state.lock().unwrap();
        state.starts.push((id, paused));
        state.events = Some((generation, events));
        let driver = self.clone();
        Box::pin(async move { Ok(Box::new(driver) as Box<dyn AudioDriver>) })
    }
}
impl AudioDriver for Controlled {
    fn pid(&self) -> Option<u32> {
        Some(std::process::id())
    }
    fn send(&mut self, command: Control) -> AudioFuture<'_, anyhow::Result<()>> {
        self.state
            .lock()
            .unwrap()
            .commands
            .push(format!("{command:?}"));
        Box::pin(async { Ok(()) })
    }
    fn stop(self: Box<Self>) -> AudioFuture<'static, ()> {
        Box::pin(async {})
    }
}
