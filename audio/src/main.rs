#![cfg(target_os = "linux")]
mod apple;
mod cenc;
mod equalizer;
mod pcm;
mod worker;
mod config {
    pub use rustune_core::paths::Paths;
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("rustune-audio {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let paths = rustune_core::paths::Paths::discover()?;
    worker::run(&paths).await
}
