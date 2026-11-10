//! Visible album actions, sharing the service's complete-album transaction API.
use super::*;
use crate::{
    catalog::album::AlbumRef,
    ipc::{AlbumAction, QueueAction},
};
pub(super) struct AlbumActions {
    pub album: AlbumRef,
    selected: usize,
}
const ACTIONS: [&str; 4] = [
    "Play album — replaces current queue",
    "Shuffle album — replaces current queue",
    "Play album next — FIFO block",
    "Append whole album to end",
];
impl App {
    pub(super) fn revision(&self) -> u64 {
        self.status.as_ref().map_or(0, |s| s.queue_revision)
    }
    pub(super) fn selected_album(&self) -> Option<AlbumRef> {
        if let Some(track) = self.items.get(self.selected).filter(|t| t.kind == "albums") {
            Some(AlbumRef {
                id: track.id.clone(),
                provider: if track.provider.is_empty() {
                    "music".into()
                } else {
                    track.provider.clone()
                },
                storefront: if track.storefront.is_empty() {
                    self.config.catalog_country.to_ascii_lowercase()
                } else {
                    track.storefront.clone()
                },
                title: track.title.clone(),
            })
        } else {
            self.album_view.clone()
        }
    }
    pub(super) fn album_request(&self, album: AlbumRef, action: AlbumAction) -> Request {
        Request::Album {
            key: ipc::operation_key(),
            revision: self.revision(),
            album,
            action,
        }
    }
    pub(super) fn open_album_actions(&mut self) {
        if let Some(album) = self.selected_album() {
            self.album_actions = Some(AlbumActions { album, selected: 0 });
        } else {
            self.note = "Choose an album result or open an album first".into();
        }
    }
    pub(super) fn album_menu_key(&mut self, key: KeyCode) -> (bool, Option<Request>) {
        let Some(menu) = self.album_actions.as_mut() else {
            return (false, None);
        };
        match key {
            KeyCode::Esc | KeyCode::Left => self.album_actions = None,
            KeyCode::Down | KeyCode::Char('j') => menu.selected = (menu.selected + 1).min(3),
            KeyCode::Up | KeyCode::Char('k') => menu.selected = menu.selected.saturating_sub(1),
            KeyCode::Enter | KeyCode::Right => {
                let menu = self.album_actions.take().expect("menu exists");
                let action = match menu.selected {
                    0 => AlbumAction::Play {
                        start_song: None,
                        shuffle: false,
                    },
                    1 => AlbumAction::Play {
                        start_song: None,
                        shuffle: true,
                    },
                    2 => AlbumAction::PlayNext,
                    _ => AlbumAction::Append,
                };
                return (false, Some(self.album_request(menu.album, action)));
            }
            _ => {}
        }
        (false, None)
    }
    pub(super) fn render_album_menu(&self, f: &mut Frame, p: Palette) {
        let Some(menu) = &self.album_actions else {
            return;
        };
        let size = f.area();
        let width = size.width.min(66);
        let height = size.height.min(10);
        let area = Rect::new(
            (size.width - width) / 2,
            (size.height - height) / 2,
            width,
            height,
        );
        f.render_widget(Clear, area);
        let block = Block::bordered()
            .title(" ALBUM ACTIONS ")
            .style(Style::default().fg(p.fg).bg(p.bg));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let [title, actions, hint] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(2),
        ])
        .areas(inner);
        f.render_widget(Paragraph::new(menu.album.title.as_str()), title);
        let rows = visible_range(menu.selected, 4, actions.height as usize)
            .map(|i| {
                ListItem::new(format!(
                    "{} {}",
                    if menu.selected == i { "›" } else { " " },
                    ACTIONS[i]
                ))
                .style(if menu.selected == i {
                    Style::default().bold().underlined()
                } else {
                    Style::default()
                })
            })
            .collect::<Vec<_>>();
        f.render_widget(List::new(rows), actions);
        f.render_widget(Paragraph::new("↑/↓ choose · Enter confirm · Esc back\nReplace/clear can be undone with u; audio position is not restored"),hint);
    }
    pub(super) fn song_request(&self, track: Track, intent: usize) -> Request {
        if intent == 2
            && let Some(playlist) = &self.playlist_view
        {
            return self.playlist_request(
                playlist.clone(),
                ipc::ContextAction::PlayFromPosition {
                    source_index: track
                        .source_index
                        .unwrap_or(self.catalog_offset + self.selected),
                    shuffle: false,
                },
            );
        }
        if intent == 2
            && let Some(album) = &self.album_view
        {
            return self.album_request(
                album.clone(),
                AlbumAction::Play {
                    start_song: Some(track.id),
                    shuffle: false,
                },
            );
        }
        let context = crate::queue::QueueContext {
            provider: track.provider.clone(),
            storefront: track.storefront.clone(),
            ..crate::queue::QueueContext::single()
        };
        let action = match intent {
            2 => QueueAction::PlayContext {
                tracks: vec![track],
                context,
                start: 0,
                shuffle: false,
            },
            3 => QueueAction::PlayNext {
                tracks: vec![track],
                context,
            },
            _ => QueueAction::Append {
                tracks: vec![track],
                context,
            },
        };
        ipc::queue_action(self.revision(), action)
    }
    pub(super) fn status_message(&self) -> String {
        if let Some(error) = self.status.as_ref().and_then(|s| s.audio_error.as_ref()) {
            return format!("Failed: {error} · Play retries audio");
        }
        if let Some(error) = self.status.as_ref().and_then(|s| s.config_error.as_ref()) {
            return error.clone();
        }
        if let Some(write) = self
            .status
            .as_ref()
            .and_then(|s| s.playlist_write.as_ref())
            .filter(|s| s.phase != "completed")
        {
            return format!(
                "Playlist write {}: {}{}",
                write.phase,
                write.message,
                if write.can_retry {
                    " · :playlist-retry"
                } else {
                    ""
                }
            );
        }
        if let Some(op) = self.status.as_ref().and_then(|s| s.operation.as_ref()) {
            if matches!(
                op.phase.as_str(),
                "preparing" | "saving" | "failed" | "cancelled"
            ) {
                return format!(
                    "Album/queue {} · {} records · {} · {}",
                    op.phase,
                    op.loaded,
                    op.message,
                    match op.phase.as_str() {
                        "preparing" => "c cancel",
                        "saving" => "commit in progress; cannot cancel",
                        _ => "R retry",
                    }
                );
            }
            let skipped = op
                .skipped
                .iter()
                .map(|s| format!("{}: {}", s.count, s.reason))
                .collect::<Vec<_>>()
                .join("; ");
            if !skipped.is_empty() {
                return format!("Queue committed · skipped {skipped}");
            }
        }
        self.note.clone()
    }
}
