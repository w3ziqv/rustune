//! Firefox Native Messaging bridge. Only fixed Apple Music operations cross stdio.
use crate::{
    config::{Paths, private_write},
    ipc::{read_frame, write_frame},
    service::Reply,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    time::Duration,
};
const LIMIT: usize = 1024 * 1024;
pub const ID: &str = "rustune@rustune.local";
pub fn read_native(reader: &mut impl Read) -> Result<Value> {
    let mut header = [0u8; 4];
    reader.read_exact(&mut header)?;
    let n = u32::from_ne_bytes(header) as usize;
    ensure!(n > 0 && n <= LIMIT, "Native Messaging size limit");
    let mut b = vec![0u8; n];
    reader.read_exact(&mut b)?;
    Ok(serde_json::from_slice(&b)?)
}
pub fn write_native(writer: &mut impl Write, value: &Value) -> Result<()> {
    let b = serde_json::to_vec(value)?;
    ensure!(b.len() <= LIMIT, "Native Messaging size limit");
    writer.write_all(&(b.len() as u32).to_ne_bytes())?;
    writer.write_all(&b)?;
    writer.flush()?;
    Ok(())
}
pub async fn request(paths: &Paths, operation: &str, args: Value) -> Result<(u32, Value)> {
    let task = async {
        let mut s=tokio::net::UnixStream::connect(paths.runtime.join("firefox.sock")).await.context("Firefox bridge is not connected. Run rustune firefox-setup and load its Firefox add-on (docs/firefox.md)")?;
        write_frame(&mut s, &json!({"operation":operation,"args":args})).await?;
        let response = read_frame(&mut s).await?;
        let v = serde_json::from_slice::<Reply>(&response)?.value()?;
        let pid = v["browser_pid"]
            .as_u64()
            .filter(|p| *p <= u32::MAX as u64)
            .context("invalid Firefox process ID")? as u32;
        Ok((pid, v["result"].clone()))
    };
    tokio::time::timeout(Duration::from_secs(25), task)
        .await
        .context("Firefox bridge timed out")?
}
fn firefox_parent() -> u32 {
    let mut pid = nix::unistd::getppid().as_raw() as u32;
    let mut root = pid;
    for _ in 0..32 {
        let Ok(comm) = fs::read_to_string(format!("/proc/{pid}/comm")) else {
            break;
        };
        if comm.trim().to_ascii_lowercase().contains("firefox") {
            root = pid;
        }
        let Some((parent, _, _)) = fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| crate::metrics::parse_stat(&s))
        else {
            break;
        };
        if parent <= 1 || parent == pid {
            break;
        }
        pid = parent;
    }
    root
}
pub async fn native_host(paths: Paths, args: &[String]) -> Result<()> {
    ensure!(
        args.last().is_some_and(|a| a == ID),
        "native-host is reserved for the Rustune Firefox extension"
    );
    paths.init()?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(paths.runtime.join("firefox.lock"))?;
    let _lock = nix::fcntl::Flock::lock(lock, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .map_err(|_| anyhow::anyhow!("Firefox bridge already running"))?;
    let socket = paths.runtime.join("firefox.sock");
    if socket.exists() {
        fs::remove_file(&socket)?;
    }
    let listener = tokio::net::UnixListener::bind(&socket)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    let uid = fs::metadata(&paths.runtime)?.uid();
    let pid = firefox_parent();
    let (tx, mut incoming) = tokio::sync::mpsc::channel(2);
    std::thread::spawn(move || {
        let mut input = std::io::stdin().lock();
        loop {
            let value = read_native(&mut input);
            let stop = value.is_err();
            if tx.blocking_send(value).is_err() || stop {
                break;
            }
        }
    });
    let mut seq = 0u64;
    loop {
        tokio::select! {
            // No unsolicited data or logs are accepted from browser pages.
            value=incoming.recv()=>{if !matches!(value,Some(Ok(_))){break;}}
            accepted=listener.accept()=>{
                let (mut stream,_)=accepted?;
                if stream.peer_cred()?.uid()!=uid{continue;}
                let operation=async {
                    let b=read_frame(&mut stream).await?;ensure!(b.len()<=64*1024,"Firefox command size limit");
                    let mut command:Value=serde_json::from_slice(&b)?;
                    ensure!(["snapshot","dispatch","login"].contains(&command["operation"].as_str().unwrap_or("")),"unsupported Firefox operation");
                    seq+=1;command["id"]=json!(seq);write_native(&mut std::io::stdout().lock(),&command)?;
                    loop {
                        let response=incoming.recv().await.context("Firefox disconnected")??;
                        if response["id"].as_u64()!=Some(seq){continue;}
                        let stage=response["stage"].as_str().filter(|s|["validate","permissions","find_tab","install_adapter","dispatch"].contains(s)).unwrap_or("unknown");
                        let hint=response["hint"].as_str().filter(|s|["no_music_tab","pinned_tab_closed","wrong_origin","url_not_visible","none"].contains(s)).unwrap_or("none");
                        ensure!(response["ok"]==true,"Firefox bridge failed at {stage} ({hint}); keep a signed-in music.apple.com tab open and reload the Rustune add-on if needed");
                        return Ok(json!({"browser_pid":pid,"result":response["result"]}));
                    }
                };
                let result=match tokio::time::timeout(Duration::from_secs(20),operation).await {Ok(r)=>r,Err(_)=>Err(anyhow::anyhow!("Firefox request timed out"))};
                let _=write_frame(&mut stream,&Reply::result(result)).await;
            }
        }
    }
    let _ = fs::remove_file(socket);
    Ok(())
}
pub fn setup(paths: &Paths) -> Result<std::path::PathBuf> {
    paths.init()?;
    let dir = paths.data.join("firefox");
    crate::config::private_dir(&dir)?;
    for (name, content) in [
        (
            "manifest.json",
            include_str!("../../extensions/firefox/manifest.json"),
        ),
        (
            "background.js",
            include_str!("../../extensions/firefox/background.js"),
        ),
        (
            "adapter.js",
            include_str!("../../extensions/firefox/adapter.js"),
        ),
    ] {
        private_write(&dir.join(name), content.as_bytes())?;
    }
    let binary = std::env::current_exe()?.canonicalize()?;
    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\"'\"'"));
    let wrapper = dir.join("native-host");
    private_write(
        &wrapper,
        format!(
            "#!/bin/sh\nexec {} native-host \"$@\"\n",
            quote(&binary.to_string_lossy())
        )
        .as_bytes(),
    )?;
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700))?;
    let hosts = std::path::PathBuf::from(std::env::var_os("HOME").context("HOME missing")?)
        .join(".mozilla/native-messaging-hosts");
    private_write(&hosts.join("org.rustune.bridge.json"),serde_json::to_string_pretty(&json!({"name":"org.rustune.bridge","description":"Rustune Apple Music controller","path":wrapper,"type":"stdio","allowed_extensions":[ID]}))?.as_bytes())?;
    Ok(dir.join("manifest.json"))
}
