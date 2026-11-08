//! Anonymous application authorization and private user authorization for native playback.
use crate::config::{Paths, private_write};
use anyhow::{Result, anyhow, ensure};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use reqwest::{
    Client, Url,
    header::{HeaderMap, HeaderValue},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

pub struct Manifest {
    pub media: Url,
    pub key_uri: String,
    pub key_id: Vec<u8>,
    pub duration: f64,
}
pub fn trusted_media(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.fragment().is_none()
        && url.host_str().is_some_and(|h| {
            h.ends_with(".itunes.apple.com")
                || h.ends_with(".mzstatic.com")
                || h == "music.apple.com"
        })
}
fn public_redirect(base: &Url, location: &str) -> Result<Url> {
    let url = base
        .join(location)
        .map_err(|_| anyhow!("Invalid public Apple redirect"))?;
    ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && url.fragment().is_none()
            && matches!(
                url.host_str(),
                Some("music.apple.com" | "js-cdn.music.apple.com")
            ),
        "Public Apple redirect left the trusted HTTPS origin"
    );
    Ok(url)
}
async fn anonymous_get(client: &Client, mut url: Url, max: usize) -> Result<Zeroizing<Vec<u8>>> {
    for _ in 0..5 {
        let response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|_| anyhow!("Cannot retrieve public Apple application metadata"))?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|h| h.to_str().ok())
                .ok_or_else(|| anyhow!("Missing public Apple redirect destination"))?;
            url = public_redirect(&url, location)?;
        } else {
            return bounded(response, max).await;
        }
    }
    Err(anyhow!("Too many public Apple application redirects"))
}
pub fn parse_manifest(base: &Url, text: &str) -> Result<Manifest> {
    ensure!(
        trusted_media(base)
            && text.len() <= 512 * 1024
            && text.starts_with("#EXTM3U")
            && text.lines().any(|l| l == "#EXT-X-ENDLIST"),
        "Unsupported or incomplete native stream manifest"
    );
    let keys = text
        .lines()
        .filter(|l| l.starts_with("#EXT-X-KEY:"))
        .collect::<Vec<_>>();
    ensure!(
        keys.len() == 1 && keys[0].contains("METHOD=ISO-23001-7,"),
        "Only CENC AAC streams are supported"
    );
    let uri = keys[0]
        .split_once("URI=\"")
        .and_then(|(_, s)| s.split('"').next())
        .ok_or_else(|| anyhow!("Missing CENC initialization metadata"))?;
    let id = uri
        .strip_prefix("data:text/plain;base64,")
        .or_else(|| uri.strip_prefix("data:;base64,"))
        .ok_or_else(|| anyhow!("Unsupported CENC initialization metadata"))?;
    let key_id = STANDARD
        .decode(id)
        .map_err(|_| anyhow!("Invalid CENC key identifier"))?;
    ensure!(key_id.len() == 16, "Invalid CENC key identifier size");
    let mut media = None;
    let mut duration = 0.0;
    let mut segments = 0;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("#EXTINF:") {
            let seconds = value
                .split(',')
                .next()
                .unwrap_or("")
                .parse::<f64>()
                .map_err(|_| anyhow!("Invalid stream duration"))?;
            ensure!(
                seconds.is_finite() && seconds > 0.0 && seconds <= 600.0,
                "Invalid stream duration"
            );
            duration += seconds;
            segments += 1;
        } else if !line.trim().is_empty() && !line.starts_with('#') {
            let url = base
                .join(line)
                .map_err(|_| anyhow!("Invalid stream resource"))?;
            ensure!(
                trusted_media(&url) && media.as_ref().is_none_or(|previous| previous == &url),
                "Unsupported stream resource"
            );
            media = Some(url);
        }
    }
    ensure!(
        (1..=4096).contains(&segments) && duration <= 6.0 * 3600.0,
        "Native stream exceeds duration limits"
    );
    Ok(Manifest {
        media: media.ok_or_else(|| anyhow!("Missing stream resource"))?,
        key_uri: uri.to_owned(),
        key_id,
        duration,
    })
}
pub fn valid_public_token(text: &str, now: u64) -> bool {
    if text.len() > 16 * 1024 {
        return false;
    }
    let parts = text.split('.').collect::<Vec<_>>();
    if parts.len() != 3 {
        return false;
    }
    let decode = |s: &str| {
        URL_SAFE_NO_PAD
            .decode(s)
            .ok()
            .and_then(|v| serde_json::from_slice::<Value>(&v).ok())
    };
    let (Some(header), Some(payload)) = (decode(parts[0]), decode(parts[1])) else {
        return false;
    };
    header["alg"] == "ES256"
        && header["kid"].as_str().is_some_and(|v| !v.is_empty())
        && payload["iss"].as_str().is_some_and(|v| !v.is_empty())
        && payload["exp"]
            .as_u64()
            .is_some_and(|expiry| expiry > now.saturating_add(60))
}

