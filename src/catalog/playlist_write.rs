//! Account write contracts. No retry policy is hidden in the HTTP client.
use super::*;
use crate::catalog::playlist::PlaylistRef;
use serde::{Deserialize, Serialize};
pub const MAX_WRITE_TRACKS: usize = 1000;
pub const MAX_WRITE_BYTES: usize = 128 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CredentialStamp {
    pub inode: u64,
    pub mtime: i64,
    pub nanos: i64,
    pub size: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "intent", rename_all = "snake_case")]
pub enum WriteAction {
    Create { name: String, description: String },
    Add { playlist: PlaylistRef },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistWriteResult {
    pub playlist_id: String,
    pub tracks: usize,
}
#[derive(Debug)]
pub(crate) struct WriteFailure {
    pub message: String,
    pub uncertain: bool,
}
impl WriteFailure {
    pub fn rejected(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            uncertain: false,
        }
    }
    pub fn uncertain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            uncertain: true,
        }
    }
}
impl std::fmt::Display for WriteFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for WriteFailure {}
pub(crate) fn payload(action: &WriteAction, ids: &[String]) -> Result<Value> {
    ensure!(
        ids.len() <= MAX_WRITE_TRACKS,
        "Playlist export exceeds 1000 tracks; nothing sent"
    );
    ensure!(
        ids.iter()
            .all(|id| !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_digit())),
        "Playlist export requires available catalog song IDs; nothing sent"
    );
    let data = ids
        .iter()
        .map(|id| serde_json::json!({"id":id,"type":"songs"}))
        .collect::<Vec<_>>();
    let body = match action {
        WriteAction::Create { name, description } => {
            ensure!(
                !name.trim().is_empty()
                    && name.len() <= 200
                    && !name.chars().any(char::is_control)
                    && description.len() <= 2000
                    && !description.chars().any(char::is_control),
                "Playlist name must be 1..200 bytes and description at most 2000 bytes without control characters"
            );
            let mut body = serde_json::json!({"attributes":{"name":name,"description":description,"isPublic":false}});
            if !data.is_empty() {
                body["relationships"] = serde_json::json!({"tracks":{"data":data}});
            }
            body
        }
        WriteAction::Add { playlist } => {
            playlist.validate()?;
            ensure!(playlist.can_edit, "This playlist is read-only");
            ensure!(!data.is_empty(), "No songs to add to playlist");
            serde_json::json!({"data":data})
        }
    };
    ensure!(
        serde_json::to_vec(&body)?.len() <= MAX_WRITE_BYTES,
        "Playlist request exceeds 128 KiB; nothing sent"
    );
    Ok(body)
}
impl Catalog {
    pub(crate) fn playlist_account(&self) -> Result<CredentialStamp> {
        self.apple
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Playlist writes require Apple Music account access"))?
            .credential_stamp()
    }
    pub(crate) async fn write_playlist(
        &self,
        action: &WriteAction,
        ids: &[String],
        account: CredentialStamp,
    ) -> std::result::Result<PlaylistWriteResult, WriteFailure> {
        self.apple
            .as_ref()
            .ok_or_else(|| {
                WriteFailure::rejected("Playlist writes require Apple Music account access")
            })?
            .write_playlist(action, ids, account)
            .await
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn creation_and_append_preserve_duplicates_and_reject_partial_export() {
        let body = payload(
            &WriteAction::Create {
                name: "Fixture".into(),
                description: "".into(),
            },
            &["1".into(), "1".into(), "2".into()],
        )
        .unwrap();
        assert_eq!(
            body["relationships"]["tracks"]["data"],
            serde_json::json!([{"id":"1","type":"songs"},{"id":"1","type":"songs"},{"id":"2","type":"songs"}])
        );
        assert_eq!(body["attributes"]["isPublic"], false);
        assert!(
            payload(
                &WriteAction::Create {
                    name: "Fixture".into(),
                    description: "".into()
                },
                &vec!["1".into(); 1001]
            )
            .is_err()
        );
        assert!(
            payload(
                &WriteAction::Create {
                    name: "Fixture".into(),
                    description: "".into()
                },
                &["i.local".into()]
            )
            .is_err()
        );
        let playlist = PlaylistRef {
            id: "p.fixture".into(),
            storefront: "pl".into(),
            title: "Fixture".into(),
            can_edit: false,
        };
        assert!(payload(&WriteAction::Add { playlist }, &["1".into()]).is_err());
    }
}
