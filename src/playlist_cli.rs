use anyhow::Result;
use clap::{Args, Subcommand};
use rustune::{
    catalog::{
        playlist::{PlaylistRef, page_cursor},
        playlist_write::WriteAction,
    },
    ipc::{self, ContextAction, PlaylistWriteSource, Request},
};
#[derive(Args)]
pub struct Target {
    id: String,
    #[arg(long)]
    storefront: Option<String>,
    #[arg(long)]
    key: Option<String>,
    #[arg(long)]
    revision: Option<u64>,
}
#[derive(Subcommand)]
pub enum PlaylistCommand {
    /// Browse own playlists; offset is a multiple of 25
    List {
        #[arg(long)]
        storefront: Option<String>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
    /// Browse songs without changing playback
    Tracks {
        id: String,
        #[arg(long)]
        storefront: Option<String>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
    /// Replace the queue with a complete playlist
    Play {
        #[command(flatten)]
        target: Target,
        #[arg(long)]
        shuffle: bool,
        #[arg(long)]
        from_position: Option<usize>,
    },
    /// Insert the complete playlist as the next FIFO block
    Next(Target),
    /// Append the complete playlist to the queue
    Append(Target),
    /// Create a private empty Apple Music playlist (explicit account write)
    Create {
        name: String,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long)]
        key: Option<String>,
    },
    /// Save the entire queue as a private Apple Music playlist (account write)
    Save {
        name: String,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        revision: Option<u64>,
    },
    /// Append the queue, or one --song/--entry-id, to an editable Apple playlist
    Add {
        #[command(flatten)]
        target: Target,
        #[arg(long, conflicts_with = "entry_id")]
        song: Option<String>,
        #[arg(long)]
        entry_id: Option<u64>,
    },
    /// Retry only a definitely rejected write; uncertain writes are blocked
    Retry,
}
impl PlaylistCommand {
    pub fn request(self, revision: u64, country: &str, generation: Option<u64>) -> Result<Request> {
        let cursor = |storefront: Option<String>, offset| {
            let generation = generation
                .ok_or_else(|| anyhow::anyhow!("Personal library requires a Music User Token"))?;
            Ok::<_, anyhow::Error>(page_cursor(
                storefront.as_deref().unwrap_or(country),
                generation,
                offset,
            ))
        };
        let reference = |id, storefront| PlaylistRef {
            id,
            storefront,
            title: "Playlist".into(),
            can_edit: true,
        };
        let target = |t: Target| {
            let p = reference(
                t.id,
                t.storefront.unwrap_or_else(|| country.to_ascii_lowercase()),
            );
            (
                t.key.unwrap_or_else(ipc::operation_key),
                t.revision.unwrap_or(revision),
                p,
            )
        };
        let context = |t, action| {
            let (key, revision, playlist) = target(t);
            Request::Playlist {
                key,
                revision,
                playlist,
                action,
            }
        };
        let write = |key: Option<String>, source, action| Request::PlaylistWrite {
            key: key.unwrap_or_else(ipc::operation_key),
            source,
            action,
        };
        Ok(match self {
            Self::List { offset, storefront } => Request::Browse {
                kind: "playlists".into(),
                term: String::new(),
                next: if offset == 0 && storefront.is_none() {
                    None
                } else {
                    Some(cursor(storefront, offset)?)
                },
            },
            Self::Tracks {
                id,
                storefront,
                offset,
            } => {
                anyhow::ensure!(
                    rustune::catalog::playlist::valid_id(&id),
                    "Invalid playlist ID"
                );
                Request::Browse {
                    kind: format!("open:library-playlists:{id}"),
                    term: String::new(),
                    next: Some(cursor(storefront, offset)?),
                }
            }
            Self::Play {
                target,
                shuffle,
                from_position,
            } => context(
                target,
                match from_position {
                    Some(source_index) => ContextAction::PlayFromPosition {
                        source_index,
                        shuffle,
                    },
                    None => ContextAction::Play {
                        start_song: None,
                        shuffle,
                    },
                },
            ),
            Self::Next(t) => context(t, ContextAction::PlayNext),
            Self::Append(t) => context(t, ContextAction::Append),
            Self::Create {
                name,
                description,
                key,
            } => write(
                key,
                PlaylistWriteSource::Empty,
                WriteAction::Create { name, description },
            ),
            Self::Save {
                name,
                description,
                key,
                revision: r,
            } => write(
                key,
                PlaylistWriteSource::Queue {
                    revision: r.unwrap_or(revision),
                },
                WriteAction::Create { name, description },
            ),
            Self::Add {
                target: t,
                song,
                entry_id,
            } => {
                let (key, revision, playlist) = target(t);
                let source = match (song, entry_id) {
                    (Some(id), None) => PlaylistWriteSource::Track {
                        track: Box::new(rustune::model::Track {
                            id,
                            kind: "songs".into(),
                            ..Default::default()
                        }),
                    },
                    (None, Some(entry_id)) => PlaylistWriteSource::Entry { revision, entry_id },
                    (None, None) => PlaylistWriteSource::Queue { revision },
                    _ => anyhow::bail!("Choose song or entry ID"),
                };
                write(Some(key), source, WriteAction::Add { playlist })
            }
            Self::Retry => Request::RetryPlaylistWrite,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn browse(args: &[&str]) -> (String, Option<String>) {
        let cli = crate::Cli::try_parse_from(
            ["rustune", "playlist"]
                .into_iter()
                .chain(args.iter().copied()),
        )
        .unwrap();
        let Some(crate::Commands::Playlist { command }) = cli.command else {
            panic!("expected playlist subcommand");
        };
        let Request::Browse { kind, next, .. } = command.request(7, "PL", Some(42)).unwrap() else {
            panic!("expected Browse");
        };
        (kind, next)
    }

    #[test]
    fn tracks_pins_even_the_first_page_to_the_current_account() {
        let (kind, cursor) = browse(&["tracks", "p.fixture"]);
        assert_eq!(kind, "open:library-playlists:p.fixture");
        assert_eq!(cursor.as_deref(), Some("music:pl:000000000000002a:0"));
    }

    #[test]
    fn list_offset_uses_current_generation_and_selected_storefront() {
        let (kind, cursor) = browse(&["list", "--offset", "25", "--storefront", "DE"]);
        assert_eq!(kind, "playlists");
        assert_eq!(cursor.as_deref(), Some("music:de:000000000000002a:25"));
        assert_eq!(browse(&["list"]).1, None);
    }
}
