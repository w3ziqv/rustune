use rustune::config::Config;

#[test]
fn help_has_no_removed_browser_setup_or_authorization_options() {
    let binary = env!("CARGO_BIN_EXE_rustune");
    for args in [
        vec!["--help"],
        vec!["login", "--help"],
        vec!["logout", "--help"],
    ] {
        let output = std::process::Command::new(binary)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        let help = String::from_utf8(output.stdout).unwrap().to_lowercase();
        for removed in ["firefox", "--browser", "add-on", "chromium"] {
            assert!(
                !help.contains(removed),
                "removed option {removed} remains in help"
            );
        }
    }
    for args in [
        vec!["firefox-setup", "--help"],
        vec!["native-host", "--help"],
        vec!["login", "--browser"],
        vec!["logout", "--browser"],
    ] {
        let output = std::process::Command::new(binary)
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success(), "retired command still accepted");
    }
}

#[test]
fn old_browser_config_migrates_without_losing_preferences() {
    let c = Config::parse("backend = 'firefox'\nbrowser = '/bin/false'\nheadless = true\nbridge_port = 8973\ntheme = 'nord'\nvolume = 0.4\n").unwrap();
    assert_eq!(c.theme, "nord");
    assert_eq!(c.volume, 0.4);
    let saved = toml::to_string(&c).unwrap();
    for removed in ["firefox", "browser", "bridge_port", "headless"] {
        assert!(
            !saved.contains(removed),
            "removed setting {removed} would be saved again"
        );
    }
    assert!(Config::parse("unrecognized_setting = true").is_err());
}

#[test]
fn migration_rewrites_only_config_and_keeps_the_token_and_queue() {
    use rustune::config::{Paths, private_write};
    let d = tempfile::tempdir().unwrap();
    let p = Paths {
        config: d.path().join("config"),
        data: d.path().join("data"),
        cache: d.path().join("cache"),
        runtime: d.path().join("run"),
    };
    p.init().unwrap();
    private_write(
        &p.config.join("config.toml"),
        b"backend = 'firefox'\nheadless = true\ntheme = 'nord'\n",
    )
    .unwrap();
    private_write(&p.config.join("music-user-token"), b"artificial-token").unwrap();
    private_write(&p.data.join("state.json"), b"saved-queue-fixture").unwrap();
    assert_eq!(Config::load(&p).unwrap().theme, "nord");
    assert!(
        !std::fs::read_to_string(p.config.join("config.toml"))
            .unwrap()
            .contains("firefox")
    );
    assert_eq!(
        std::fs::read(p.config.join("music-user-token")).unwrap(),
        b"artificial-token"
    );
    assert_eq!(
        std::fs::read(p.data.join("state.json")).unwrap(),
        b"saved-queue-fixture"
    );
}

#[test]
fn browser_authorization_is_not_an_ipc_operation() {
    for command in ["login", "logout"] {
        let request = serde_json::json!({"command":command});
        assert!(serde_json::from_value::<rustune::ipc::Request>(request).is_err());
    }
}
