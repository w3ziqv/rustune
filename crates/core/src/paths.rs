use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
#[derive(Clone, Debug)]
pub struct Paths {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub runtime: PathBuf,
}
impl Paths {
    pub fn discover() -> Result<Self> {
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME not set")?);
        let base = |key: &str, fallback: PathBuf| {
            std::env::var_os(key)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or(fallback)
        };
        let data = base("XDG_DATA_HOME", home.join(".local/share")).join("rustune");
        let runtime = base("XDG_RUNTIME_DIR", data.clone()).join("rustune");
        Ok(Self {
            config: base("XDG_CONFIG_HOME", home.join(".config")).join("rustune"),
            cache: base("XDG_CACHE_HOME", home.join(".cache")).join("rustune"),
            data,
            runtime,
        })
    }
    pub fn init(&self) -> Result<()> {
        for p in [&self.config, &self.data, &self.cache, &self.runtime] {
            private_dir(p)?;
        }
        Ok(())
    }
    pub fn socket(&self) -> PathBuf {
        self.runtime.join("service.sock")
    }
}

pub fn private_dir(p: &Path) -> Result<()> {
    fs::create_dir_all(p)?;
    ensure!(
        !fs::symlink_metadata(p)?.file_type().is_symlink(),
        "private directory cannot be a symlink"
    );
    fs::set_permissions(p, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

pub fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("file needs parent directory")?;
    private_dir(parent)?;
    let tmp = parent.join(format!(
        ".write-{}-{:016x}",
        std::process::id(),
        fastrand::u64(..)
    ));
    let result = (|| -> Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
