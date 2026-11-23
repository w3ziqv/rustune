use super::*;
use crate::{config::PerformanceMode, ipc, service};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UnixStream},
    time::timeout,
};

const WAIT: Duration = Duration::from_secs(2);
const QUIET: Duration = Duration::from_millis(65);

async fn incoming(listener: &TcpListener) -> (TcpStream, String) {
    timeout(WAIT, async {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(socket.read_u8().await.unwrap());
            assert!(header.len() < 8192);
        }
        (socket, String::from_utf8(header).unwrap())
    })
    .await
    .expect("HTTP request reached the fixture")
}

async fn reply(mut socket: TcpStream, code: &str, body: Value) {
    let body = body.to_string();
    socket
        .write_all(
            format!(
                "HTTP/1.1 {code}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn apple_reads_coalesce_and_run_two_independent_queries() {
    let dir = tempfile::tempdir().unwrap();
    let paths = playlist_tests::paths(dir.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let apple = apple::AppleCatalog::with_origin(
        paths,
        format!("http://{}", listener.local_addr().unwrap()),
    )
    .unwrap();
    let request = |term| {
        let apple = apple.clone();
        tokio::spawn(async move {
            apple
                .browse(
                    "search:songs",
                    term,
                    Some("pl:0"),
                    "PL",
                    PerformanceMode::Balanced.limits(),
                )
                .await
        })
    };
    let leader = request("one");
    let (mut first, header) = incoming(&listener).await;
    assert!(header.contains("term=one"));
    let follower = request("one");
    let second = request("two");
    let (other, header) = incoming(&listener).await;
    assert!(header.contains("term=two"));
    let third = request("three");
    assert!(timeout(QUIET, listener.accept()).await.is_err());
    leader.abort();
    assert!(leader.await.unwrap_err().is_cancelled());
    assert!(
        timeout(QUIET, first.read_u8()).await.is_err(),
        "remaining waiter still owns HTTP"
    );
    follower.abort();
    assert!(follower.await.unwrap_err().is_cancelled());
    assert_eq!(
        timeout(WAIT, first.read(&mut [0])).await.unwrap().unwrap(),
        0
    );
    let (last, header) = incoming(&listener).await;
    assert!(header.contains("term=three"));
    reply(other, "200 OK", json!({"results":{}})).await;
    reply(last, "200 OK", json!({"results":{}})).await;
    assert!(second.await.unwrap().is_ok());
    assert!(third.await.unwrap().is_ok());
}

#[tokio::test]
async fn itunes_ipc_waiters_share_http_and_last_disconnect_allows_retry() {
    let dir = tempfile::tempdir().unwrap();
    let paths = playlist_tests::paths(dir.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let catalog = Catalog::with_origin(&format!("http://{}", listener.local_addr().unwrap()));
    let daemon = tokio::spawn(service::run_with(
        crate::config::Config::default(),
        paths.clone(),
        Arc::new(crate::backend::testing::Controlled::default()),
        Some(Arc::new(catalog)),
        Some("unix:path=/nonexistent".into()),
    ));
    timeout(WAIT, async {
        while !paths.socket().exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();

    let browse = ipc::Request::Browse {
        kind: "search:songs".into(),
        term: "shared".into(),
        next: None,
    };
    let connect = || async {
        let mut client = UnixStream::connect(paths.socket()).await.unwrap();
        ipc::write_frame(&mut client, &browse).await.unwrap();
        client
    };
    let leader = connect().await;
    let (mut upstream, _) = incoming(&listener).await;
    let follower = connect().await;
    assert!(timeout(QUIET, listener.accept()).await.is_err());
    drop(leader);
    assert!(timeout(QUIET, upstream.read_u8()).await.is_err());
    drop(follower);
    assert_eq!(
        timeout(WAIT, upstream.read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );

    let mut client = connect().await;
    let (upstream, _) = incoming(&listener).await;
    reply(upstream, "503 Service Unavailable", json!({})).await;
    let read = |bytes: &[u8]| {
        serde_json::from_slice::<service::Reply>(bytes)
            .unwrap()
            .value()
    };
    assert!(
        read(&ipc::read_frame(&mut client).await.unwrap())
            .unwrap_err()
            .to_string()
            .contains("503")
    );
    let mut retry = connect().await;
    let (upstream, _) = incoming(&listener).await;
    reply(upstream, "200 OK", json!({"results":[]})).await;
    assert_eq!(
        read(&ipc::read_frame(&mut retry).await.unwrap()).unwrap()["items"],
        json!([])
    );
    ipc::request(&paths, &ipc::Request::Shutdown).await.unwrap();
    daemon.await.unwrap().unwrap();
}

#[tokio::test]
async fn same_storefront_account_rotation_rejects_old_cursor_and_late_page() {
    let dir = tempfile::tempdir().unwrap();
    let paths = playlist_tests::paths(dir.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut catalog = Catalog::with_origin("http://127.0.0.1:1");
    catalog.apple = Some(
        apple::AppleCatalog::with_origin(
            paths.clone(),
            format!("http://{}", listener.local_addr().unwrap()),
        )
        .unwrap(),
    );
    let limits = PerformanceMode::Balanced.limits();
    let request = |cursor: Option<String>| {
        let catalog = catalog.clone();
        tokio::spawn(async move {
            catalog
                .browse("library", "", cursor.as_deref(), "PL", limits)
                .await
        })
    };
    let initial = request(None);
    let (region, _) = incoming(&listener).await;
    reply(region, "200 OK", json!({"data":[{"id":"pl"}]})).await;
    let (page, _) = incoming(&listener).await;
    reply(page, "200 OK", json!({"data":[], "next":"next"})).await;
    let cursor = initial.await.unwrap().unwrap().next.unwrap();
    let late = request(Some(cursor.clone()));
    let (pending, header) = incoming(&listener).await;
    assert!(header.contains("offset=25"));
    crate::auth::save_user_token(
        &paths,
        &crate::auth::SecretToken::parse("next-user").unwrap(),
    )
    .unwrap();

    let stale = request(Some(cursor));
    assert!(
        timeout(WAIT, stale)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("expired")
    );
    let fresh = request(None);
    let (region, header) = incoming(&listener).await;
    assert!(
        header
            .to_lowercase()
            .contains("music-user-token: next-user")
    );
    reply(region, "200 OK", json!({"data":[{"id":"pl"}]})).await;
    let (page, header) = incoming(&listener).await;
    assert!(header.contains("offset=0"));
    reply(pending, "200 OK", json!({"data":[]})).await;
    assert!(
        late.await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("credentials changed")
    );
    assert_eq!(catalog.cache_entries(), 0);
    reply(page, "200 OK", json!({"data":[]})).await;
    assert!(fresh.await.unwrap().is_ok());
    assert_eq!(catalog.cache_entries(), 1);
}

#[tokio::test]
async fn late_negative_storefront_cannot_overwrite_the_new_account_region() {
    let dir = tempfile::tempdir().unwrap();
    let paths = playlist_tests::paths(dir.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let catalog = Catalog::with_music_origin(
        paths.clone(),
        format!("http://{}", listener.local_addr().unwrap()),
    );
    let browse = |kind| {
        let catalog = catalog.clone();
        tokio::spawn(async move {
            catalog
                .browse(kind, "", None, "PL", PerformanceMode::Balanced.limits())
                .await
        })
    };
    let old = browse("library");
    let (old_region, request) = incoming(&listener).await;
    assert!(request.starts_with("GET /v1/me/storefront"));
    crate::auth::save_user_token(
        &paths,
        &crate::auth::SecretToken::parse("new-account").unwrap(),
    )
    .unwrap();
    let fresh = browse("library");
    let (new_region, request) = incoming(&listener).await;
    assert!(request.starts_with("GET /v1/me/storefront"));
    reply(new_region, "200 OK", json!({"data":[{"id":"pl"}]})).await;
    let (page, _) = incoming(&listener).await;
    reply(page, "200 OK", json!({"data":[]})).await;
    assert!(fresh.await.unwrap().is_ok());
    reply(old_region, "401 Unauthorized", json!({})).await;
    assert!(
        old.await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("credentials changed")
    );

    let playlists = browse("playlists");
    let (page, request) = incoming(&listener).await;
    assert!(request.starts_with("GET /v1/me/library/playlists?"));
    reply(page, "200 OK", json!({"data":[]})).await;
    assert!(playlists.await.unwrap().is_ok());
}

#[tokio::test]
async fn last_private_page_clamps_size_and_has_no_invalid_next_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let paths = playlist_tests::paths(dir.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let catalog =
        Catalog::with_music_origin(paths, format!("http://{}", listener.local_addr().unwrap()));
    let cursor = playlist::page_cursor("pl", catalog.account_generation().unwrap(), 9999);
    let browse = tokio::spawn(async move {
        catalog
            .browse(
                "library",
                "",
                Some(&cursor),
                "pl",
                PerformanceMode::Balanced.limits(),
            )
            .await
            .unwrap()
    });
    let (region, _) = incoming(&listener).await;
    reply(region, "200 OK", json!({"data":[{"id":"pl"}]})).await;
    let (body, request) = incoming(&listener).await;
    assert!(request.contains("limit=1&offset=9999"));
    reply(
        body,
        "200 OK",
        json!({"data":[], "next":"upstream-has-more"}),
    )
    .await;
    let page = browse.await.unwrap();
    assert!(page.next.is_none());
    assert!(page.truncated);
}
