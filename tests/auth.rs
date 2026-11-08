use rustune::{
    auth::{SecretToken, TokenInput, save_user_token},
    config::Paths,
};
use std::os::unix::fs::PermissionsExt;

fn paths(d: &tempfile::TempDir) -> Paths {
    Paths {
        config: d.path().join("config"),
        data: d.path().join("data"),
        cache: d.path().join("cache"),
        runtime: d.path().join("run"),
    }
}

#[test]
fn token_storage_is_private_and_invalid_input_does_not_replace_it() {
    let d = tempfile::tempdir().unwrap();
    let p = paths(&d);
    p.init().unwrap();
    let token = SecretToken::parse("  opaque-test-user-token\n").unwrap();
    assert!(!format!("{token:?}").contains("opaque-test-user-token"));
    save_user_token(&p, &token).unwrap();
    let file = p.config.join("music-user-token");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "opaque-test-user-token"
    );
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    for invalid in ["", "  ", "bad token", "bad\u{1b}token"] {
        assert!(SecretToken::parse(invalid).is_err());
    }
    assert_eq!(
        std::fs::read_to_string(file).unwrap(),
        "opaque-test-user-token"
    );
}

#[test]
fn oversized_paste_cannot_save_a_truncated_token() {
    let mut input = TokenInput::default();
    input.append("first-part");
    input.append(&"a".repeat(16 * 1024));
    assert!(input.finish().is_err());
    let mut input = TokenInput::default();
    input.append("discard");
    input.clear();
    input.append("replacement");
    assert!(input.finish().is_ok());
}

#[test]
fn cli_login_prompts_without_echo_and_cancellation_preserves_the_token() {
    let output = std::process::Command::new("python3")
        .arg("scripts/verify-login.py")
        .arg(env!("CARGO_BIN_EXE_rustune"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
