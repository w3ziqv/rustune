use super::*;
use crate::catalog::playlist_write::{
    CredentialStamp, PlaylistWriteResult, WriteAction, WriteFailure, payload,
};
impl AppleCatalog {
    pub(crate) fn credential_stamp(&self) -> Result<CredentialStamp> {
        let s = stamp(&self.paths)
            .ok_or_else(|| anyhow!("A private Music User Token is required for playlist writes"))?;
        Ok(CredentialStamp {
            inode: s.inode,
            mtime: s.mtime,
            nanos: s.nanos,
            size: s.size,
        })
    }
    pub(crate) async fn write_playlist(
        &self,
        action: &WriteAction,
        ids: &[String],
        account: CredentialStamp,
    ) -> std::result::Result<PlaylistWriteResult, WriteFailure> {
        let body = payload(action, ids).map_err(|e| WriteFailure::rejected(e.to_string()))?;
        let mut state = self.state.lock().await;
        let preflight = async {
            ensure!(
                self.credential_stamp()? == account,
                "Account credentials changed before playlist write; nothing sent"
            );
            let region = self.get("me/storefront", &[], true, &mut state).await?;
            if let WriteAction::Add { playlist } = action {
                ensure!(
                    region["data"]
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(|r| r["id"].as_str())
                        .is_some_and(|r| r.eq_ignore_ascii_case(&playlist.storefront)),
                    "Account storefront changed; refresh playlist before writing"
                );
                let response = self
                    .get(
                        &format!("me/library/playlists/{}", playlist.id),
                        &[],
                        true,
                        &mut state,
                    )
                    .await?;
                let resource = &response["data"][0];
                ensure!(
                    resource["id"] == playlist.id
                        && resource["type"] == "library-playlists"
                        && resource["attributes"]["canEdit"] == true,
                    "Playlist is no longer editable; nothing sent"
                );
            }
            ensure!(
                state.calls.len() < 20,
                "Apple Music request budget reached; retry later"
            );
            let headers = self.headers(true).await?;
            ensure!(
                self.credential_stamp()? == account,
                "Account credentials changed before playlist write; nothing sent"
            );
            Ok::<_, anyhow::Error>(headers)
        }
        .await;
        let headers = preflight.map_err(|e| WriteFailure::rejected(e.to_string()))?;
        let path = match action {
            WriteAction::Create { .. } => "me/library/playlists".into(),
            WriteAction::Add { playlist } => format!("me/library/playlists/{}/tracks", playlist.id),
        };
        let bytes = serde_json::to_vec(&body)
            .map_err(|_| WriteFailure::rejected("Invalid playlist payload"))?;
        state.cache = Cache::new(self.capacity.load(Ordering::Relaxed));
        self.entries.store(0, Ordering::Relaxed);
        state.calls.push_back(Instant::now());
        let response = self
            .client
            .post(format!("{}/v1/{path}", self.origin))
            .headers(headers)
            .header("content-type", "application/json")
            .body(bytes)
            .send()
            .await;
        let mut response=response.map_err(|_|WriteFailure::uncertain("Playlist response was lost; check Apple Music. This operation will not be sent again automatically"))?;
        let code = response.status().as_u16();
        if code == 429 {
            let wait = response
                .headers()
                .get("retry-after")
                .and_then(|h| h.to_str().ok())
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or(60)
                .clamp(1, 3600);
            state.blocked = Some(Instant::now() + Duration::from_secs(wait));
        }
        if !(200..300).contains(&code) {
            let message = format!("Apple playlist write returned HTTP {code}");
            return Err(if code >= 500 || code == 408 {
                WriteFailure::uncertain(format!(
                    "{message}; outcome uncertain, inspect Apple Music before a new operation"
                ))
            } else {
                WriteFailure::rejected(message)
            });
        }
        if self.credential_stamp().ok() != Some(account) {
            return Err(WriteFailure::uncertain(
                "Credentials changed during playlist write; inspect the original account before retrying",
            ));
        }
        let result = match action {
            WriteAction::Add { playlist } if code == 204 => PlaylistWriteResult {
                playlist_id: playlist.id.clone(),
                tracks: ids.len(),
            },
            WriteAction::Create { .. } if code == 201 => {
                let mut bytes = Vec::new();
                while let Some(chunk)=response.chunk().await.map_err(|_|WriteFailure::uncertain("Playlist may have been created but response was interrupted; check Apple Music"))? {if bytes.len()+chunk.len()>MAX_BODY{return Err(WriteFailure::uncertain("Playlist may have been created but response exceeds limits; check Apple Music"));}bytes.extend_from_slice(&chunk);}
                let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
                    WriteFailure::uncertain(
                        "Playlist may have been created but response is invalid; check Apple Music",
                    )
                })?;
                let id=value["data"][0]["id"].as_str().filter(|id|crate::catalog::playlist::valid_id(id)).ok_or_else(||WriteFailure::uncertain("Playlist may have been created but identifier is missing; check Apple Music"))?;
                PlaylistWriteResult {
                    playlist_id: id.into(),
                    tracks: ids.len(),
                }
            }
            _ => {
                return Err(WriteFailure::uncertain(
                    "Unexpected successful playlist response; inspect Apple Music before a new operation",
                ));
            }
        };
        state.cache = Cache::new(self.capacity.load(Ordering::Relaxed));
        self.entries.store(0, Ordering::Relaxed);
        Ok(result)
    }
}
