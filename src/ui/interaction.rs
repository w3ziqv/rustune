//! Keyboard actions and focused menus; no network or credential handling.
use super::*;
pub(super) struct BrowseView {
    album_view: Option<crate::catalog::album::AlbumRef>,
    playlist_view: Option<crate::catalog::playlist::PlaylistRef>,
    screen: usize,
    kind: String,
    query: String,
    items: Vec<Track>,
    next: Option<String>,
    previous: Option<String>,
    selected: usize,
    queue_offset: usize,
    catalog_offset: usize,
}
pub(super) struct SongActions {
    track: Track,
    queue_index: Option<usize>,
    revision: u64,
    entry_id: Option<u64>,
    selected: usize,
}
impl SongActions {
    fn labels(&self) -> &'static [&'static str] {
        if self.queue_index.is_some() {
            &[
                "Lyrics and cover",
                "Play",
                "Remove and advance (if current)",
                "Move up",
                "Move down",
                "Add song to Apple playlist…",
            ]
        } else {
            &[
                "Lyrics and cover",
                "Append to end",
                "Play song — replaces queue",
                "Play next (FIFO)",
                "Add song to Apple playlist…",
            ]
        }
    }
}
impl App {
    pub(super) fn remember_view(&mut self) {
        if self.back_stack.len() == 8 {
            self.back_stack.remove(0);
        }
        self.back_stack.push(BrowseView {
            album_view: self.album_view.clone(),
            playlist_view: self.playlist_view.clone(),
            screen: self.screen,
            kind: self.kind.clone(),
            query: self.query.clone(),
            items: self.items.clone(),
            next: self.next.clone(),
            previous: self.previous.clone(),
            selected: self.selected,
            queue_offset: self.queue_offset,
            catalog_offset: self.catalog_offset,
        });
    }
    pub(super) fn restore_view(&mut self) -> bool {
        let Some(view) = self.back_stack.pop() else {
            return false;
        };
        self.playlist_target = None;
        self.album_view = view.album_view;
        self.playlist_view = view.playlist_view;
        self.screen = view.screen;
        self.nav = view.screen;
        self.kind = view.kind;
        self.query = view.query;
        self.items = if view.screen == 7 {
            self.queue_view_revision = None;
            Vec::new()
        } else {
            view.items
        };
        self.next = view.next;
        self.previous = view.previous;
        self.selected = view.selected;
        self.queue_offset = view.queue_offset;
        self.catalog_offset = view.catalog_offset;
        self.sidebar = false;
        self.control_focus = None;
        self.inspected_track = None;
        self.input = Input::None;
        true
    }

