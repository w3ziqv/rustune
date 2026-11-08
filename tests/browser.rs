use rustune::{
    backend::{cdp::Cdp, musickit::browser_binary},
    config::Config,
};
#[tokio::test]
#[ignore = "launches real Chromium; run explicitly on Linux"]
async fn chromium_pipe_round_trip() {
    let p = tempfile::tempdir().unwrap();
    let mut c = Cdp::launch(
        &browser_binary(&Config::default()).expect("Chromium installed"),
        p.path(),
        true,
    )
    .await
    .unwrap();
    c.attach("about:blank").await.unwrap();
    assert_eq!(
        c.eval("({answer: 6*7, text:'Zażółć gęślą jaźń'})")
            .await
            .unwrap()["answer"],
        42
    );
    assert!(
        !c.eval("navigator.userAgent")
            .await
            .unwrap()
            .as_str()
            .unwrap()
            .is_empty()
    );
    let pid = c.pid();
    c.close().await;
    drop(c);
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}

#[tokio::test]
#[ignore = "loads official MusicKit from Apple's CDN; no account or live playback"]
async fn official_musickit_host_loads_and_rejects_invalid_developer_token() {
    use rustune::backend::musickit::serve_document;
    let profile = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let document = serve_document(port).await.unwrap();
    let mut c = Cdp::launch(
        &browser_binary(&Config::default()).unwrap(),
        profile.path(),
        true,
    )
    .await
    .unwrap();
    c.attach(&format!("http://127.0.0.1:{port}/"))
        .await
        .unwrap();
    let mut loaded = false;
    for _ in 0..100 {
        if c.eval("typeof MusicKit !== 'undefined'").await.unwrap() == true {
            loaded = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        loaded,
        "official MusicKit CDN must load without disabling browser security"
    );
    // Deliberately invalid test credential. This only exercises library initialization,
    // not authentication or streaming; there is no mock media or preview playback.
    let config = serde_json::json!({"developerToken":"eyJhbGciOiJFUzI1NiIsImtpZCI6IlRFU1RPTkxZIn0.eyJpc3MiOiJURVNUT05MWSIsImV4cCI6MX0.invalid","app":{"name":"Rustune transport test","build":"test"}});
    let script = include_str!("../src/backend/musickit.js").replace(
        "__RUSTUNE_INSTANCE__",
        &format!("await MusicKit.configure({config})"),
    );
    let diagnostic = format!(
        "(async()=>{{try{{return await {script}}}catch(e){{return {{test_error:e.name,message:e.message}}}}}})()"
    );
    let rejection = c.eval(&diagnostic).await.unwrap();
    assert_eq!(rejection["test_error"], "Error");
    assert!(rejection["message"].as_str().unwrap().contains("token"));
    c.close().await;
    document.abort();
}

#[tokio::test]
#[ignore = "loads Apple's official web player; no credentials or playback"]
async fn official_web_player_exposes_a_musickit_controller() {
    let profile = tempfile::tempdir().unwrap();
    let mut c = Cdp::launch(
        &browser_binary(&Config::default()).unwrap(),
        profile.path(),
        true,
    )
    .await
    .unwrap();
    c.attach("https://music.apple.com/").await.unwrap();
    let mut ready = false;
    for _ in 0..200 {
        if c.eval("typeof MusicKit !== 'undefined' && !!MusicKit.getInstance()")
            .await
            .unwrap()
            == true
        {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(ready, "official web player controller must be available");
    assert_eq!(
        c.eval("MusicKit.getInstance().isAuthorized").await.unwrap(),
        false
    );
    // No token access: only public controller shape and unauthorized state.
    assert_eq!(
        c.eval("typeof MusicKit.getInstance().setQueue")
            .await
            .unwrap(),
        "function"
    );
    let script = include_str!("../src/backend/musickit.js")
        .replace("__RUSTUNE_INSTANCE__", "MusicKit.getInstance()");

    assert_eq!(c.eval(&script).await.unwrap(), true);
    assert_eq!(
        c.eval("window.rustune.dispatch('play', {})").await.unwrap()["error"],
        "unauthorized"
    );
    let result=c.eval("window.rustune.dispatch('browse', {kind:'search:songs',term:'Bach',limit:5,next:null})").await.unwrap();
    assert!(
        result["items"].as_array().is_some_and(|a| !a.is_empty()),
        "real catalog search should return songs: {result}"
    );
    c.close().await;
}
