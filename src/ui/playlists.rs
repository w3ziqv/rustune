//! Playlist navigation and explicit account-write dialogs; never sends HTTP itself.
use super::*;
use crate::{
    catalog::{playlist::PlaylistRef, playlist_write::WriteAction},
    ipc::{ContextAction, PlaylistWriteSource},
};
pub(super) enum PlaylistDialog {
    Actions {
        playlist: PlaylistRef,
        selected: usize,
    },
    Name {
        source: PlaylistWriteSource,
        text: String,
    },
    Confirm {
        request: Box<Request>,
        summary: String,
    },
}
const ACTIONS: [&str; 5] = [
    "Play playlist — replaces queue",
    "Shuffle playlist — replaces queue",
    "Play playlist next — FIFO",
    "Append playlist to queue",
    "Add current queue to this Apple playlist",
];
impl App {
    pub(super) fn selected_playlist(&self) -> Option<PlaylistRef> {
        self.items
            .get(self.selected)
            .filter(|t| t.kind == "library-playlists")
            .map(|t| PlaylistRef {
                id: t.id.clone(),
                storefront: t.storefront.clone(),
                title: t.title.clone(),
                can_edit: t.can_edit == Some(true),
            })
            .or_else(|| self.playlist_view.clone())
    }
    pub(super) fn playlist_request(&self, playlist: PlaylistRef, action: ContextAction) -> Request {
        Request::Playlist {
            key: ipc::operation_key(),
            revision: self.revision(),
            playlist,
            action,
        }
    }
    pub(super) fn playlist_name(&mut self, queue: bool) {
        self.playlist_dialog = Some(PlaylistDialog::Name {
            source: if queue {
                PlaylistWriteSource::Queue {
                    revision: self.revision(),
                }
            } else {
                PlaylistWriteSource::Empty
            },
            text: String::new(),
        });
    }
    pub(super) fn playlist_confirm(&mut self, source: PlaylistWriteSource, action: WriteAction) {
        let count = match &source {
            PlaylistWriteSource::Queue { .. } => self.status.as_ref().map_or(0, |s| s.queue_len),
            PlaylistWriteSource::Empty => 0,
            _ => 1,
        };
        let summary = match &action {
            WriteAction::Create { name, .. } => format!(
                "Create PRIVATE Apple Music playlist:\n{name}\n\nInclude {count} tracks in current queue order (duplicates retained)."
            ),
            WriteAction::Add { playlist } => format!(
                "Append {count} tracks to Apple Music playlist:\n{}\n\nFresh edit permission will be checked.",
                playlist.title
            ),
        };
        self.playlist_dialog = Some(PlaylistDialog::Confirm {
            request: Box::new(Request::PlaylistWrite {
                key: ipc::operation_key(),
                source,
                action,
            }),
            summary,
        });
    }
    pub(super) fn playlist_destination(&mut self, source: PlaylistWriteSource) -> Request {
        self.remember_view();
        self.playlist_target = Some(source);
        self.go(6).expect("playlist screen requests metadata")
    }
    pub(super) fn playlist_menu_key(&mut self, key: KeyCode) -> (bool, Option<Request>) {
        let Some(dialog) = self.playlist_dialog.take() else {
            return (false, None);
        };
        let next = match dialog {
            PlaylistDialog::Actions {
                playlist,
                mut selected,
            } => match key {
                KeyCode::Esc | KeyCode::Left => None,
                KeyCode::Down | KeyCode::Char('j') => {
                    selected = (selected + 1).min(4);
                    Some(PlaylistDialog::Actions { playlist, selected })
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    selected = selected.saturating_sub(1);
                    Some(PlaylistDialog::Actions { playlist, selected })
                }
                KeyCode::Enter | KeyCode::Right => {
                    if selected == 4 {
                        if playlist.can_edit {
                            self.playlist_confirm(
                                PlaylistWriteSource::Queue {
                                    revision: self.revision(),
                                },
                                WriteAction::Add { playlist },
                            );
                        } else {
                            self.note =
                                "This playlist is read-only; choose an editable destination".into();
                        }
                        return (false, None);
                    }
                    let action = match selected {
                        0 => ContextAction::Play {
                            start_song: None,
                            shuffle: false,
                        },
                        1 => ContextAction::Play {
                            start_song: None,
                            shuffle: true,
                        },
                        2 => ContextAction::PlayNext,
                        _ => ContextAction::Append,
                    };
                    return (false, Some(self.playlist_request(playlist, action)));
                }
                _ => Some(PlaylistDialog::Actions { playlist, selected }),
            },
            PlaylistDialog::Name { source, mut text } => match key {
                KeyCode::Esc => None,
                KeyCode::Enter => {
                    if text.trim().is_empty() {
                        self.note = "Enter a playlist name".into();
                        Some(PlaylistDialog::Name { source, text })
                    } else {
                        self.playlist_confirm(
                            source,
                            WriteAction::Create {
                                name: text,
                                description: String::new(),
                            },
                        );
                        return (false, None);
                    }
                }
                KeyCode::Backspace => {
                    text.pop();
                    Some(PlaylistDialog::Name { source, text })
                }
                KeyCode::Char(c) if !c.is_control() && text.len() + c.len_utf8() <= 200 => {
                    text.push(c);
                    Some(PlaylistDialog::Name { source, text })
                }
                _ => Some(PlaylistDialog::Name { source, text }),
            },
            PlaylistDialog::Confirm { request, summary } => match key {
                KeyCode::Esc | KeyCode::Left => None,
                KeyCode::Enter => return (false, Some(*request)),
                _ => Some(PlaylistDialog::Confirm { request, summary }),
            },
        };
        self.playlist_dialog = next;
        (false, None)
    }
    pub(super) fn playlist_paste(&mut self, text: &str) {
        if let Some(PlaylistDialog::Name { text: entry, .. }) = &mut self.playlist_dialog {
            for c in text.chars().filter(|c| !c.is_control()) {
                if entry.len() + c.len_utf8() > 200 {
                    break;
                }
                entry.push(c);
            }
        }
    }
    pub(super) fn render_playlist_dialog(&self, f: &mut Frame, p: Palette) {
        let Some(dialog) = &self.playlist_dialog else {
            return;
        };
        let size = f.area();
        let w = size.width.min(78);
        let h = size.height.min(18);
        let area = Rect::new((size.width - w) / 2, (size.height - h) / 2, w, h);
        f.render_widget(Clear, area);
        let (title, text) = match dialog {
            PlaylistDialog::Actions { playlist, selected } => {
                let rows = ACTIONS
                    .iter()
                    .enumerate()
                    .map(|(i, label)| {
                        format!(
                            "{} {}{}",
                            if i == *selected { "›" } else { " " },
                            label,
                            if i == 4 && !playlist.can_edit {
                                " (read-only)"
                            } else {
                                ""
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (
                    " PLAYLIST ACTIONS ",
                    format!(
                        "{}\n\n{rows}\n\n↑/↓ choose · Enter confirm · Esc back",
                        playlist.title
                    ),
                )
            }
            PlaylistDialog::Name { text, .. } => (
                " NEW APPLE MUSIC PLAYLIST ",
                format!(
                    "Playlist name (max 200 UTF-8 bytes):\n\n{text}▏\n\nEnter reviews account write · Esc cancels\nNo account changes until the confirmation step."
                ),
            ),
            PlaylistDialog::Confirm { summary, .. } => (
                " CONFIRM ACCOUNT WRITE ",
                format!("{summary}\n\nQueue and audio remain unchanged."),
            ),
        };
        let block = Block::bordered()
            .title(title)
            .padding(Padding::horizontal(1))
            .style(Style::default().fg(p.fg).bg(p.bg));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let [body, footer] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(inner);
        f.render_widget(Paragraph::new(if matches!(dialog,PlaylistDialog::Confirm{..}){"Enter SEND to Apple · Esc CANCEL\nLost response: inspect Apple before a new operation."}else{"Enter confirms · Esc back"}).style(Style::default().fg(p.fg).bg(p.bg)),footer);
        f.render_widget(
            Paragraph::new(text)
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(p.fg).bg(p.bg)),
            body,
        );
    }
}
