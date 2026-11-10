use rustune::{config::Config, ipc::Request};
#[test]
fn equalizer_config_persists_and_rejects_invalid_gains() {
    let valid = "[equalizer]\nenabled=true\npreamp_db=-2.0\nbands_db=[0,0,0,0,0,-6,0,0,0,0]\n";
    let config = Config::parse(valid).unwrap();
    let saved = toml::to_string(&config).unwrap();
    assert!(saved.contains("preamp_db = -2.0"));
    assert!(Config::parse(&valid.replace("-6", "18")).is_err());
    assert!(Config::parse(&valid.replace("-2.0", "nan")).is_err());
    let request:Request=serde_json::from_value(serde_json::json!({"command":"equalizer","args":{"enabled":true,"preamp_db":-2.0,"bands_db":[0,0,0,0,0,-6,0,0,0,0]}})).unwrap();
    assert_eq!(
        serde_json::to_value(request).unwrap()["args"]["bands_db"][5],
        -6.0
    );
}
