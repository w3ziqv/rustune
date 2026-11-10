//! Lightweight frontend of the private native audio helper.
use crate::config::Paths;
use anyhow::{Result, anyhow, ensure};
pub use rustune_core::protocol::{Control, Event};
use std::{
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Stdio,
};
pub fn audio_executable(binary: &Path) -> Result<PathBuf> {
    let helper = binary.with_file_name("rustune-audio");
    ensure!(
        std::fs::metadata(&helper)
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0),
        "Native audio helper missing or not executable; install rustune-audio beside rustune"
    );
    Ok(helper)
}
pub fn available(paths: &Paths) -> bool {
    crate::auth::has_user_token(paths)
        && paths.cache.join("drm/libwidevinecdm.so").is_file()
        && std::env::current_exe()
            .ok()
            .is_some_and(|p| audio_executable(&p).is_ok())
}
pub async fn run(_paths: &Paths) -> Result<()> {
    let helper = audio_executable(&std::env::current_exe()?)?;
    let _ = std::process::Command::new(helper)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::null())
        .exec();
    Err(anyhow!("Could not execute the native audio helper"))
}
