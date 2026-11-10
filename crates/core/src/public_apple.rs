use crate::paths::{Paths, private_write};
use anyhow::{Result, anyhow, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Client, Url};
use serde_json::Value;
use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;
pub fn public_redirect(base: &Url, location: &str) -> Result<Url> {
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

pub fn read_private_secret(path: &Path) -> Result<Zeroizing<String>> {
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
pub async fn bounded(response: reqwest::Response, max: usize) -> Result<Zeroizing<Vec<u8>>> {
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
pub async fn application_token(client: &Client, paths: &Paths) -> Result<Zeroizing<String>> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let cache = paths.cache.join("apple-app-token");
    if let Ok(token) = read_private_secret(&cache)
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
