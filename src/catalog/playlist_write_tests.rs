use super::*;
use crate::catalog::{playlist::PlaylistRef, playlist_write::WriteAction};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub(crate) async fn server(
    responses: Vec<(u16, Option<Value>)>,
) -> (
    tempfile::TempDir,
    Catalog,
    tokio::task::JoinHandle<Vec<Value>>,
) {
    server_gated(responses, None).await
}
pub(crate) async fn server_gated(
    responses: Vec<(u16, Option<Value>)>,
    gate: Option<(
        std::sync::Arc<tokio::sync::Notify>,
        std::sync::Arc<tokio::sync::Notify>,
    )>,
) -> (
    tempfile::TempDir,
    Catalog,
    tokio::task::JoinHandle<Vec<Value>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let paths = super::playlist_tests::paths(dir.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (code, body) in responses {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    break end + 4;
                }
                assert!(bytes.len() <= 16384);
            };
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains("music-user-token: fixture-user")
            );
            let length = headers
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|n| n.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            assert!(length <= 128 * 1024);
            let path = headers.lines().next().unwrap().to_owned();
            while bytes.len() - header_end < length {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
            }
            let payload = if length > 0 {
                serde_json::from_slice::<Value>(&bytes[header_end..header_end + length]).unwrap()
            } else {
                Value::Null
            };
            requests.push(json!({"request":path,"body":payload}));
            if path.starts_with("POST ")
                && let Some((reached, release)) = &gate
            {
                reached.notify_one();
                release.notified().await;
            }
            if let Some(body) = body {
                let body = if code == 204 {
                    String::new()
                } else {
                    body.to_string()
                };
                socket.write_all(format!("HTTP/1.1 {code} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        }
        requests
    });
    let mut c = Catalog::with_origin("http://127.0.0.1:1");
    c.apple = Some(apple::AppleCatalog::with_origin(paths, origin).unwrap());
    (dir, c, task)
}
fn region() -> Value {
    json!({"data":[{"id":"pl"}]})
}
#[tokio::test]
async fn create_one_post_has_private_attributes_and_all_duplicate_tracks() {
    let (_dir, c, http) = server(vec![
        (200, Some(region())),
        (
            201,
            Some(json!({"data":[{"id":"p.created","type":"library-playlists"}]})),
        ),
    ])
    .await;
    let result = c
        .write_playlist(
            &WriteAction::Create {
                name: "Fixture".into(),
                description: "Synthetic".into(),
            },
            &["1".into(), "1".into(), "2".into()],
            c.playlist_account().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.tracks, 3);
    let requests = http.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1]["request"]
            .as_str()
            .unwrap()
            .starts_with("POST /v1/me/library/playlists ")
    );
    assert_eq!(
        requests[1]["body"]["relationships"]["tracks"]["data"][1]["id"],
        "1"
    );
    assert_eq!(requests[1]["body"]["attributes"]["isPublic"], false);
}
#[tokio::test]
async fn fresh_readonly_check_denies_post_even_when_client_claims_editable() {
    let(_dir,c,http)=server(vec![(200,Some(region())),(200,Some(json!({"data":[{"id":"p.fixture","type":"library-playlists","attributes":{"canEdit":false}}]})))]).await;
    let error = c
        .write_playlist(
            &WriteAction::Add {
                playlist: PlaylistRef {
                    id: "p.fixture".into(),
                    storefront: "pl".into(),
                    title: "Fixture".into(),
                    can_edit: true,
                },
            },
            &["1".into()],
            c.playlist_account().unwrap(),
        )
        .await
        .unwrap_err();
    assert!(!error.uncertain);
    assert!(error.message.contains("editable"));
    assert_eq!(http.await.unwrap().len(), 2);
}
#[tokio::test]
async fn append_post_uses_catalog_ids_and_204_confirms_success() {
    let(_dir,c,http)=server(vec![(200,Some(region())),(200,Some(json!({"data":[{"id":"p.fixture","type":"library-playlists","attributes":{"canEdit":true}}]}))),(204,Some(Value::Null))]).await;
    let result = c
        .write_playlist(
            &WriteAction::Add {
                playlist: PlaylistRef {
                    id: "p.fixture".into(),
                    storefront: "pl".into(),
                    title: "Fixture".into(),
                    can_edit: true,
                },
            },
            &["1".into()],
            c.playlist_account().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.tracks, 1);
    let requests = http.await.unwrap();
    assert!(
        requests[2]["request"]
            .as_str()
            .unwrap()
            .contains("/p.fixture/tracks")
    );
    assert_eq!(
        requests[2]["body"],
        json!({"data":[{"id":"1","type":"songs"}]})
    );
}
#[tokio::test]
async fn rejected_and_ambiguous_post_failures_are_distinct_and_hide_backend_details() {
    for (code, body, uncertain) in [
        (
            403,
            Some(json!({"errors":[{"detail":"sensitive fixture body"}]})),
            false,
        ),
        (401, Some(json!({})), false),
        (429, Some(json!({})), false),
        (408, Some(json!({})), true),
        (500, Some(json!({})), true),
        (201, Some(json!({"data":[]})), true),
        (200, None, true),
    ] {
        let (_dir, c, http) = server(vec![(200, Some(region())), (code, body)]).await;
        let error = c
            .write_playlist(
                &WriteAction::Create {
                    name: "Fixture".into(),
                    description: "".into(),
                },
                &[],
                c.playlist_account().unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.uncertain, uncertain);
        assert!(!error.message.contains("sensitive"));
        assert_eq!(http.await.unwrap().len(), 2);
    }
}
