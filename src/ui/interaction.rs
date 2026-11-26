//! Keyboard actions and focused menus; no network or credential handling.
use super::*;
pub(super) struct BrowseView {
    album_view: Option<crate::catalog::album::AlbumRef>,
    playlist_view: Option<crate::catalog::playlist::PlaylistRef>,
    playlist_top: usize,
    list_view: scroll::ListViewport,
    playlist_bytes: usize,
    screen: Screen,
    browse_state: BrowseState,
    browse_retry: Option<Request>,
    inspected_track: Option<Track>,
    lyrics_scroll: Option<usize>,
    note: String,
    search_type: usize,
    catalog_context: String,
    kind: String,
    query: String,
    items: Vec<Track>,
    next: Option<String>,
    previous: Option<String>,
    selected: usize,
    queue_offset: usize,
    queue_revision: Option<u64>,
    catalog_offset: usize,
}
impl BrowseView {
    fn private(&self) -> bool {
        crate::catalog::playlist::private_kind(&self.kind)
            || self
                .inspected_track
                .as_ref()
                .is_some_and(|track| track.kind == "library-songs")
    }
}
pub(super) struct SongActions {
    track: Track,
    queue_index: Option<usize>,
    revision: u64,
    entry_id: Option<u64>,
    selected: usize,
    /// Page request that, once it actually applies the wheel-initiated shift,
    /// closes this menu; failures, retries and view changes keep it open.
    pending_scroll: Option<Box<Request>>,
}
impl SongActions {
    fn labels(&self) -> &'static [&'static str] {
        if self.queue_index.is_some() {
            &[
                "Lyrics",
                "Play",
                "Remove and advance (if current)",
                "Move up",
                "Move down",
                "Add song to Apple playlist…",
            ]
        } else {
            &[
                "Lyrics",
                "Append to end",
                "Play song — replaces queue",
                "Play next (FIFO)",
                "Add song to Apple playlist…",
            ]
        }
    }
}
impl App {
    pub(super) fn private_views_present(&self) -> bool {
        crate::catalog::playlist::private_kind(&self.kind)
            || self.playlist_dialog.is_some()
            || self
                .inspected_track
                .as_ref()
                .is_some_and(|track| track.kind == "library-songs")
            || self.back_stack.iter().any(BrowseView::private)
    }

    pub(super) fn trim_playlist_history(&mut self, current: usize) {
        let allowance = crate::catalog::album::MAX_ALBUM_BYTES.saturating_sub(current);
        let mut stored: usize = self.back_stack.iter().map(|view| view.playlist_bytes).sum();
        while stored > allowance {
            let Some(index) = self
                .back_stack
                .iter()
                .position(|view| view.playlist_bytes != 0)
            else {
                break;
            };
            stored -= self.back_stack.remove(index).playlist_bytes;
        }
    }

    pub(super) fn invalidate_private_views(&mut self) {
        self.back_stack.retain(|view| !view.private());
        self.playlist_dialog = None;
        self.playlist_target = None;
        let private_browse = crate::catalog::playlist::private_kind(&self.kind);
        if private_browse {
            if self.playlist_tracks() {
                self.kind = "playlists".into();
            }
            self.query.clear();
            self.text.clear();
            self.input = Input::None;
            self.catalog_context.clear();
            self.items.clear();
            self.next = None;
            self.previous = None;
            self.selected = 0;
            self.catalog_offset = 0;
            self.playlist_view = None;
            self.playlist_top = 0;
            self.list_view = Default::default();
            self.detail_scroll = 0;
            self.empty_scroll = 0;
            self.playlist_bytes = 0;
            self.browse_state = BrowseState::Idle;
            self.browse_retry = None;
            self.song_actions = None;
            self.note = "Account changed; reopen the library or playlist".into();
        }
        if self
            .inspected_track
            .as_ref()
            .is_some_and(|track| track.kind == "library-songs")
        {
            self.inspected_track = None;
            self.lyrics_result = None;
            self.lyric_layout.clear();
            self.lyrics_scroll = None;
        }
    }

