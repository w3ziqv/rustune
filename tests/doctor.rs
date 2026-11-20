use serde_json::Value;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command, time::Duration};
use tempfile::TempDir;

struct Installation {
    binaries: TempDir,
    state: TempDir,
}
impl Installation {
    fn new() -> Self {
        let binary = Path::new(env!("CARGO_BIN_EXE_rustune"));
        let binaries = tempfile::tempdir_in(binary.parent().unwrap()).unwrap();
        fs::hard_link(binary, binaries.path().join("rustune")).unwrap();
        Self {
            binaries,
            state: tempfile::tempdir().unwrap(),
        }
    }

    fn command(&self) -> Command {
        let root = self.state.path();
        let mut command = Command::new(self.binaries.path().join("rustune"));
        command
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("XDG_RUNTIME_DIR", root.join("run"))
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", root.join("missing-bus").display()),
            );
        command
    }

    fn install_helper(&self, contents: impl AsRef<[u8]>) {
        let path = self.binaries.path().join("rustune-audio");
        fs::write(&path, contents).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn install_helper_protocol(&self, version: u32) {
        let hello = r#"{"command":"hello","args":{"version":2}}"#;
        let reply = format!(r#"{{"event":"hello","data":{{"version":{version}}}}}"#);
        self.install_helper(format!(
            r#"#!/bin/sh
hello=$(dd bs=1 skip=4 count={} 2>/dev/null)
if [ "$hello" = '{}' ]; then
    printf '\000\000\000\{:03o}%s' '{}'
fi
"#,
            hello.len(),
            hello,
            reply.len(),
            reply
        ));
    }

    fn json(&self) -> Value {
        let output = self.command().args(["doctor", "--json"]).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

#[test]
fn json_distinguishes_missing_prerequisites_without_starting_service() {
    let install = Installation::new();
    let report = install.json();
    assert_eq!(report["token"], "missing");
    assert_eq!(report["cdm_present"], false);
    assert_eq!(report["audio_helper"]["state"], "missing");
    assert_eq!(report["service"]["state"], "not_running");
    assert_eq!(report["session_dbus"], false);
    assert_eq!(report["source_quality"], "Source quality unknown");
    assert!(!install.state.path().join("run").exists());
    assert!(!install.state.path().join("cache").exists());
}

#[test]
fn doctor_checks_private_token_and_cdm_independently_of_an_unresponsive_helper() {
    let install = Installation::new();
    let root = install.state.path();
    let token_path = root.join("config/rustune/music-user-token");
    rustune::config::private_write(&token_path, b"private-diagnostic-fixture").unwrap();
    let cdm = root.join("cache/rustune/drm/libwidevinecdm.so");
    fs::create_dir_all(cdm.parent().unwrap()).unwrap();
    fs::write(cdm, b"CDM file presence fixture").unwrap();
    install.install_helper("#!/bin/sh\nwhile IFS= read -r line; do :; done\n");

    let start = std::time::Instant::now();
    let report = install.json();
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(report["token"], "saved_locally");
    assert_eq!(report["cdm_present"], true);
    assert_eq!(report["audio_helper"]["state"], "unresponsive");
    assert_eq!(report["source_quality"], "Source quality unknown");
    assert!(!report.to_string().contains("private-diagnostic-fixture"));
    let text = install.command().arg("doctor").output().unwrap();
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("saved locally"));
    assert!(text.contains("account acceptance unverified"));
    assert!(text.contains("Reinstall rustune and rustune-audio"));
    assert!(!text.contains("private-diagnostic-fixture"));
}

#[test]
fn doctor_leaves_existing_configuration_untouched() {
    let install = Installation::new();
    let config = install.state.path().join("config/rustune/config.toml");
    let old = b"browser = '/bin/false'\ntheme = 'nord'\n";
    rustune::config::private_write(&config, old).unwrap();
    install.json();
    assert_eq!(fs::read(config).unwrap(), old);
}

#[test]
fn doctor_reads_the_helper_wire_version_without_sending_playback_commands() {
    for (version, state) in [(1, "incompatible"), (2, "compatible")] {
        let install = Installation::new();
        install.install_helper_protocol(version);
        let report = install.json();
        assert_eq!(report["audio_helper"]["state"], state);
        assert_eq!(report["audio_helper"]["version"], version);
        assert_eq!(report["expected_helper_version"], 2);
        assert_eq!(report["service"]["state"], "not_running");
        assert_eq!(report["source_quality"], "Source quality unknown");
    }
}

#[tokio::test]
async fn doctor_recognizes_a_running_daemon_without_starting_playback() {
    use std::process::Stdio;
    use tokio::{net::UnixStream, process::Command, time};

    let install = Installation::new();
    let socket = install.state.path().join("run/rustune/service.sock");
    let mut daemon = Command::from(install.command())
        .arg("daemon")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    time::timeout(Duration::from_secs(5), async {
        while UnixStream::connect(&socket).await.is_err() {
            assert!(daemon.try_wait().unwrap().is_none(), "daemon exited early");
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("daemon listening");

    let report = install.json();
    assert_eq!(report["service"]["state"], "running");
    assert_eq!(report["service"]["pid"], daemon.id().unwrap());
    assert_eq!(report["service"]["helper_protocol"], 2);
    assert_eq!(report["audio_helper"]["state"], "missing");
    assert_eq!(report["source_quality"], "Source quality unknown");
    assert!(
        !report["next_steps"]
            .to_string()
            .contains("Restart the Rustune service")
    );

    assert!(
        Command::from(install.command())
            .args(["ctl", "shutdown"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .unwrap()
            .success()
    );
    assert!(
        time::timeout(Duration::from_secs(5), daemon.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
}

#[tokio::test]
async fn doctor_flags_old_daemon_even_when_the_clients_sibling_helper_matches() {
    use rustune::{
        ipc::{self, Request},
        service::Reply,
    };
    use serde_json::json;
    use tokio::{net::UnixListener, process::Command, time};

    for (protocol, protocol_label) in [
        (None, "helper protocol unreported"),
        (Some(1), "helper protocol 1, expected 2"),
    ] {
        let install = Installation::new();
        install.install_helper_protocol(2);
        let runtime = install.state.path().join("run/rustune");
        fs::create_dir_all(&runtime).unwrap();
        let listener = UnixListener::bind(runtime.join("service.sock")).unwrap();
        let mut payload = json!({
            "pid": 7,
            "playback": {},
            "queue_len": 0,
            "repeat": "off",
            "shuffle": false,
            "profile": "balanced",
            "backend": "native",
            "browser_shared": false,
            "uptime_secs": 0,
            "cache_entries": 0,
            "message": "",
        });
        if let Some(version) = protocol {
            payload["helper_protocol"] = json!(version);
        }
        let serve = async {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let frame = ipc::read_frame(&mut stream).await.unwrap();
                let request: Request = serde_json::from_slice(&frame).unwrap();
                assert!(matches!(request, Request::Status));
                ipc::write_frame(&mut stream, &Reply::result(Ok(payload.clone())))
                    .await
                    .unwrap();
            }
        };
        let run = async {
            let json = Command::from(install.command())
                .args(["doctor", "--json"])
                .output()
                .await
                .unwrap();
            let text = Command::from(install.command())
                .arg("doctor")
                .output()
                .await
                .unwrap();
            (json, text)
        };
        let ((), (json, text)) =
            time::timeout(Duration::from_secs(5), async { tokio::join!(serve, run) })
                .await
                .unwrap();
        assert!(json.status.success() && text.status.success());
        let report: Value = serde_json::from_slice(&json.stdout).unwrap();
        assert_eq!(report["audio_helper"]["state"], "compatible");
        assert_eq!(report["audio_helper_scope"], "frontend_sibling");
        assert_eq!(report["service"]["state"], "running");
        assert_eq!(
            report["service"].get("helper_protocol"),
            Some(&json!(protocol))
        );
        assert!(
            report["next_steps"]
                .to_string()
                .contains("Restart the Rustune service")
        );
        let text = String::from_utf8(text.stdout).unwrap();
        assert!(text.contains("sibling of this rustune"));
        assert!(text.contains(protocol_label));
        assert!(text.contains("Restart the Rustune service"));
    }
}
