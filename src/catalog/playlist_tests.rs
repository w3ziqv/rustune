use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub(crate) fn paths(root: &std::path::Path) -> crate::config::Paths {
    let p = crate::config::Paths {
        config: root.join("c"),
        data: root.join("d"),
        cache: root.join("a"),
        runtime: root.join("r"),
    };
    p.init().unwrap();
    let app = format!(
        "{}.{}.fixture",
        URL_SAFE_NO_PAD.encode(r#"{"alg":"ES256","kid":"fixture"}"#),
        URL_SAFE_NO_PAD.encode(r#"{"iss":"fixture","exp":4102444800}"#)
    );
    crate::config::private_write(&p.cache.join("apple-app-token"), app.as_bytes()).unwrap();
    crate::config::private_write(&p.config.join("music-user-token"), b"fixture-user").unwrap();
    p
}
pub(crate) async fn fixture(
    bodies: Vec<Value>,
) -> (
    tempfile::TempDir,
    Catalog,
    tokio::task::JoinHandle<Vec<String>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut endpoints = Vec::new();
        for body in bodies {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = vec![0; 8192];
            let mut n = 0;
            while !bytes[..n].windows(4).any(|s| s == b"\r\n\r\n") {
                n += stream.read(&mut bytes[n..]).await.unwrap();
            }
            let raw = String::from_utf8(bytes[..n].to_vec()).unwrap();
            assert!(
                raw.to_ascii_lowercase()
                    .contains("music-user-token: fixture-user")
            );
            endpoints.push(raw.lines().next().unwrap().to_owned());
            let body = body.to_string();
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
        endpoints
    });
    let mut c = Catalog::with_origin("http://127.0.0.1:1");
    c.apple = Some(apple::AppleCatalog::with_origin(paths, origin).unwrap());
    (dir, c, server)
}
#[tokio::test]
async fn playlist_list_maps_private_ids_and_can_edit_without_itunes_fallback() {
    let(_dir,c,server)=fixture(vec![json!({"data":[{"id":"pl"}]}),json!({"data":[{"id":"p.editable","type":"library-playlists","attributes":{"name":"Fixture playlist","canEdit":true}},{"id":"p.readonly","type":"library-playlists","attributes":{"name":"Readonly","canEdit":false}}]})]).await;
    let page = c
        .browse(
            "playlists",
            "",
            None,
            "DE",
            crate::config::PerformanceMode::Balanced.limits(),
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].kind, "library-playlists");
    assert_eq!(page.items[0].can_edit, Some(true));
    assert_eq!(page.items[1].can_edit, Some(false));
    assert_eq!(page.storefront.as_deref(), Some("pl"));
    assert!(server.await.unwrap()[1].starts_with("GET /v1/me/library/playlists?"));
}
#[tokio::test]
async fn whole_playlist_keeps_225_source_positions_and_duplicates_across_pages() {
    let mut bodies = vec![json!({"data":[{"id":"pl"}]})];
    for page in 0..9 {
        let data=(page*25..(page+1)*25).map(|i|json!({"id":format!("i.{i}"),"type":"library-songs","attributes":{"name":format!("Song {i}"),"discNumber":9-i/25,"trackNumber":25-i%25,"playParams":{"catalogId":if i==26{"26".to_string()}else{(i+1).to_string()}}}})).collect::<Vec<_>>();
        let mut body = json!({"data":data,"meta":{"total":225}});
        if page < 8 {
            body["next"] = json!("https://untrusted.test/not-followed");
        }
        bodies.push(body);
    }
    let (_dir, c, server) = fixture(bodies).await;
    let playlist = playlist::PlaylistRef {
        id: "p.fixture".into(),
        storefront: "pl".into(),
        title: "Synthetic playlist".into(),
        can_edit: true,
    };
    let prepared = c
        .playlist(
            &playlist,
            crate::config::PerformanceMode::Balanced.limits(),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(prepared.tracks.len(), 225);
    assert_eq!(prepared.tracks[0].id, "1");
    assert_eq!(prepared.tracks[25].id, "26");
    assert_eq!(prepared.tracks[26].id, "26");
    assert_eq!(prepared.tracks[26].source_index, Some(26));
    assert_eq!(prepared.tracks[224].id, "225");
    assert_eq!(prepared.context.playlist_id.as_deref(), Some("p.fixture"));
    assert_eq!(server.await.unwrap().len(), 10);
}
#[tokio::test]
async fn library_song_can_map_catalog_relationship_without_play_params_catalog_id() {
    let t=apple::map_resource(&json!({"id":"i.local","type":"library-songs","attributes":{"name":"Fixture"},"relationships":{"catalog":{"data":[{"id":"123","type":"songs"}]}}}),"library-songs").unwrap();
    assert_eq!(t.id, "123");
}
#[tokio::test]
async fn playlist_refresh_reads_current_account_metadata_instead_of_stale_cache() {
    let (_dir,c,http)=fixture(vec![json!({"data":[{"id":"pl"}]}),json!({"data":[{"id":"p.first","type":"library-playlists","attributes":{"name":"First","canEdit":true}}]}),json!({"data":[{"id":"p.second","type":"library-playlists","attributes":{"name":"Second","canEdit":false}}]})]).await;
    let first = c
        .browse(
            "playlists",
            "",
            None,
            "pl",
            crate::config::Config::default().performance.limits(),
        )
        .await
        .unwrap();
    let second = c
        .browse(
            "playlists",
            "",
            None,
            "pl",
            crate::config::Config::default().performance.limits(),
        )
        .await
        .unwrap();
    assert_ne!(first.items[0].id, second.items[0].id);
    assert_eq!(http.await.unwrap().len(), 3);
}
#[tokio::test]
async fn account_replacement_between_playlist_pages_rejects_whole_context() {
    let page = |offset: usize| json!({"data":(offset..offset+25).map(|i|json!({"id":format!("i.{i}"),"type":"library-songs","attributes":{"name":"Synthetic","playParams":{"catalogId":(i+1).to_string()}}})).collect::<Vec<_>>(),"meta":{"total":50}});
    let mut first = page(0);
    first["next"] = json!("ignored");
    let region = json!({"data":[{"id":"pl"}]});
    let (dir, c, http) = fixture(vec![region.clone(), first, region, page(25)]).await;
    let p = playlist::PlaylistRef {
        id: "p.fixture".into(),
        storefront: "pl".into(),
        title: "Synthetic".into(),
        can_edit: true,
    };
    let mut changed = false;
    let result = c
        .playlist(
            &p,
            crate::config::PerformanceMode::Balanced.limits(),
            |count| {
                if count == 25 && !changed {
                    crate::config::private_write(
                        &dir.path().join("c/music-user-token"),
                        b"fixture-user",
                    )
                    .unwrap();
                    changed = true;
                }
            },
        )
        .await;
    assert!(
        result.is_err(),
        "Mixed credential generations must not become one queue"
    );
    http.await.unwrap();
}
