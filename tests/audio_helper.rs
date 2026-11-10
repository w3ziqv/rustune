use rustune::backend::worker::audio_executable;
use std::{fs, os::unix::fs::PermissionsExt};
#[test]
fn helper_must_be_an_executable_sibling_and_missing_installation_is_explained() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("rustune");
    let helper = dir.path().join("rustune-audio");
    assert!(
        audio_executable(&main)
            .unwrap_err()
            .to_string()
            .contains("audio helper")
    );
    fs::write(&helper, b"fixture, never executed").unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(audio_executable(&main).is_err());
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(audio_executable(&main).unwrap(), helper);
}