    pub(super) fn checked_queue_edit(&self, index: usize, to: Option<usize>) -> Option<Request> {
        use crate::ipc::QueueAction;
        let row = self.queue_rows.get(index.checked_sub(self.queue_offset)?)?;
        let revision = self.queue_view_revision?;
        let action = if let Some(to) = to {
            QueueAction::MoveBy {
                entry_id: row.entry_id,
                delta: if to > index { 1 } else { -1 },
            }
        } else {
            QueueAction::RemoveAndAdvance {
                entry_id: row.entry_id,
            }
        };
        Some(ipc::queue_action(revision, action))
    }
    pub(super) fn close_queue_actions(&mut self) {
        if self
            .song_actions
            .as_ref()
            .is_some_and(|menu| menu.queue_index.is_some())
        {
            self.song_actions = None;
        }
    }
    pub(super) fn open_song_actions(&mut self, track: Track) {
        self.song_actions = Some(SongActions {
            track,
            entry_id: if self.screen == 7 {
                self.queue_rows.get(self.selected).map(|e| e.entry_id)
            } else {
                None
            },
            revision: self
                .queue_view_revision
                .or_else(|| self.status.as_ref().map(|status| status.queue_revision))
                .unwrap_or(0),
            queue_index: (self.screen == 7).then_some(self.queue_offset + self.selected),
            selected: 0,
        });
    }
    pub(super) fn song_menu_key(&mut self, key: KeyCode) -> (bool, Option<Request>) {
        let Some(menu) = self.song_actions.as_mut() else {
            return (false, None);
        };
        match key {
            KeyCode::Esc | KeyCode::Left => {
                self.song_actions = None;
            }
            KeyCode::Up | KeyCode::Char('k') => menu.selected = menu.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                menu.selected = (menu.selected + 1).min(menu.labels().len() - 1)
            }
            KeyCode::Enter | KeyCode::Right => {
                let menu = self.song_actions.take().expect("song menu exists");
                if menu.selected == 0 {
                    self.remember_view();
                    self.inspected_track = Some(menu.track);
                    self.go(8);
                    self.note = "Lyrics preview · playback unchanged".into();
                } else if menu.selected == if menu.queue_index.is_some() { 5 } else { 4 } {
                    let source = if let Some(entry_id) = menu.entry_id {
                        ipc::PlaylistWriteSource::Entry {
                            revision: menu.revision,
                            entry_id,
                        }
                    } else {
                        ipc::PlaylistWriteSource::Track {
                            track: Box::new(menu.track),
                        }
                    };
                    let request = self.playlist_destination(source);
                    return (false, Some(request));
                } else if let Some(index) = menu.queue_index {
                    let Some(entry_id) = menu.entry_id else {
                        self.note = "Refresh Queue before editing".into();
                        return (false, None);
                    };
                    let action = match menu.selected {
                        1 => Some(ipc::QueueAction::PlayEntry { entry_id }),
                        2 => Some(ipc::QueueAction::RemoveAndAdvance { entry_id }),
                        3 if index > 0 => Some(ipc::QueueAction::MoveBy {
                            entry_id,
                            delta: -1,
                        }),
                        4 => Some(ipc::QueueAction::MoveBy { entry_id, delta: 1 }),
                        _ => None,
                    };
                    return (false, action.map(|a| ipc::queue_action(menu.revision, a)));
                } else {
                    return (false, Some(self.song_request(menu.track, menu.selected)));
                }
            }
            _ => {}
        }
        (false, None)
    }
    pub(super) fn player_action(&mut self, index: usize) -> (bool, Option<Request>) {
        let volume = self
            .status
            .as_ref()
            .map_or(self.config.volume, |s| s.playback.volume);
        (
            index == 6,
            match index {
                0 => Some(Request::Toggle),
                1 => Some(Request::Next),
                2 => Some(Request::Previous),
                3 => Some(Request::Volume((volume + 0.05).min(1.0))),
                4 => Some(Request::Volume((volume - 0.05).max(0.0))),
                5 => Some(Request::Mute),
                _ => None,
            },
        )
    }
    pub(super) fn settings_rows(&self) -> Vec<String> {
        let status = self.status.as_ref();
        vec![
            format!(
                "Music User Token: {} · Enter to edit",
                if status.is_some_and(|s| s.playback.authorized) {
                    "Apple playback authorized"
                } else if self.token_stored {
                    "saved locally, unverified"
                } else {
                    "not saved"
                }
            ),
            "Remove saved token".into(),
            format!("Theme: {}", self.config.theme),
            format!(
                "Resources: {:?}",
                status.map_or(self.config.performance, |s| s.profile)
            ),
            format!(
                "Artwork: {}",
                if self.config.artwork { "on" } else { "off" }
            ),
            format!("Lyrics: {}", if self.config.lyrics { "on" } else { "off" }),
            format!(
                "Public catalog country: {}",
                status.map_or(self.config.catalog_country.as_str(), |s| s
                    .catalog_country
                    .as_str())
            ),
            format!("Queue shuffle: {}", status.is_some_and(|s| s.shuffle)),
            format!(
                "Queue repeat: {:?}",
                status.map_or(crate::model::Repeat::Off, |s| s.repeat)
            ),
            "Search history".into(),
            "Help".into(),
            "Quit".into(),
        ]
    }
    pub(super) fn settings_action(&mut self) -> (bool, Option<Request>) {
        let request = match self.selected {
            0 => {
                self.token_input = Some(TokenInput::default());
                None
            }
            1 => {
                self.auth_job = Some(LocalAuth::Remove);
                None
            }
            2 => {
                let themes = [
                    "monochrome",
                    "tokyo_night",
                    "nord",
                    "gruvbox",
                    "dark",
                    "light",
                    "catppuccin_mocha",
                    "catppuccin_latte",
                    "custom",
                ];
                let i = themes
                    .iter()
                    .position(|name| *name == self.config.theme)
                    .unwrap_or(0);
                Some(Request::Theme(themes[(i + 1) % themes.len()].into()))
            }
            3 => Some(Request::Profile(
                match self
                    .status
                    .as_ref()
                    .map_or(self.config.performance, |s| s.profile)
                {
                    PerformanceMode::Minimal => PerformanceMode::Balanced,
                    PerformanceMode::Balanced => PerformanceMode::Enhanced,
                    PerformanceMode::Enhanced => PerformanceMode::Minimal,
                },
            )),
            4 => Some(Request::Artwork(!self.config.artwork)),
            5 => Some(Request::Lyrics(!self.config.lyrics)),
            6 => {
                let countries = ["PL", "US", "GB", "DE", "FR", "JP"];
                let country = self
                    .status
                    .as_ref()
                    .map_or(self.config.catalog_country.as_str(), |s| {
                        s.catalog_country.as_str()
                    });
                let i = countries.iter().position(|c| *c == country).unwrap_or(0);
                Some(Request::CatalogCountry(
                    countries[(i + 1) % countries.len()].into(),
                ))
            }
            7 => Some(Request::Shuffle),
            8 => Some(Request::Repeat),
            9 => Some(Request::History),
            10 => {
                self.go(12);
                None
            }
            11 => return (true, None),
            _ => None,
        };
        (false, request)
    }
    pub(super) fn render_settings(&self, f: &mut Frame, area: Rect, p: Palette) {
        let [heading, body] =
            Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(area);
        f.render_widget(
            Paragraph::new("SETTINGS\n↑/↓ choose · Enter/→ change · ←/Esc menu"),
            heading,
        );
        let rows = self.settings_rows();
        let range = visible_range(self.selected, rows.len(), body.height as usize);
        let items = range
            .map(|i| {
                ListItem::new(format!(
                    "{} {}",
                    if i == self.selected { "›" } else { " " },
                    rows[i]
                ))
                .style(if i == self.selected {
                    Style::default().fg(p.fg).bg(p.bg).bold().underlined()
                } else {
                    Style::default().fg(p.fg)
                })
            })
            .collect::<Vec<_>>();
        f.render_widget(List::new(items), body);
    }
    pub(super) fn render_song_menu(&self, f: &mut Frame, p: Palette) {
        let Some(menu) = self.song_actions.as_ref() else {
            return;
        };
        let size = f.area();
        let width = size.width.min(80);
        let height = size.height.min(menu.labels().len() as u16 + 6);
        let area = Rect::new(
            size.x + (size.width - width) / 2,
            size.y + (size.height - height) / 2,
            width,
            height,
        );
        f.render_widget(Clear, area);
        let block = Block::bordered()
            .title(" SONG ACTIONS ")
            .style(Style::default().fg(p.fg).bg(p.bg))
            .padding(Padding::horizontal(1));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let [title, actions, hint] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(2),
        ])
        .areas(inner);
        f.render_widget(Paragraph::new(menu.track.title.as_str()), title);
        let range = visible_range(menu.selected, menu.labels().len(), actions.height as usize);
        f.render_widget(
            List::new(
                range
                    .map(|i| {
                        ListItem::new(format!(
                            "{} {}",
                            if i == menu.selected { "›" } else { " " },
                            menu.labels()[i]
                        ))
                        .style(if i == menu.selected {
                            Style::default().fg(p.fg).bg(p.bg).bold().underlined()
                        } else {
                            Style::default().fg(p.fg)
                        })
                    })
                    .collect::<Vec<_>>(),
            ),
            actions,
        );
        f.render_widget(Paragraph::new("↑/↓ choose · Enter confirm · Esc back\nPlay uses your saved token and the native CDM"),hint);
    }
}