fn private_secret(path: &Path) -> Result<Zeroizing<String>> {
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| anyhow!("Private token file is unavailable; run rustune login"))?;
    let m = file
        .metadata()
        .map_err(|_| anyhow!("Cannot inspect private token file"))?;
    ensure!(
        m.is_file()
            && m.mode() & 0o077 == 0
            && m.uid() == nix::unistd::Uid::current().as_raw()
            && m.len() > 0
            && m.len() <= 16384,
        "Token file must be owned by you, mode 0600, and at most 16 KiB"
    );
    let mut text = Zeroizing::new(String::new());
    file.by_ref()
        .take(16385)
        .read_to_string(&mut text)
        .map_err(|_| anyhow!("Cannot read private token"))?;
    ensure!(text.len() <= 16384, "Token file exceeds limit");
    Ok(text)
}
async fn bounded(response: reqwest::Response, max: usize) -> Result<Zeroizing<Vec<u8>>> {
    let mut response = response;
    ensure!(
        response.status().is_success(),
        "Apple request rejected (HTTP {})",
        response.status().as_u16()
    );
    let mut body = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow!("Apple response interrupted"))?
    {
        ensure!(
            body.len().saturating_add(chunk.len()) <= max,
            "Apple response exceeds limit"
        );
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
async fn public_token(client: &Client, paths: &Paths) -> Result<Zeroizing<String>> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let cache = paths.cache.join("apple-app-token");
    if let Ok(token) = private_secret(&cache)
        && valid_public_token(token.trim(), now)
    {
        return Ok(token);
    }
    let html = anonymous_get(
        client,
        Url::parse("https://music.apple.com/").expect("fixed public origin"),
        4 * 1024 * 1024,
    )
    .await?;
    let html =
        std::str::from_utf8(&html).map_err(|_| anyhow!("Invalid public application metadata"))?;
    let base = Url::parse("https://music.apple.com/").expect("fixed public origin");
    for fragment in html.split("src=").skip(1).take(16) {
        let Some(quote) = fragment.chars().next().filter(|c| matches!(c, '\'' | '"')) else {
            continue;
        };
        let Some(path) = fragment[1..].split(quote).next() else {
            continue;
        };
        let Ok(url) = public_redirect(&base, path) else {
            continue;
        };
        if url.scheme() != "https"
            || !url
                .host_str()
                .is_some_and(|h| h == "music.apple.com" || h == "js-cdn.music.apple.com")
            || !url.path().ends_with(".js")
        {
            continue;
        }
        let script = anonymous_get(client, url, 8 * 1024 * 1024).await?;
        for candidate in script.split(|b| !b.is_ascii_alphanumeric() && !b"._-".contains(b)) {
            if !candidate.starts_with(b"eyJ") {
                continue;
            }
            if let Ok(token) = std::str::from_utf8(candidate)
                && valid_public_token(token, now)
            {
                private_write(&cache, token.as_bytes())?;
                return Ok(Zeroizing::new(token.to_owned()));
            }
        }
    }
    Err(anyhow!(
        "Public Apple application authorization was not found; the website may have changed"
    ))
}

