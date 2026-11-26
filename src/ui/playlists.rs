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
pub(super) const TRACK_WINDOW: usize = 25;
const PREFETCH_MARGIN: usize = 5;

impl App {
    pub(super) fn playlist_tracks(&self) -> bool {
        self.kind.starts_with("open:library-playlists:")
    }

    pub(super) fn playlist_range(&self, height: usize) -> Range<usize> {
        let height = height.min(TRACK_WINDOW).min(self.items.len());
        if height == 0 {
            return 0..0;
        }
        let first = self.selected.saturating_add(1).saturating_sub(height);
        let start = self
            .playlist_top
            .clamp(first, self.selected)
            .min(self.items.len() - height);
        start..start + height
    }

    pub(super) fn update_playlist_top(&mut self) {
        if self.playlist_tracks() {
            let main = navigation::layout(self.viewport, self.screen).main;
            self.playlist_top = self
                .playlist_range(usize::from(self.catalog_areas(main)[1].height))
                .start;
        }
    }

    fn playlist_near_end(&self) -> bool {
        let main = navigation::layout(self.viewport, self.screen).main;
        let height = usize::from(self.catalog_areas(main)[1].height);
        if self.list_view.top.is_some() {
            self.list_range(height).end.saturating_add(PREFETCH_MARGIN) >= self.items.len()
        } else {
            self.selected.saturating_add(PREFETCH_MARGIN) >= self.items.len()
        }
    }

    pub(super) fn playlist_page(&self, requested: bool) -> Option<Request> {
        if !self.playlist_tracks()
            || self.browse_state == BrowseState::Loading
            || (!requested
                && (self.browse_state != BrowseState::Ready || !self.playlist_near_end()))
        {
            return None;
        }
        Some(Request::Browse {
            kind: self.kind.clone(),
            term: self.query.clone(),
            next: Some(self.next.clone()?),
        })
    }

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
            account_generation: self.account_generation,
            action,
        }
    }
    pub(super) fn playlist_name(&mut self, queue: bool) {
        self.dialog_scroll = 0;
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
        self.dialog_scroll = 0;
        self.playlist_dialog = Some(PlaylistDialog::Confirm {
            request: Box::new(Request::PlaylistWrite {
                key: ipc::operation_key(),
                account_generation: self.account_generation,
                source,
                action,
            }),
            summary,
        });
    }
    pub(super) fn playlist_destination(&mut self, source: PlaylistWriteSource) -> Request {
        self.remember_view();
        self.playlist_target = Some(source);
        self.go(Screen::Playlists)
            .expect("playlist screen requests metadata")
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
                    self.dialog_scroll = usize::MAX;
                    Some(PlaylistDialog::Name { source, text })
                }
                KeyCode::Char(c) if !c.is_control() && text.len() + c.len_utf8() <= 200 => {
                    text.push(c);
                    self.dialog_scroll = usize::MAX;
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
            self.dialog_scroll = usize::MAX;
            for c in text.chars().filter(|c| !c.is_control()) {
                if entry.len() + c.len_utf8() > 200 {
                    break;
                }
                entry.push(c);
            }
        }
    }
    pub(super) fn playlist_dialog_area(area: Rect) -> Rect {
        scroll::centered(area, 78, 18)
    }

    fn playlist_dialog_inner(area: Rect) -> Rect {
        Block::bordered()
            .padding(Padding::horizontal(1))
            .inner(area)
    }

    fn playlist_form(dialog: &PlaylistDialog) -> (&'static str, String) {
        match dialog {
            PlaylistDialog::Name { text, .. } => (
                " NEW APPLE MUSIC PLAYLIST ",
                format!(
                    "Playlist name (max 200 UTF-8 bytes):\nEnter reviews account write · Esc cancels\nNo account changes until confirmation.\n\n{text}▏"
                ),
            ),
            PlaylistDialog::Confirm { summary, .. } => (
                " CONFIRM ACCOUNT WRITE ",
                format!(
                    "Enter SEND to Apple · Esc CANCEL\n\n{summary}\n\nQueue and audio remain unchanged.\nLost response: inspect Apple before a new operation."
                ),
            ),
            PlaylistDialog::Actions { .. } => unreachable!("actions use their own rows"),
        }
    }

    pub(super) fn scroll_playlist_dialog(&mut self, area: Rect, direction: Direction) {
        match &mut self.playlist_dialog {
            Some(PlaylistDialog::Actions { selected, .. }) => {
                *selected = direction.step(*selected, ACTIONS.len() - 1);
            }
            Some(dialog) => {
                let body = Self::playlist_dialog_inner(area);
                if !body.is_empty() {
                    let (_, text) = Self::playlist_form(dialog);
                    let widget = scroll::paragraph(text);
                    self.dialog_scroll =
                        scroll::text_offset(widget, body, self.dialog_scroll, direction);
                }
            }
            None => {}
        }
    }

    pub(super) fn render_playlist_dialog(&self, f: &mut Frame, p: Palette) {
        let Some(dialog) = &self.playlist_dialog else {
            return;
        };
        let area = Self::playlist_dialog_area(f.area());
        f.render_widget(Clear, area);
        let style = Style::default().fg(p.fg).bg(p.bg);
        let block = Block::bordered()
            .padding(Padding::horizontal(1))
            .style(style);
        let inner = block.inner(area);
        if let PlaylistDialog::Actions { playlist, selected } = dialog {
            f.render_widget(block.title(" PLAYLIST ACTIONS "), area);
            let [title, actions, hint] = scroll::menu_rows(inner);
            f.render_widget(Paragraph::new(playlist.title.as_str()), title);
            let range = visible_range(*selected, ACTIONS.len(), actions.height as usize);
            let rows = range.map(|i| {
                let marker = if i == *selected { "›" } else { " " };
                let suffix = if i == 4 && !playlist.can_edit {
                    " (read-only)"
                } else {
                    ""
                };
                ListItem::new(format!("{marker} {}{suffix}", ACTIONS[i]))
            });
            f.render_widget(List::new(rows), actions);
            scroll::indicator(f, actions, *selected, ACTIONS.len() - 1, p);
            f.render_widget(
                Paragraph::new("↑/↓ or wheel · Enter confirm · Esc back"),
                hint,
            );
        } else {
            let (title, text) = Self::playlist_form(dialog);
            f.render_widget(block.title(title), area);
            let widget = scroll::paragraph(text).style(style);
            scroll::render_text(f, widget, inner, self.dialog_scroll, p);
        }
    }
}