    pub(super) fn remember_view(&mut self) {
        if self.back_stack.len() == 8 {
            self.back_stack.remove(0);
        }
        self.back_stack.push(BrowseView {
            album_view: self.album_view.clone(),
            playlist_view: self.playlist_view.clone(),
            playlist_top: self.playlist_top,
            list_view: self.list_view.clone(),
            playlist_bytes: self.playlist_bytes,
            screen: self.screen,
            browse_state: self.browse_state,
            browse_retry: self.browse_retry.clone(),
            inspected_track: self.inspected_track.clone(),
            lyrics_scroll: self.lyrics_scroll,
            note: self.note.clone(),
            search_type: self.search_type,
            catalog_context: self.catalog_context.clone(),
            kind: self.kind.clone(),
            query: self.query.clone(),
            items: if self.screen == Screen::Queue {
                Vec::new()
            } else {
                std::mem::take(&mut self.items)
            },
            next: self.next.clone(),
            previous: self.previous.clone(),
            selected: self.selected,
            queue_offset: self.queue_offset,
            queue_revision: self.queue_view_revision,
            catalog_offset: self.catalog_offset,
        });
        self.playlist_bytes = 0;
    }
    pub(super) fn restore_view(&mut self) -> bool {
        let Some(view) = self.back_stack.pop() else {
            return false;
        };
        self.playlist_target = None;
        self.album_view = view.album_view;
        self.playlist_view = view.playlist_view;
        self.playlist_top = view.playlist_top;
        self.list_view = view.list_view;
        // A restored view re-fetches its pending pages: the pre-leave wheel
        // shift intent must not consume the restored reply and move rows.
        self.list_view.stop_append();
        self.list_view.forget_placement();
        self.rows_revision = self.rows_revision.wrapping_add(1);
        self.nav_top = None;
        self.detail_scroll = 0;
        self.empty_scroll = 0;
        self.queue_request = None;
        self.queue_retry = None;
        self.playlist_bytes = view.playlist_bytes;
        self.screen = view.screen;
        self.browse_state = view.browse_state;
        self.search_type = view.search_type;
        self.catalog_context = view.catalog_context;
        self.browse_retry = view.browse_retry;
        self.note = view.note;
        self.nav = view.screen.nav_index();
        self.kind = view.kind;
        self.query = view.query;
        self.items = if view.screen == Screen::Queue {
            if view.queue_revision != self.status.as_ref().map(|status| status.queue_revision) {
                self.list_view = Default::default();
            }
            self.list_view.forget_placement();
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
        self.focus = Focus::Content;
        self.focus = Focus::Content;
        self.inspected_track = view.inspected_track;
        self.lyrics_scroll = view.lyrics_scroll;
        self.restore_lyrics_scroll = self.screen == Screen::Lyrics;
        self.input = Input::None;
        true
    }

    pub(super) fn checked_queue_edit(&self, index: usize, to: Option<usize>) -> Option<Request> {
        use crate::ipc::QueueAction;
        let row = self.queue_rows.get(index.checked_sub(self.queue_offset)?)?;
        let revision = self
            .queue_view_revision
            .filter(|revision| self.queue_revision_valid(*revision))?;
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
    fn queue_revision_valid(&self, revision: u64) -> bool {
        self.connected
            && self
                .status
                .as_ref()
                .is_none_or(|status| status.queue_revision == revision)
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
    pub(super) fn song_menu_area(&self, area: Rect) -> Option<Rect> {
        self.song_actions
            .as_ref()
            .map(|menu| scroll::centered(area, 80, menu.labels().len() as u16 + 6))
    }

    pub(super) fn scroll_song_menu(&mut self, direction: Direction) {
        if let Some(menu) = &mut self.song_actions {
            menu.selected = direction.step(menu.selected, menu.labels().len() - 1);
        }
    }

    pub(super) fn open_song_actions(&mut self, track: Track) {
        if self.screen == Screen::Queue
            && (!self.connected || self.queue_rows.get(self.selected).is_none())
        {
            return;
        }
        self.song_actions = Some(SongActions {
            track,
            entry_id: if self.screen == Screen::Queue {
                self.queue_rows.get(self.selected).map(|e| e.entry_id)
            } else {
                None
            },
            revision: self
                .queue_view_revision
                .or_else(|| self.status.as_ref().map(|status| status.queue_revision))
                .unwrap_or(0),
            queue_index: (self.screen == Screen::Queue)
                .then_some(self.queue_offset + self.selected),
            selected: 0,
            pending_scroll: None,
        });
    }
    /// Bind the wheel-initiated close to the page application that completes
    /// the shift: a newly issued request first, else the one already in
    /// flight. Without any pending page the rows already moved, so the menu
    /// closes now.
    pub(super) fn defer_menu_scroll(&mut self, request: Option<Request>) {
        let bound = request.or_else(|| {
            if self.browse_state == BrowseState::Loading {
                self.browse_retry.clone()
            } else if self.screen == Screen::Queue {
                self.queue_request.map(|offset| self.queue_page(offset))
            } else {
                None
            }
        });
        match bound.map(Box::new) {
            Some(bound) => {
                if let Some(menu) = self.song_actions.as_mut() {
                    menu.pending_scroll = Some(bound);
                }
            }
            None => self.song_actions = None,
        }
    }
    /// Close the open menu only when this applied page is the very shift the
    /// wheel that opened the intent started.
    pub(super) fn close_menu_after_scroll(&mut self, applied: &Request) {
        if let Some(menu) = self.song_actions.as_ref()
            && let Some(pending) = menu.pending_scroll.as_ref()
            && Self::page_request_matches(pending, applied)
        {
            self.song_actions = None;
        }
    }
    pub(super) fn queue_page(&self, offset: usize) -> Request {
        Request::QueuePage {
            offset,
            annotated: true,
        }
    }
    fn page_request_matches(pending: &Request, applied: &Request) -> bool {
        match (pending, applied) {
            (
                Request::Browse {
                    kind: pending_kind,
                    term: pending_term,
                    next: pending_next,
                },
                Request::Browse { kind, term, next },
            ) => pending_kind == kind && pending_term == term && pending_next == next,
            (
                Request::QueuePage {
                    offset: pending, ..
                },
                Request::QueuePage { offset, .. },
            ) => pending == offset,
            _ => false,
        }
    }
    pub(super) fn song_menu_key(&mut self, key: KeyCode) -> (bool, Option<Request>) {
        if self.song_actions.is_none() {
            return (false, None);
        }
        match key {
            KeyCode::Esc | KeyCode::Left => {
                self.song_actions = None;
            }
            KeyCode::Up | KeyCode::Char('k') => self.scroll_song_menu(Direction::Up),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_song_menu(Direction::Down),
            // Keep the page retry reachable while the menu is open: a pending
            // page close waits for its application, so a failed fetch must be
            // retryable without leaving the menu first.
            KeyCode::Char('R') => {
                let request = if self.screen == Screen::Queue {
                    self.queue_retry.map(|offset| self.queue_page(offset))
                } else if self.browse_state == BrowseState::Failed {
                    self.browse_retry.clone()
                } else {
                    None
                };
                return (false, request);
            }
            KeyCode::Enter | KeyCode::Right => {
                let menu = self.song_actions.take().expect("song menu exists");
                if menu.queue_index.is_some()
                    && (!self.queue_revision_valid(menu.revision)
                        || self
                            .queue_view_revision
                            .is_some_and(|revision| revision != menu.revision))
                {
                    self.note = "Queue changed; select the song again".into();
                    return (false, None);
                }
                if menu.selected == 0 {
                    self.remember_view();
                    self.inspected_track = Some(menu.track);
                    self.go(Screen::Lyrics);
                    self.note = "Lyrics preview · playback unchanged".into();
                } else if menu.queue_index.is_some()
                    && self.queue_view_revision != Some(menu.revision)
                {
                    self.note = "Refresh Queue before editing".into();
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
                if self.token_stored {
                    "saved locally"
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
            format!(
                "Pixelated artwork: {}",
                if self.config.artwork_style == crate::config::ArtworkStyle::Pixelated {
                    "on"
                } else {
                    "off"
                }
            ),
            "Equalizer".into(),
            "Audio information".into(),
            "Help".into(),
            "Quit".into(),
        ]
    }
    pub(super) fn settings_action(&mut self) -> (bool, Option<Request>) {
        let request = match self.selected {
            0 => {
                self.dialog_scroll = 0;
                self.token_input = Some(TokenInput::default());
                None
            }
            1 => {
                self.auth_job = Some(LocalAuth::Remove);
                None
            }
            2 => {
                let themes = [
                    "graphite",
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
            10 => Some(Request::ArtworkStyle(match self.config.artwork_style {
                crate::config::ArtworkStyle::Pixelated => crate::config::ArtworkStyle::Smooth,
                crate::config::ArtworkStyle::Smooth => crate::config::ArtworkStyle::Pixelated,
            })),
            11 => {
                self.go(Screen::Equalizer);
                None
            }
            12 => {
                self.go(Screen::AudioInfo);
                None
            }
            13 => {
                self.go(Screen::Help);
                None
            }
            14 => return (true, None),
            _ => None,
        };
        (false, request)
    }
    pub(super) fn render_settings(&self, f: &mut Frame, area: Rect, p: Palette) {
        let [heading, body] = self.settings_areas(area);
        f.render_widget(
            Paragraph::new(concat!(
                "SETTINGS · Rustune v",
                env!("CARGO_PKG_VERSION"),
                "\n",
                env!("CARGO_PKG_REPOSITORY"),
                "\n↑/↓ choose · Enter/→ change · ←/Esc menu"
            )),
            heading,
        );
        let rows = self.settings_rows();
        let range = self.settings_range(body, rows.len());
        let start = range.start;
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
        scroll::indicator(
            f,
            body,
            start,
            rows.len().saturating_sub(body.height as usize),
            p,
        );
    }
    pub(super) fn render_song_menu(&self, f: &mut Frame, p: Palette) {
        let Some(menu) = self.song_actions.as_ref() else {
            return;
        };
        let area = self.song_menu_area(f.area()).expect("song menu exists");
        f.render_widget(Clear, area);
        let block = Block::bordered()
            .title(" SONG ACTIONS ")
            .style(Style::default().fg(p.fg).bg(p.bg))
            .padding(Padding::horizontal(1));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let [title, actions, hint] = scroll::menu_rows(inner);
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
        scroll::indicator(f, actions, menu.selected, menu.labels().len() - 1, p);
        f.render_widget(Paragraph::new("↑/↓ or wheel · Enter confirm · Esc back\nPlay uses your saved token and the native CDM"),hint);
    }
}