pub struct Session {
    client: Client,
    headers: HeaderMap,
}
impl Session {
    pub async fn new(paths: &Paths) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Rustune/0.1 native Linux audio")
            .build()
            .map_err(|_| anyhow!("Cannot initialize native HTTPS client"))?;
        let user = private_secret(&paths.config.join("music-user-token"))?;
        let app = public_token(&client, paths).await?;
        let mut headers = HeaderMap::new();
        for (name, value) in [
            ("media-user-token", Zeroizing::new(user.trim().to_owned())),
            (
                "authorization",
                Zeroizing::new(format!("Bearer {}", app.trim())),
            ),
        ] {
            let mut header = HeaderValue::from_str(&value)
                .map_err(|_| anyhow!("Invalid token format; enter it again using rustune login"))?;
            header.set_sensitive(true);
            headers.insert(name, header);
        }
        for (name, value) in [
            ("origin", "https://music.apple.com"),
            ("referer", "https://music.apple.com/"),
            ("content-type", "application/json"),
            ("accept", "application/json"),
        ] {
            headers.insert(name, HeaderValue::from_static(value));
        }
        Ok(Self { client, headers })
    }
    pub async fn prepare(&self, id: &str, cdm: &cdm_host::CdmHost) -> Result<Manifest> {
        ensure!(
            !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_digit()),
            "Native playback currently supports catalog song IDs only"
        );
        let response = self
            .client
            .post("https://play.music.apple.com/WebObjects/MZPlay.woa/wa/webPlayback")
            .headers(self.headers.clone())
            .body(json!({"salableAdamId":id}).to_string())
            .send()
            .await
            .map_err(|_| anyhow!("Apple playback request failed"))?;
        let body = bounded(response, 512 * 1024).await?;
        let body: Value =
            serde_json::from_slice(&body).map_err(|_| anyhow!("Invalid playback response"))?;
        let song=body["songList"].as_array().and_then(|v|v.first()).ok_or_else(||anyhow!("Apple did not authorize this song; check subscription, region and Music User Token"))?;
        let asset = song["assets"]
            .as_array()
            .and_then(|assets| assets.iter().find(|a| a["flavor"] == "28:ctrp256"))
            .ok_or_else(|| anyhow!("No supported full CENC AAC stream is available"))?;
        let url = asset["URL"]
            .as_str()
            .and_then(|v| Url::parse(v).ok())
            .filter(trusted_media)
            .ok_or_else(|| anyhow!("Unsupported Apple media origin"))?;
        let response = self
            .client
            .get(url.clone())
            .send()
            .await
            .map_err(|_| anyhow!("Cannot retrieve native stream manifest"))?;
        let manifest = bounded(response, 512 * 1024).await?;
        let manifest = parse_manifest(
            &url,
            std::str::from_utf8(&manifest)
                .map_err(|_| anyhow!("Invalid stream manifest encoding"))?,
        )?;
        let mut protobuf = vec![0x08, 0x01, 0x12, 0x10];
        protobuf.extend_from_slice(&manifest.key_id);
        let mut pssh = Vec::with_capacity(52);
        pssh.extend_from_slice(&52u32.to_be_bytes());
        pssh.extend_from_slice(b"pssh");
        pssh.extend_from_slice(&[0; 4]);
        pssh.extend_from_slice(&[
            0xed, 0xef, 0x8b, 0xa9, 0x79, 0xd6, 0x4a, 0xce, 0xa3, 0xc8, 0x27, 0xdc, 0xd5, 0x1d,
            0x21, 0xed,
        ]);
        pssh.extend_from_slice(&20u32.to_be_bytes());
        pssh.extend_from_slice(&protobuf);
        let challenge = Zeroizing::new(
            cdm.challenge(&pssh)
                .map_err(|_| anyhow!("Native CDM rejected stream initialization"))?,
        );
        let license_url = song["hls-key-server-url"]
            .as_str()
            .and_then(|v| Url::parse(v).ok())
            .filter(|u| {
                u.scheme() == "https"
                    && u.username().is_empty()
                    && u.password().is_none()
                    && u.port().is_none()
                    && matches!(
                        u.host_str(),
                        Some("play.music.apple.com" | "play.itunes.apple.com")
                    )
            })
            .ok_or_else(|| anyhow!("Unsupported Apple license origin"))?;
        let payload=Zeroizing::new(json!({"challenge":STANDARD.encode(&*challenge),"key-system":"com.widevine.alpha","uri":manifest.key_uri,"adamId":id,"isLibrary":false,"user-initiated":true}).to_string());
        let response = self
            .client
            .post(license_url)
            .headers(self.headers.clone())
            .body(payload.to_string())
            .send()
            .await
            .map_err(|_| anyhow!("Apple license request failed"))?;
        let body = bounded(response, 512 * 1024).await?;
        let body: Value =
            serde_json::from_slice(&body).map_err(|_| anyhow!("Invalid Apple license response"))?;
        let license = body["license"]
            .as_str()
            .and_then(|v| STANDARD.decode(v).ok())
            .map(Zeroizing::new)
            .ok_or_else(|| anyhow!("Apple did not issue a native playback license"))?;
        cdm.update(&license)
            .map_err(|_| anyhow!("Native CDM rejected the Apple license"))?;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    #[test]
    fn anonymous_public_redirects_stay_on_verified_apple_https_origins() {
        let base = Url::parse("https://music.apple.com/").unwrap();
        assert_eq!(
            public_redirect(&base, "/pl/browse").unwrap().as_str(),
            "https://music.apple.com/pl/browse"
        );
        for invalid in [
            "http://music.apple.com/",
            "https://music.apple.com.evil.example/",
            "https://user@music.apple.com/",
            "https://music.apple.com:8443/",
            "file:///etc/passwd",
        ] {
            assert!(public_redirect(&base, invalid).is_err());
        }
    }
    const PLAYLIST: &str = "#EXTM3U\n#EXT-X-KEY:METHOD=ISO-23001-7,URI=\"data:text/plain;base64,BwcHBwcHBwcHBwcHBwcHBw==\"\n#EXTINF:10.0,\n#EXT-X-BYTERANGE:100@100\nmedia.m4a\n#EXTINF:2.5,\n#EXT-X-BYTERANGE:25@200\nmedia.m4a\n#EXT-X-ENDLIST\n";
    #[test]
    fn accepts_only_complete_single_resource_cenc_playlists() {
        let base = Url::parse("https://audio-ssl.itunes.apple.com/path/list.m3u8").unwrap();
        let parsed = parse_manifest(&base, PLAYLIST).unwrap();
        assert_eq!(
            parsed.media.as_str(),
            "https://audio-ssl.itunes.apple.com/path/media.m4a"
        );
        assert_eq!(parsed.key_id, [7; 16]);
        assert_eq!(parsed.duration, 12.5);
        assert!(
            parse_manifest(
                &base,
                &PLAYLIST.replace("data:text/plain;base64,", "data:;base64,")
            )
            .is_ok()
        );
        for invalid in [
            PLAYLIST.replace("#EXT-X-ENDLIST", ""),
            PLAYLIST.replacen("media.m4a", "https://evil.example/file", 1),
            PLAYLIST.replacen("media.m4a", "different.m4a", 1),
            PLAYLIST.replace("ISO-23001-7", "SAMPLE-AES"),
            PLAYLIST.replace("#EXTINF:10.0,", "#EXTINF:NaN,"),
        ] {
            assert!(parse_manifest(&base, &invalid).is_err());
        }
    }
    #[test]
    fn public_authorization_requires_es256_issuer_and_future_expiry() {
        let jwt = |header: &str, payload: &str| {
            format!(
                "{}.{}.test",
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(header),
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload)
            )
        };
        assert!(valid_public_token(
            &jwt(
                r#"{"alg":"ES256","kid":"test"}"#,
                r#"{"iss":"test","exp":2000}"#
            ),
            1000
        ));
        for token in [
            jwt(
                r#"{"alg":"none","kid":"test"}"#,
                r#"{"iss":"test","exp":2000}"#,
            ),
            jwt(
                r#"{"alg":"ES256","kid":"test"}"#,
                r#"{"iss":"test","exp":900}"#,
            ),
            "invalid".into(),
        ] {
            assert!(!valid_public_token(&token, 1000));
        }
    }
}
