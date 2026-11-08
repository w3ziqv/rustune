use rustune::backend::firefox::{read_native, write_native};
#[test]
fn native_messaging_uses_native_endian_length_and_bounds() {
    let value = serde_json::json!({"id":1,"operation":"snapshot"});
    let mut bytes = Vec::new();
    write_native(&mut bytes, &value).unwrap();
    let n = u32::from_ne_bytes(bytes[..4].try_into().unwrap()) as usize;
    assert_eq!(n, bytes.len() - 4);
    assert_eq!(read_native(&mut &bytes[..]).unwrap(), value);
    let huge = (1024u32 * 1024 + 1).to_ne_bytes();
    assert!(
        read_native(&mut &huge[..])
            .unwrap_err()
            .to_string()
            .contains("limit")
    );
}

#[tokio::test]
async fn real_native_host_relays_only_framed_fixed_operations() {
    use rustune::{backend::firefox, config::Paths};
    use std::process::{Command, Stdio};
    let d = tempfile::tempdir().unwrap();
    let paths = Paths {
        config: d.path().join("config/rustune"),
        data: d.path().join("data/rustune"),
        cache: d.path().join("cache/rustune"),
        runtime: d.path().join("run/rustune"),
    };
    paths.init().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rustune"))
        .args(["native-host", "/test/manifest.json", firefox::ID])
        .env("XDG_CONFIG_HOME", d.path().join("config"))
        .env("XDG_DATA_HOME", d.path().join("data"))
        .env("XDG_CACHE_HOME", d.path().join("cache"))
        .env("XDG_RUNTIME_DIR", d.path().join("run"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = child.stdout.take().unwrap();
    let responder = std::thread::spawn(move || {
        let message = read_native(&mut output).unwrap();
        assert_eq!(message["operation"], "snapshot");
        write_native(
            &mut input,
            &serde_json::json!({"id":message["id"],"ok":true,"result":{"authorized":false}}),
        )
        .unwrap();
    });
    for _ in 0..100 {
        if paths.runtime.join("firefox.sock").exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let result = firefox::request(&paths, "snapshot", serde_json::json!({})).await;
    responder.join().unwrap();
    let _ = child.kill();
    let _ = child.wait();
    let (pid, value) = result.unwrap();
    assert!(pid > 0);
    assert_eq!(value["authorized"], false);
}
