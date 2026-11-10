//! Injectable native transport boundary. Session state stays in the service.
use super::{
    controller::Controller,
    worker::{Control, Event},
};
use crate::config::Paths;
use anyhow::Result;
use std::{future::Future, pin::Pin};
use tokio::sync::mpsc;
pub type AudioFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub trait AudioDriver: Send {
    fn pid(&self) -> Option<u32>;
    fn send(&mut self, control: Control) -> AudioFuture<'_, Result<()>>;
    fn stop(self: Box<Self>) -> AudioFuture<'static, ()>;
}
pub trait DriverFactory: Send + Sync {
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
pub struct NativeFactory;
impl DriverFactory for NativeFactory {
    fn available(&self, paths: &Paths) -> bool {
        super::worker::available(paths)
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
