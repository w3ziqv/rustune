//! Event-driven terminal client. Only visible rows are constructed.
mod album;
mod equalizer;
#[cfg(test)]
mod flow_tests;
mod interaction;
#[cfg(test)]
mod keyboard_tests;
mod player;
mod playlists;
mod runtime;
use crate::{
    artwork::{self, CoverRequest, CoverResult},
    auth::{self, LocalAuth, TokenInput},
    config::{Config, Paths, PerformanceMode},
    input::{Events, InputEvent as Event},
    ipc::{self, Request},
    keys,
    lyrics::{self, LyricsQuery, LyricsRequest, LyricsResult},
    model::{Page, Track},
    service::{Reply, Status},
};
use anyhow::{Context, Result};
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, KeyCode, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect, Size},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, Padding, Paragraph, Wrap},
};
use std::{
    io::{IsTerminal, stdout},
    ops::Range,
};
use tokio::sync::mpsc;

pub const SCREENS: &[&str] = &[
    "Home",
    "Now Playing",
    "Search",
    "Library",
    "Albums",
    "Artists",
    "Playlists",
    "Queue",
    "Lyrics",
    "Equalizer",
    "Audio Settings",
    "Settings",
    "Help",
];
pub fn visible_range(selected: usize, len: usize, height: usize) -> Range<usize> {
    if len == 0 || height == 0 {
        return 0..0;
    }
    let start = selected
        .min(len - 1)
        .saturating_sub(height / 2)
        .min(len.saturating_sub(height));
    start..(start + height).min(len)
}
#[derive(Clone, Copy)]
struct Palette {
    bg: Color,
    fg: Color,
    muted: Color,
    accent: Color,
    panel: Color,
}
fn palette(c: &Config) -> Palette {
    let (bg, fg, muted, accent, panel) = match c.theme.as_str() {
        "light" | "catppuccin_latte" => (0xeff1f5, 0x4c4f69, 0x6c6f85, 0x8839ef, 0xe6e9ef),
        "gruvbox" => (0x282828, 0xebdbb2, 0xa89984, 0xfabd2f, 0x32302f),
        "nord" => (0x2e3440, 0xeceff4, 0x9caabc, 0x88c0d0, 0x3b4252),
        "monochrome" => (0x000000, 0xffffff, 0xffffff, 0xffffff, 0x000000),
        "tokyo_night" | "tokio" => (0x1a1b26, 0xc0caf5, 0x9aa5ce, 0x7aa2f7, 0x24283b),
        "dark" => (0x111318, 0xe6e6e6, 0x939ba8, 0x84c7b5, 0x1c2028),
        _ => (0x1e1e2e, 0xcdd6f4, 0x9399b2, 0xcba6f7, 0x313244),
    };
    let rgb = |n: u32| Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8);
    let custom = |name: &str, n: u32| {
        c.colors
            .get(name)
            .filter(|_| c.theme == "custom")
            .and_then(|v| u32::from_str_radix(v.trim_start_matches('#'), 16).ok())
            .map_or(rgb(n), rgb)
    };
    Palette {
        bg: custom("background", bg),
        fg: custom("foreground", fg),
        muted: custom("muted", muted),
        accent: custom("accent", accent),
        panel: custom("panel", panel),
    }
}
#[derive(Default, PartialEq)]
enum Input {
    #[default]
    None,
    Search,
    Command,
}
pub struct App {
    eq_selected: usize,
    eq_preset: usize,
    eq_pending: Option<rustune_core::equalizer::EqSettings>,
    eq_confirmed: rustune_core::equalizer::EqSettings,
    pub config: Config,
    pub status: Option<Status>,
    pub screen: usize,
    pub selected: usize,
    sidebar: bool,
    control_focus: Option<usize>,
    song_actions: Option<interaction::SongActions>,
    album_actions: Option<album::AlbumActions>,
    album_view: Option<crate::catalog::album::AlbumRef>,
    playlist_view: Option<crate::catalog::playlist::PlaylistRef>,
    playlist_dialog: Option<playlists::PlaylistDialog>,
    playlist_target: Option<ipc::PlaylistWriteSource>,
    queue_rows: Vec<crate::queue::QueueEntry>,
    queue_upcoming: Vec<u64>,
    nav: usize,
    items: Vec<Track>,
    next: Option<String>,
    previous: Option<String>,
    queue_offset: usize,
    queue_view_revision: Option<u64>,
    catalog_offset: usize,
    back_stack: Vec<interaction::BrowseView>,
    input: Input,
    text: String,
    query: String,
    catalog_context: String,
    kind: String,
    search_type: usize,
    note: String,
    busy: bool,
    cover: Option<CoverResult>,
    token_input: Option<TokenInput>,
    auth_job: Option<LocalAuth>,
    token_stored: bool,
    lyrics_result: Option<LyricsResult>,
    lyrics_generation: u64,
    lyrics_scroll: Option<usize>,
    viewport: Size,
    inspected_track: Option<Track>,
}
impl App {
    fn request_failed(&mut self, request: &Request) {
        if let Request::Equalizer(settings) = request
            && self.eq_pending.as_ref() == Some(settings)
        {
            self.eq_pending = None;
            self.config.equalizer = self.eq_confirmed.clone();
        }
    }
    pub fn new(config: Config) -> Self {
        let eq_confirmed = config.equalizer.clone();
        Self {
            eq_selected: 0,
            eq_preset: 0,
            eq_pending: None,
            eq_confirmed,
            config,
            status: None,
            screen: 1,
            selected: 0,
            sidebar: false,
            control_focus: None,
            song_actions: None,
            album_actions: None,
            album_view: None,
            playlist_view: None,
            playlist_dialog: None,
            playlist_target: None,
            queue_rows: Vec::new(),
            queue_upcoming: Vec::new(),
            nav: 1,
            items: Vec::new(),
            next: None,
            previous: None,
            queue_offset: 0,
            queue_view_revision: None,
            catalog_offset: 0,
            back_stack: Vec::new(),
            input: Input::None,
            text: String::new(),
            query: String::new(),
            catalog_context: "Apple Music catalog".into(),
            kind: "search:songs".into(),
            search_type: 0,
            note: "/ Search • ? Help • :audio Playback capabilities".into(),
            busy: false,
            cover: None,
            token_input: None,
            auth_job: None,
            token_stored: false,
            lyrics_result: None,
            lyrics_generation: 0,
            lyrics_scroll: None,
            viewport: Size::new(100, 30),
            inspected_track: None,
        }
    }
    pub fn artwork_request(&self, size: Size) -> Option<CoverRequest> {
        let profile = self
            .status
            .as_ref()
            .map_or(self.config.performance, |s| s.profile);
        if !self.config.artwork || profile.limits().artwork_entries == 0 {
            return None;
        }
        let cover_size = if matches!(self.screen, 1 | 8) && !self.sidebar {
            player::cover_size(self, size)?
        } else if !matches!(self.screen, 1 | 8) && size.width >= 130 && size.height >= 28 {
            artwork::COVER_SIZE
        } else {
            return None;
        };
        let url = self.displayed_track()?.artwork.as_ref()?.clone();
        let background = match palette(&self.config).bg {
            Color::Rgb(r, g, b) => [r, g, b],
            _ => [0, 0, 0],
        };
        Some(CoverRequest {
            size: cover_size,
            url,
            profile,
            protocol: self.config.artwork_protocol.clone(),
            background,
        })
    }
    pub fn accept_artwork(&mut self, cover: CoverResult) {
        self.cover = Some(cover);
    }
    pub fn lyrics_request(&self) -> Option<LyricsRequest> {
        if !self.config.lyrics || !matches!(self.screen, 1 | 8) {
            return None;
        }
        let mut query = LyricsQuery::from_track(self.displayed_track()?)?;
        query.generation = self.lyrics_generation;
        Some(LyricsRequest {
            query,
            profile: self
                .status
                .as_ref()
                .map_or(self.config.performance, |s| s.profile),
        })
    }
    pub fn accept_lyrics(&mut self, result: LyricsResult) {
        self.lyrics_result = Some(result);
    }
    fn on_status(&mut self, status: Status) -> Option<Request> {
        self.eq_confirmed = status.equalizer.clone();
        if self.eq_pending.as_ref() == Some(&status.equalizer) {
            self.eq_pending = None;
        }
        if self.eq_pending.is_none() {
            self.config.equalizer = status.equalizer.clone();
        }
        let changed = self.status.as_ref().is_none_or(|old| {
            old.queue_revision != status.queue_revision
                || old.queue_len != status.queue_len
                || old.current != status.current
                || old.pid != status.pid
        });
        if changed {
            self.close_queue_actions();
            if self.screen == 7 && self.queue_view_revision != Some(status.queue_revision) {
                self.items.clear();
                self.queue_view_revision = None;
            }
        }
        if let Some(write) = &status.playlist_write
            && self
                .status
                .as_ref()
                .and_then(|s| s.playlist_write.as_ref())
                .is_none_or(|old| old.id != write.id || old.phase != write.phase)
        {
            self.note = format!("Playlist write {}: {}", write.phase, write.message);
        }
        self.status = Some(status);
        (self.screen == 7 && changed).then(|| self.page_request())
    }
    fn displayed_track(&self) -> Option<&Track> {
        if self.screen == 8 {
            self.inspected_track
                .as_ref()
                .or_else(|| self.status.as_ref()?.playback.track.as_ref())
        } else if matches!(self.screen, 2..=7) {
            self.items.get(self.selected)
        } else {
            self.status.as_ref()?.playback.track.as_ref()
        }
    }
    fn lyrics_preview(&self) -> bool {
        self.screen == 8
            && self.inspected_track.as_ref().is_some_and(|t| {
                self.status
                    .as_ref()
                    .and_then(|s| s.playback.track.as_ref())
                    .is_none_or(|current| current.id != t.id)
            })
    }
    fn displayed_position(&self) -> f64 {
        if self.lyrics_preview() {
            0.0
        } else {
            self.status.as_ref().map_or(0.0, |s| s.playback.position)
        }
    }
    fn render_cover(&self, f: &mut Frame, area: Rect, p: Palette) {
        let area = Rect::new(area.x, area.y, 26, 14);
        f.render_widget(
            Block::bordered()
                .title(" ARTWORK ")
                .border_style(Style::default().fg(p.fg)),
            area,
        );
        let inner = Rect::new(area.x + 1, area.y + 1, 24, 12);
        let key = self.artwork_request(Size::new(f.area().width, f.area().height));
        let cover = self
            .cover
            .as_ref()
            .filter(|c| Some(&c.request) == key.as_ref());
        if let Some(image) = cover.and_then(|c| c.image.as_ref()) {
            f.render_widget(ratatui_image::Image::new(image), inner);
        } else {
            f.render_widget(
                Paragraph::new(if cover.is_some() {
                    "Artwork unavailable"
                } else {
                    "Loading artwork…"
                })
                .wrap(Wrap { trim: false }),
                inner,
            );
        }
    }
    fn page_request(&mut self) -> Request {
        if self.screen == 7 {
            if self
                .status
                .as_ref()
                .is_some_and(|s| self.queue_offset >= s.queue_len)
            {
                self.queue_offset = 0;
                self.selected = 0;
            }
            Request::QueuePage {
                offset: self.queue_offset,
            }
        } else {
            Request::Browse {
                kind: self.kind.clone(),
                term: self.query.clone(),
                next: if let Some(playlist) = self
                    .playlist_view
                    .as_ref()
                    .filter(|_| self.kind.starts_with("open:library-playlists:"))
                {
                    Some(format!("music:{}:0", playlist.storefront))
                } else {
                    self.album_view
                        .as_ref()
                        .filter(|_| self.kind.starts_with("open:albums:"))
                        .map(|album| format!("{}:{}:0", album.provider, album.storefront))
                },
            }
        }
    }
    fn go(&mut self, screen: usize) -> Option<Request> {
        if screen != 8 {
            self.inspected_track = None;
        }
        if screen != 8 && !(screen == 6 && self.playlist_target.is_some()) {
            self.back_stack.clear();
        }
        self.screen = screen;
        self.nav = screen;
        self.control_focus = None;
        self.song_actions = None;
        self.album_actions = None;
        self.album_view = None;
        self.playlist_view = None;
        self.playlist_dialog = None;
        if screen != 6 {
            self.playlist_target = None;
        }
        self.queue_rows.clear();
        self.selected = 0;
        self.items.clear();
        self.next = None;
        self.previous = None;
        self.sidebar = false;
        self.query.clear();
        self.queue_offset = 0;
        self.catalog_offset = 0;
        self.kind = match screen {
            4 => "search:albums",
            5 => "search:artists",
            3 => "library",
            6 => "playlists",
            _ => "search:songs",
        }
        .into();
        self.input = Input::None;
        if matches!(screen, 2 | 4 | 5) {
            self.search_type = if screen == 4 {
                1
            } else if screen == 5 {
                2
            } else {
                0
            };
            self.input = Input::Search;
            self.text.clear();
            self.note =
                "Public Apple catalog · type a query, Tab changes type, Enter searches".into();
        }
        matches!(screen, 3 | 6 | 7).then(|| self.page_request())
    }

    fn command(&mut self) -> Result<Option<Request>> {
        let words: Vec<_> = self.text.split_whitespace().collect();
        let Some(cmd) = words.first() else {
            return Ok(None);
        };
        let arg = words.get(1).copied().unwrap_or("");
        let req = match *cmd {
            "login" => {
                self.token_input = Some(TokenInput::default());
                return Ok(None);
            }
            "logout" => {
                self.auth_job = Some(LocalAuth::Remove);
                return Ok(None);
            }
            "play" => Request::Play,
            "pause" => Request::Pause,
            "next" => Request::Next,
            "previous" => Request::Previous,
            "mute" => Request::Mute,
            "clear" => ipc::queue_action(self.revision(), ipc::QueueAction::ClearUpcoming),
            "undo" => ipc::queue_action(self.revision(), ipc::QueueAction::Undo),
            "stop" => Request::StopAudio,
            "shutdown" => Request::Shutdown,
            "retry" => Request::RetryOperation,
            "cancel" => Request::CancelOperation(
                self.status
                    .as_ref()
                    .and_then(|s| s.operation.as_ref())
                    .context("No operation")?
                    .id
                    .clone(),
            ),
            "recover-state" => Request::RecoverState,
            "status" => Request::Status,
            "sleep" => Request::Sleep(
                arg.parse::<u64>()
                    .context("use :sleep MINUTES (0 cancels)")?
                    .checked_mul(60)
                    .context("timer too large")?,
            ),
            "profile" => Request::Profile(match arg {
                "minimal" => PerformanceMode::Minimal,
                "balanced" => PerformanceMode::Balanced,
                "enhanced" => PerformanceMode::Enhanced,
                _ => anyhow::bail!("use :profile minimal|balanced|enhanced"),
            }),
            "theme" => {
                let mut c = self.config.clone();
                c.theme = arg.into();
                Config::parse(&toml::to_string(&c)?)?;
                self.config = c;
                Request::Theme(arg.into())
            }
            "playlists" => return Ok(self.go(6)),
            "playlist-new" => {
                self.playlist_name(false);
                return Ok(None);
            }
            "playlist-save" => {
                self.playlist_name(true);
                return Ok(None);
            }
            "playlist-retry" => Request::RetryPlaylistWrite,
            "queue" => {
                return Ok(self.go(7));
            }
            "audio" => {
                self.go(10);
                return Ok(None);
            }
            "help" => {
                self.go(12);
                return Ok(None);
            }
            "recent" => {
                self.go(3);
                self.kind = "recent".into();
                self.note = "Recently played (authorized Apple API)".into();
                self.page_request()
            }
            "history" => Request::History,
            _ => anyhow::bail!("unknown command; :help lists available commands"),
        };
        Ok(Some(req))
    }
    fn key(&mut self, k: crossterm::event::KeyEvent) -> Result<(bool, Option<Request>)> {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            return Ok((true, None));
        }
        if let Some(input) = self.token_input.as_mut() {
            match k.code {
                KeyCode::Esc => {
                    self.token_input = None;
                    self.note = "Login cancelled; saved token unchanged.".into();
                }
                KeyCode::Enter => {
                    let input = self.token_input.take().expect("token input exists");
                    match input.finish() {
                        Ok(token) => {
                            self.auth_job = Some(LocalAuth::Save(token));
                            self.note = "Saving token privately…".into();
                        }
                        Err(_) => {
                            self.token_input = Some(TokenInput::default());
                            self.note="Invalid token. Paste one token, at most 16 KiB, without internal whitespace.".into();
                        }
                    }
                }
                KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => input.clear(),
                KeyCode::Backspace => input.backspace(),
                KeyCode::Char(c)
                    if !k
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    input.append(&c.to_string())
                }
                _ => {}
            }
            return Ok((false, None));
        }
        if self.playlist_dialog.is_some() {
            return Ok(self.playlist_menu_key(k.code));
        }
        if self.album_actions.is_some() {
            return Ok(self.album_menu_key(k.code));
        }
        if self.song_actions.is_some() {
            return Ok(self.song_menu_key(k.code));
        }
        if self.input != Input::None {
            match k.code {
                KeyCode::Esc => {
                    self.input = Input::None;
                    self.text.clear();
                }
                KeyCode::Backspace => {
                    self.text.pop();
                }
                KeyCode::Enter => {
                    let r = if self.input == Input::Search {
                        if self.text.trim().is_empty() {
                            self.note = "Enter a song, album or artist to search".into();
                            return Ok((false, None));
                        }
                        if !matches!(self.screen, 2 | 4 | 5) {
                            self.screen = 2;
                        }
                        self.nav = self.screen;
                        self.sidebar = false;
                        self.control_focus = None;
                        self.items.clear();
                        self.album_view = None;
                        self.playlist_view = None;
                        self.playlist_target = None;
                        self.next = None;
                        self.previous = None;
                        self.catalog_offset = 0;
                        self.back_stack.clear();
                        self.note = "Searching the public Apple catalog…".into();
                        self.selected = 0;
                        self.query = self.text.trim().into();
                        self.kind = format!(
                            "search:{}",
                            ["songs", "albums", "artists", "library-songs"][self.search_type]
                        );
                        Some(self.page_request())
                    } else {
                        self.command()?
                    };
                    self.input = Input::None;
                    self.text.clear();
                    return Ok((false, r));
                }
                KeyCode::Char(c)
                    if !k
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    if self.text.len() < 512 {
                        self.text.push(c);
                    }
                }
                KeyCode::F(2) | KeyCode::Tab | KeyCode::Up | KeyCode::Down
                    if self.input == Input::Search =>
                {
                    self.search_type = (self.search_type + 1) % 4
                }
                _ => {}
            }
            return Ok((false, None));
        }
        if self.input == Input::None && !self.sidebar {
            match k.code {
                KeyCode::Char('a') if matches!(self.screen, 2..=6) => {
                    if let Some(playlist) = self.selected_playlist() {
                        self.playlist_dialog = Some(playlists::PlaylistDialog::Actions {
                            playlist,
                            selected: 0,
                        });
                    } else {
                        self.open_album_actions();
                    }
                    return Ok((false, None));
                }
                KeyCode::Char('N') if self.screen == 6 => {
                    self.playlist_name(false);
                    return Ok((false, None));
                }
                KeyCode::Char('S') if matches!(self.screen, 6 | 7) => {
                    self.playlist_name(true);
                    return Ok((false, None));
                }
                KeyCode::Char('A') if self.screen == 6 => {
                    if let Some(playlist) = self.selected_playlist() {
                        if playlist.can_edit {
                            self.playlist_confirm(
                                ipc::PlaylistWriteSource::Queue {
                                    revision: self.revision(),
                                },
                                crate::catalog::playlist_write::WriteAction::Add { playlist },
                            );
                        } else {
                            self.note = "This playlist is read-only".into();
                        }
                    }
                    return Ok((false, None));
                }
                KeyCode::Char('u') if self.screen == 7 => {
                    return Ok((
                        false,
                        Some(ipc::queue_action(self.revision(), ipc::QueueAction::Undo)),
                    ));
                }
                KeyCode::Char('c') => {
                    if let Some(op) = self
                        .status
                        .as_ref()
                        .and_then(|s| s.operation.as_ref())
                        .filter(|o| o.phase == "preparing")
                    {
                        return Ok((false, Some(Request::CancelOperation(op.id.clone()))));
                    }
                }
                KeyCode::F(5) if self.screen == 6 => {
                    return Ok((false, Some(self.page_request())));
                }
                KeyCode::Char('R') if self.screen != 8 => {
                    return Ok((false, Some(Request::RetryOperation)));
                }
                _ => {}
            }
        }
        let action = keys::action_for(k, &self.config.keybindings);
        if self.screen == 9
            && k.modifiers.is_empty()
            && !self.sidebar
            && self.control_focus.is_none()
            && matches!(
                k.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::Enter
                    | KeyCode::Home
                    | KeyCode::Char('j' | 'k' | 'b' | 'r')
            )
        {
            return Ok((false, self.equalizer_key(k.code)));
        }
        if let Some(index) = self.control_focus {
            match action {
                Some("menu_left") => {
                    if index > 0 {
                        self.control_focus = Some(index - 1);
                    } else {
                        self.control_focus = None;
                        self.sidebar = true;
                        self.nav = self.screen;
                    }
                    return Ok((false, None));
                }
                Some("menu_right") => {
                    self.control_focus = Some((index + 1).min(6));
                    return Ok((false, None));
                }
                Some("select") => return Ok(self.player_action(index)),
                Some("up" | "down") => {
                    self.control_focus = None;
                    return Ok((false, None));
                }
                _ => {}
            }
        }
        if action == Some("down")
            && k.code == KeyCode::Down
            && matches!(self.screen, 1 | 8)
            && !self.sidebar
            && !matches!(
                self.lyrics_result.as_ref().map(|r| &r.state),
                Some(lyrics::LyricsState::Found(_))
            )
        {
            self.control_focus = Some(0);
            return Ok((false, None));
        }

        if matches!(action, Some("down" | "up")) && matches!(self.screen, 1 | 8) && !self.sidebar {
            if let Some(lyrics::LyricsState::Found(lyrics)) =
                self.lyrics_result.as_ref().map(|r| &r.state)
            {
                let area = player::lyrics_area(self, self.viewport);
                let rows = player::lyric_rows(lyrics, area.width);
                let height = usize::from(area.height.saturating_sub(1));
                let current = self
                    .lyrics_scroll
                    .unwrap_or_else(|| {
                        let active = lyrics
                            .active_line(self.displayed_position())
                            .and_then(|index| rows.iter().position(|(line, _)| *line == index))
                            .unwrap_or(0);
                        visible_range(active, rows.len(), height).start
                    })
                    .min(rows.len().saturating_sub(height));
                self.lyrics_scroll = Some(if action == Some("down") {
                    current
                        .saturating_add(1)
                        .min(rows.len().saturating_sub(height))
                } else {
                    current.saturating_sub(1)
                });
            }
            return Ok((false, None));
        }
        let mut req = None;
        match action {
            Some("lyrics_refresh") => {
                self.lyrics_generation = self.lyrics_generation.wrapping_add(1);
                self.lyrics_scroll = None;
            }
            Some("login") => self.token_input = Some(TokenInput::default()),
            Some("quit") => return Ok((true, None)),
            Some("toggle") => req = Some(Request::Toggle),
            Some("next") => req = Some(Request::Next),
            Some("previous") => req = Some(Request::Previous),
            Some("search") => {
                self.control_focus = None;
                self.sidebar = false;
                self.input = Input::Search;
                self.text = self.query.clone();
            }
            Some("command") => {
                self.input = Input::Command;
                self.text.clear();
            }
            Some("down") => {
                if self.sidebar {
                    self.nav = (self.nav + 1) % SCREENS.len();
                } else {
                    self.selected = (self.selected + 1).min(if self.screen == 11 {
                        self.settings_rows().len() - 1
                    } else {
                        self.items.len().saturating_sub(1)
                    });
                }
            }
            Some("up") => {
                if self.sidebar {
                    self.nav = self.nav.saturating_sub(1);
                } else {
                    self.selected = self.selected.saturating_sub(1);
                }
            }
            Some("focus") => {
                if self.sidebar {
                    self.sidebar = false;
                    if matches!(self.screen, 1 | 8) {
                        self.control_focus = Some(0);
                    }
                } else if self.control_focus.is_some() {
                    self.control_focus = None;
                } else {
                    self.sidebar = true;
                    self.nav = self.screen;
                }
            }
            Some("menu_left") if !self.sidebar && self.restore_view() => {
                if self.screen == 7 {
                    req = Some(self.page_request());
                }
            }
            Some("back" | "menu_left") => {
                self.control_focus = None;
                self.sidebar = true;
                self.nav = self.screen;
            }
            Some("menu_right") if !self.sidebar && matches!(self.screen, 1 | 8) => {
                self.control_focus = Some(0);
            }

            Some("select" | "menu_right") => {
                if self.sidebar {
                    req = self.go(self.nav);
                } else if self.screen == 11 {
                    return Ok(self.settings_action());
                } else if let Some(track) = self.items.get(self.selected).cloned() {
                    if track.kind == "library-playlists" && self.playlist_target.is_some() {
                        let playlist = self.selected_playlist().expect("playlist row");
                        if !playlist.can_edit {
                            self.note =
                                "This playlist is read-only; choose an editable destination".into();
                            return Ok((false, None));
                        }
                        let source = self.playlist_target.take().expect("destination source");
                        self.playlist_confirm(
                            source,
                            crate::catalog::playlist_write::WriteAction::Add { playlist },
                        );
                    } else if track.kind == "songs" || track.kind == "library-songs" {
                        self.open_song_actions(track);
                    } else {
                        self.remember_view();
                        self.catalog_offset = 0;
                        self.playlist_view = self.selected_playlist();
                        self.album_view = self.selected_album();
                        self.kind = format!("open:{}:{}", track.kind, track.id);
                        self.query.clear();
                        self.selected = 0;
                        self.items.clear();
                        self.next = None;
                        self.previous = None;
                        self.note = "Loading public catalog items…".into();
                        req = Some(self.page_request());
                    }
                }
            }
            Some("enqueue") | Some("play_next") => {
                if let Some(track) = self.items.get(self.selected).cloned() {
                    req = Some(if track.kind == "library-playlists" {
                        self.playlist_request(
                            self.selected_playlist().expect("playlist row"),
                            if action == Some("play_next") {
                                ipc::ContextAction::PlayNext
                            } else {
                                ipc::ContextAction::Append
                            },
                        )
                    } else if track.kind == "albums" {
                        self.album_request(
                            self.selected_album().expect("album selected"),
                            if action == Some("play_next") {
                                ipc::AlbumAction::PlayNext
                            } else {
                                ipc::AlbumAction::Append
                            },
                        )
                    } else {
                        self.song_request(track, if action == Some("play_next") { 3 } else { 1 })
                    });
                }
            }
            Some("remove") if self.screen == 7 => {
                req = self.checked_queue_edit(self.queue_offset + self.selected, None)
            }
            Some("move_up") if self.screen == 7 && self.queue_offset + self.selected > 0 => {
                let from = self.queue_offset + self.selected;
                req = self.checked_queue_edit(from, Some(from - 1));
                self.selected = self.selected.saturating_sub(1);
            }
            Some("move_down") if self.screen == 7 => {
                let from = self.queue_offset + self.selected;
                if self.status.as_ref().is_some_and(|s| from + 1 < s.queue_len) {
                    req = self.checked_queue_edit(from, Some(from + 1));
                    self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1));
                }
            }
            Some("page_back") => {
                if self.screen == 7 && self.queue_offset > 0 {
                    self.queue_offset = self
                        .queue_offset
                        .saturating_sub(self.config.performance.limits().page_size);
                    self.selected = 0;
                    self.items.clear();
                    self.queue_view_revision = None;
                    req = Some(self.page_request());
                } else if matches!(self.screen, 2..=6) {
                    let previous = self.previous.clone().or_else(|| {
                        (self.catalog_offset > 0).then(|| {
                            self.catalog_offset
                                .saturating_sub(self.config.performance.limits().page_size)
                                .to_string()
                        })
                    });
                    if let Some(previous) = previous {
                        req = Some(Request::Browse {
                            kind: self.kind.clone(),
                            term: self.query.clone(),
                            next: Some(previous),
                        });
                        self.selected = 0;
                    }
                }
            }
            Some("page") => {
                if self.screen == 7 {
                    if self
                        .status
                        .as_ref()
                        .is_some_and(|s| self.queue_offset + self.items.len() < s.queue_len)
                    {
                        self.queue_offset += self.items.len();
                        self.selected = 0;
                        self.items.clear();
                        self.queue_view_revision = None;
                        req = Some(self.page_request());
                    }
                } else if let Some(next) = self.next.clone() {
                    req = Some(Request::Browse {
                        kind: self.kind.clone(),
                        term: self.query.clone(),
                        next: Some(next),
                    });
                    self.selected = 0;
                }
            }
            Some("volume_up") | Some("volume_down") => {
                let v = self
                    .status
                    .as_ref()
                    .map_or(self.config.volume, |s| s.playback.volume);
                req = Some(Request::Volume(
                    (v + if action == Some("volume_up") {
                        0.05
                    } else {
                        -0.05
                    })
                    .clamp(0.0, 1.0),
                ));
            }
            Some("seek_back") | Some("seek_forward") => {
                if let Some(s) = &self.status {
                    req = Some(Request::Seek(
                        (s.playback.position
                            + if action == Some("seek_forward") {
                                10.0
                            } else {
                                -10.0
                            })
                        .clamp(0.0, s.playback.duration),
                    ));
                }
            }
            Some("shuffle") => req = Some(Request::Shuffle),
            Some("repeat") => req = Some(Request::Repeat),
            Some("mute") => req = Some(Request::Mute),
            Some("help") => {
                self.go(12);
            }
            Some("settings") => {
                self.go(11);
            }
            Some("equalizer") => {
                self.go(9);
            }
            Some("lyrics") => {
                if matches!(self.screen, 2..=7) {
                    self.remember_view();
                }
                self.inspected_track = if matches!(self.screen, 2..=7) {
                    self.items.get(self.selected).cloned()
                } else {
                    None
                };
                self.go(8);
                if self.lyrics_preview() {
                    self.note = "Lyrics preview for the selected song; playback unchanged.".into();
                }
            }
            Some("visualizer") => self.note = "Visualizer is not implemented in this build.".into(),
            _ => match k.code {
                KeyCode::Char(c @ '1'..='9') => {
                    req = self.go(c as usize - '1' as usize);
                }
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1))
                }
                _ => {}
            },
        }
        Ok((false, req))
    }
    fn accept(&mut self, r: &Request, value: serde_json::Value) -> Result<()> {
        if let Request::Equalizer(settings) = r
            && self.eq_pending.as_ref() != Some(settings)
            && self.config.equalizer != *settings
        {
            return Ok(());
        }
        match r {
            Request::Browse { kind, term, next } => {
                if kind != &self.kind || term != &self.query {
                    return Ok(());
                }
                self.catalog_offset = next
                    .as_deref()
                    .and_then(|n| n.rsplit(':').next()?.parse().ok())
                    .unwrap_or(0);
                let p: Page = serde_json::from_value(value)?;
                self.catalog_context = p.context.unwrap_or_else(|| "Apple/iTunes catalog".into());
                self.items = p.items;
                self.next = p.next;
                self.previous = p.previous;
                self.note = format!(
                    "{} results · {} · Enter actions/open · PgDn more",
                    self.items.len(),
                    self.catalog_context
                );
            }
            Request::QueuePage { offset } => {
                if self.screen != 7 || *offset != self.queue_offset {
                    return Ok(());
                }
                self.close_queue_actions();
                let revision = value["revision"].as_u64();
                if revision.is_some()
                    && self
                        .status
                        .as_ref()
                        .is_some_and(|status| Some(status.queue_revision) != revision)
                {
                    return Ok(());
                }
                self.queue_view_revision = revision;
                self.queue_rows = serde_json::from_value(value["items"].clone())?;
                self.queue_upcoming = value
                    .get("upcoming")
                    .map(|v| serde_json::from_value(v.clone()))
                    .transpose()?
                    .unwrap_or_default();
                self.items = self.queue_rows.iter().map(|e| e.track.clone()).collect();
                self.selected = self.selected.min(self.items.len().saturating_sub(1));
            }
            Request::History => {
                self.note = serde_json::from_value::<Vec<String>>(value)?.join("  ·  ")
            }
            Request::PlaylistWrite { .. } | Request::RetryPlaylistWrite => {
                // Subscription owns write progress. A delayed acknowledgement can be
                // older than a completed/uncertain event on the other socket.
                if self
                    .status
                    .as_ref()
                    .and_then(|s| s.playlist_write.as_ref())
                    .is_none_or(|w| w.phase == "preparing")
                {
                    self.note = "Account write accepted; waiting for service status".into();
                }
            }
            _ => {
                match r {
                    Request::Theme(theme) => self.config.theme = theme.clone(),
                    Request::Profile(mode) => self.config.performance = *mode,
                    Request::Artwork(enabled) => self.config.artwork = *enabled,
                    Request::Lyrics(enabled) => self.config.lyrics = *enabled,
                    Request::CatalogCountry(country) => {
                        self.config.catalog_country = country.clone()
                    }
                    Request::Equalizer(settings) => {
                        self.eq_confirmed = settings.clone();
                        if self.eq_pending.as_ref() == Some(settings) {
                            self.eq_pending = None;
                            self.config.equalizer = settings.clone();
                        }
                    }
                    _ => {}
                }
                if let Ok(s) = serde_json::from_value::<Status>(value) {
                    self.note = s.message.clone();
                    self.status = Some(s);
                }
            }
        }
        Ok(())
    }
    pub fn render(&self, f: &mut Frame) {
        self.render_base(f);
        self.render_song_menu(f, palette(&self.config));
        self.render_album_menu(f, palette(&self.config));
        self.render_playlist_dialog(f, palette(&self.config));
        if self.token_input.is_some() {
            let a = f.area();
            let w = a.width.min(70);
            let h = a.height.min(11);
            let modal = Rect::new(a.x + (a.width - w) / 2, a.y + (a.height - h) / 2, w, h);
            let p = palette(&self.config);
            f.render_widget(Clear, modal);
            let feedback = self
                .token_input
                .as_ref()
                .map(|input| input.feedback(32))
                .unwrap_or_default();
            f.render_widget(Paragraph::new(format!("Paste token (hidden)\n\nMusic User Token\n{feedback}\n\nEnter save · Esc cancel · Ctrl-U clear\n\nStorage is local; account acceptance is not verified."))
                .style(Style::default().fg(p.fg).bg(p.bg)).wrap(Wrap{trim:false})
                .block(Block::bordered().title(" LOGIN ").border_style(Style::default().fg(p.fg)).padding(Padding::horizontal(1))),modal);
        }
    }
    fn render_base(&self, f: &mut Frame) {
        let area = f.area();
        let p = palette(&self.config);
        f.render_widget(Block::new().style(Style::default().bg(p.bg).fg(p.fg)), area);
        if area.width < 20 || area.height < 4 {
            f.render_widget(Paragraph::new("Rustune • q quit"), area);
            return;
        }
        if matches!(self.screen, 1 | 8) && !self.sidebar && area.height < 8 {
            f.render_widget(
                Paragraph::new(format!("RUSTUNE\n{}", self.status_message())),
                area,
            );
            return;
        }
        if matches!(self.screen, 1 | 8) && !self.sidebar {
            player::render(self, f, p);
            return;
        }
        let [top, body, player, status] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Fill(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .areas(area);
        let mode = self
            .status
            .as_ref()
            .map_or(self.config.performance, |s| s.profile);
        let title = Line::from(vec![
            Span::styled(" RUSTUNE ", Style::default().fg(p.accent).bold()),
            Span::styled(
                format!("/ {}", SCREENS[self.screen]),
                Style::default().fg(p.fg),
            ),
            Span::styled(
                format!("    {mode:?}{}", if self.busy { " · working…" } else { "" }),
                Style::default().fg(p.muted),
            ),
        ]);
        f.render_widget(
            Paragraph::new(title).block(
                Block::new()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(p.fg)),
            ),
            top,
        );
        let (nav, main, detail) = if area.width >= 130 {
            let [a, b, c] = Layout::horizontal([
                Constraint::Length(20),
                Constraint::Fill(1),
                Constraint::Length(29),
            ])
            .areas(body);
            (Some(a), b, Some(c))
        } else if area.width >= 85 {
            let [a, b] =
                Layout::horizontal([Constraint::Length(20), Constraint::Fill(1)]).areas(body);
            (Some(a), b, None)
        } else {
            (None, body, None)
        };
        if let Some(nav) = nav {
            let rows: Vec<_> = SCREENS
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    ListItem::new(format!(
                        " {} {}",
                        if self.sidebar && self.nav == i {
                            ">"
                        } else if self.screen == i {
                            "•"
                        } else {
                            " "
                        },
                        s
                    ))
                    .style(
                        if (self.sidebar && self.nav == i) || self.screen == i {
                            Style::default().fg(p.accent).bg(p.panel)
                        } else {
                            Style::default().fg(p.muted)
                        },
                    )
                })
                .collect();
            f.render_widget(
                List::new(rows).block(
                    Block::new()
                        .borders(Borders::RIGHT)
                        .border_style(Style::default().fg(p.fg))
                        .padding(Padding::top(1)),
                ),
                nav,
            );
        }
        let content = Block::new().padding(Padding::new(2, 2, 1, 0)).inner(main);
        if self.sidebar && nav.is_none() {
            f.render_widget(
                Paragraph::new(format!(
                    "Navigation: {}\n ↑/↓ choose · Enter/→ open · ←/Esc menu",
                    SCREENS[self.nav]
                ))
                .style(Style::default().fg(p.accent))
                .wrap(Wrap { trim: false }),
                content,
            );
        } else {
            self.content(f, content, p);
        }
        if let Some(detail) = detail {
            let detail = if self.screen != 1
                && self
                    .artwork_request(Size::new(area.width, area.height))
                    .is_some()
            {
                self.render_cover(f, Rect::new(detail.x + 2, detail.y + 1, 26, 14), p);
                Rect::new(
                    detail.x,
                    detail.y + 15,
                    detail.width,
                    detail.height.saturating_sub(15),
                )
            } else {
                detail
            };
            let s = self.status.as_ref();
            let mut info = format!(
                "SESSION\n\n{}\n\nQueue: {} tracks\nRepeat: {:?}\nShuffle: {}\n\n{}\n\n:login   Save token\n:profile Change resources\n:sleep   Sleep timer\n\nTab  Switch focus\nj/k  Navigate\n?    Help",
                if s.is_some_and(|s| s.playback.authorized) {
                    "Apple Music authorized"
                } else {
                    "Account acceptance unverified"
                },
                s.map_or(0, |s| s.queue_len),
                s.map_or(crate::model::Repeat::Off, |s| s.repeat),
                s.is_some_and(|s| s.shuffle),
                if s.is_some_and(|s| s.playback.authorized) {
                    "Token accepted for native playback"
                } else if self.token_stored {
                    "Token saved locally · account unverified"
                } else {
                    "L  Enter Music User Token"
                }
            );
            if matches!(self.screen, 2 | 4 | 5) {
                let item = self.items.get(self.selected);
                info = format!(
                    "APPLE MUSIC\n\n{}\n\nEnter / →: actions or open\n←: previous results / menu\nPgUp / PgDn: pages\n\nStore link:\n{}\n\nChoose Play for native audio",
                    self.catalog_context,
                    item.and_then(|t| t.url.as_deref())
                        .unwrap_or("Select a result")
                );
            }
            f.render_widget(
                Paragraph::new(info)
                    .style(Style::default().fg(p.muted))
                    .block(
                        Block::new()
                            .borders(Borders::LEFT)
                            .border_style(Style::default().fg(p.fg))
                            .padding(Padding::new(2, 1, 1, 0)),
                    )
                    .wrap(Wrap { trim: false }),
                detail,
            );
        }
        let [trackbar, progress] =
            Layout::vertical([Constraint::Length(2), Constraint::Length(1)]).areas(player);
        let s = self.status.as_ref();
        let play = s.map(|s| &s.playback);
        let track = play.and_then(|s| s.track.as_ref());
        let label = track.map_or_else(
            || "No audio engine  ·  ← menu  ·  / public search".into(),
            |t| {
                format!(
                    "{}  {} — {}",
                    if play.is_some_and(|s| s.playing) {
                        "▶"
                    } else {
                        "Ⅱ"
                    },
                    t.title,
                    t.artist
                )
            },
        );
        let volume = play.map_or(self.config.volume, |s| s.volume);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {label}"), Style::default().fg(p.fg)),
                Span::styled(
                    format!("   vol {:.0}%", volume * 100.0),
                    Style::default().fg(p.accent),
                ),
            ]))
            .block(
                Block::new()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(p.fg)),
            ),
            trackbar,
        );
        let (position, duration) = play.map_or((0.0, 0.0), |s| (s.position, s.duration));
        let ratio = if duration > 0.0 {
            (position / duration).clamp(0.0, 1.0)
        } else {
            0.0
        };
        f.render_widget(
            Gauge::default()
                .ratio(ratio)
                .label(format!("{} / {}", time(position), time(duration)))
                .gauge_style(Style::default().fg(p.accent).bg(p.panel)),
            progress,
        );
        let text = if self
            .status
            .as_ref()
            .is_some_and(|s| s.audio_error.is_some())
        {
            self.status_message()
        } else if self.input != Input::None {
            format!(
                "{}{}{}",
                if self.input == Input::Command {
                    ":"
                } else {
                    "/ "
                },
                self.text,
                if self.input == Input::Search {
                    "  [Tab type · Enter search · Esc cancel]"
                } else {
                    "  [Enter run · Esc cancel]"
                }
            )
        } else {
            format!(" {}", self.status_message())
        };
        f.render_widget(
            Paragraph::new(text).style(Style::default().fg(p.accent).bg(p.panel)),
            status,
        );
    }
    fn content(&self, f: &mut Frame, a: Rect, p: Palette) {
        if self.screen == 9 {
            self.render_equalizer(f, a, p);
            return;
        }
        if self.screen == 11 {
            self.render_settings(f, a, p);
            return;
        }
        if self.screen == 8 {
            player::render_lyrics(self, f, a, p);
            return;
        }
        let a = if self.screen == 1
            && self
                .artwork_request(Size::new(f.area().width, f.area().height))
                .is_some()
        {
            self.render_cover(f, a, p);
            Rect::new(a.x + 28, a.y, a.width.saturating_sub(28), a.height)
        } else {
            a
        };
        if matches!(self.screen, 2..=7) {
            let title = if self.screen == 7 {
                format!(
                    "QUEUE · shuffle {} · repeat {:?} · next #{}\nEnter actions · PgUp/PgDn pages · :clear upcoming · u Undo · :stop",
                    if self.status.as_ref().is_some_and(|s| s.shuffle) {
                        "on"
                    } else {
                        "off"
                    },
                    self.status
                        .as_ref()
                        .map_or(crate::model::Repeat::Off, |s| s.repeat),
                    self.status
                        .as_ref()
                        .and_then(|s| s.next_entry_id)
                        .map_or("—".into(), |id| id.to_string())
                )
            } else if self.kind.starts_with("open:library-playlists:") {
                "PLAYLIST TRACKS · a Whole playlist actions · Enter Song actions".into()
            } else if self.screen == 6 {
                format!(
                    "{} · a Actions · N New · S Save queue · A Add queue · F5 Refresh",
                    if self.playlist_target.is_some() {
                        "CHOOSE EDITABLE DESTINATION"
                    } else {
                        "MY PLAYLISTS"
                    }
                )
            } else if self.kind.starts_with("open:albums:") {
                "ALBUM TRACKS · a Album actions (whole album) · Enter Song actions".into()
            } else if self.kind.starts_with("open:artists:") {
                "ARTIST ALBUMS".into()
            } else if matches!(self.screen, 2 | 4 | 5) {
                format!(
                    "SEARCH  {}  ·  {}",
                    ["songs", "albums", "artists", "My library"][self.search_type],
                    format_args!("{} · a Album actions", self.query)
                )
            } else {
                self.kind.to_uppercase()
            };
            let write = self
                .status
                .as_ref()
                .and_then(|s| s.playlist_write.as_ref())
                .filter(|_| matches!(self.screen, 6 | 7));
            let title = if let Some(write) = write {
                format!("{title}\nAccount write {}: {}", write.phase, write.message)
            } else {
                title
            };
            let [header, list] = Layout::vertical([
                Constraint::Length(if write.is_some() { 3 } else { 2 }),
                Constraint::Fill(1),
            ])
            .areas(a);
            f.render_widget(
                Paragraph::new(title).style(Style::default().fg(p.accent).bold()),
                header,
            );
            if self.items.is_empty() {
                f.render_widget(
                    Paragraph::new(if self.screen == 2 {
                        "Type a query and press Enter. Tab/F2 changes songs, albums, artists or My library."
                    } else {
                        if self.screen==3 {"MY LIBRARY · requires an accepted Music User Token. Search → My library searches your saved songs."}else{"No results loaded. Choose Search and type a query."}
                    })
                    .style(Style::default().fg(p.muted))
                    .wrap(Wrap { trim: false }),
                    list,
                );
                return;
            }
            let range = visible_range(self.selected, self.items.len(), list.height as usize);
            let rows: Vec<_> = range
                .map(|i| {
                    let t = &self.items[i];
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!(
                                "{} {:>3}  ",
                                if self.screen == 7
                                    && self.status.as_ref().and_then(|s| s.current_entry_id)
                                        == self.queue_rows.get(i).map(|e| e.entry_id)
                                {
                                    "▶"
                                } else if i == self.selected {
                                    "›"
                                } else {
                                    " "
                                },
                                i + 1 + self.queue_offset
                            ),
                            Style::default().fg(p.muted),
                        ),
                        Span::raw(if self.screen == 7 {
                            self.queue_rows
                                .get(i)
                                .map(|e| {
                                    if self.status.as_ref().and_then(|s| s.current_entry_id)
                                        == Some(e.entry_id)
                                    {
                                        "[now] ".into()
                                    } else if let Some(rank) =
                                        self.queue_upcoming.iter().position(|id| *id == e.entry_id)
                                    {
                                        format!("[next {}] ", rank + 1)
                                    } else {
                                        "[past] ".into()
                                    }
                                })
                                .unwrap_or_default()
                        } else {
                            String::new()
                        }),
                        Span::raw(format!(
                            "{}{}  ",
                            if t.kind == "library-playlists" {
                                format!(
                                    "{} [{}]",
                                    t.title,
                                    if t.can_edit == Some(true) {
                                        "editable"
                                    } else {
                                        "read-only"
                                    }
                                )
                            } else {
                                t.title.clone()
                            },
                            if self.screen == 7 {
                                self.queue_rows
                                    .get(i)
                                    .map(|e| {
                                        format!(
                                            " · [{} {} {}] #{}",
                                            e.context.provider,
                                            e.context.storefront,
                                            e.context.title,
                                            e.entry_id
                                        )
                                    })
                                    .unwrap_or_default()
                            } else {
                                String::new()
                            }
                        )),
                        Span::styled(
                            format!(
                                "{}  {}",
                                t.artist,
                                if t.duration_ms > 0 {
                                    time(t.duration_ms as f64 / 1000.0)
                                } else {
                                    String::new()
                                }
                            ),
                            Style::default().fg(p.muted),
                        ),
                    ]))
                    .style(if i == self.selected {
                        Style::default().bg(p.panel).fg(p.accent)
                    } else {
                        Style::default().fg(p.fg)
                    })
                })
                .collect();
            f.render_widget(List::new(rows), list);
            return;
        }
        let text=match self.screen {
            0=>"YOUR MUSIC. YOUR TERMINAL.\n\nApple Music catalog: search songs, albums and artists.\nAccount token selects your region and enables My library.\n↑/↓ and Enter navigate; ← opens the menu; Settings is keyboard actionable.\n\nNative playback uses your Music User Token and a local CDM.\nSearch → choose a song → Enter → Play.\nL enters a Music User Token; saving it does not verify account acceptance.\n\nTab switches navigation focus. 1–9 open screens.\n:theme monochrome | tokyo_night changes the palette.\n:profile minimal disables artwork and reduces caches.\n\nEXPERIMENTAL · See Audio Settings for available capabilities.".into(),

            1=>self.status.as_ref().and_then(|s|s.playback.track.as_ref()).map_or("Nothing playing. No audio quality has been reported.".into(),|t|format!("NOW PLAYING\n\n{}\n{}\n{}\n\n{}\n\nSpace  Play/pause\nAlt-← / →  Seek 10 seconds\n+ / -  Volume\nn / p  Next / previous\ns      Shuffle\nr      Repeat\nm      Mute\n\n:sleep MINUTES sets a sleep timer (0 cancels).",t.title,t.artist,t.album,self.status.as_ref().map(|s|s.playback.quality_label()).unwrap_or_default())),

            10=>format!("AUDIO SETTINGS\n\n● Auto               Backend chooses stream quality\n○ Data Saver         Unavailable\n○ Standard           Unavailable\n○ High Quality       Unavailable\n○ Lossless           Unavailable\n○ Hi-Res Lossless    Unavailable\n\nBackend: {}\n{}\n\nSource codec/rate/channels are reported above when active.\nSource bit depth    Unknown\nBuffer status       Device clock\n\nQuality is shown only when a native backend reports reliable source fields.\nPipeWire output format does not establish source quality.",self.status.as_ref().map_or("No playback process",|s|s.backend.as_str()),self.status.as_ref().map_or("Source quality unknown".into(),|s|s.playback.quality_label())),
            11=>format!("SETTINGS\n\nTheme: {}\nResources: {:?}\n\n:theme dark | light | catppuccin_mocha | catppuccin_latte\n:theme gruvbox | nord | monochrome | tokyo_night | custom\n:profile minimal | balanced | enhanced\n\n~/.config/rustune/config.toml\nKeybindings, custom colors, notifications, mouse\nand daemon behavior can be configured there.\n\n:login  :logout  :sleep MINUTES  :clear  :history\nDaemon mode keeps playback running when the TUI closes.",self.config.theme,self.status.as_ref().map_or(self.config.performance,|s|s.profile)),
            _=>{let mut s="HELP · DEFAULT BINDINGS\n\n".to_owned();for (action,key) in keys::DEFAULTS {s.push_str(&format!("{key:<12}{action}   "));if s.lines().last().is_some_and(|l|l.len()>65){s.push('\n');}}s.push_str("\n\n←/Esc menu · ↑/↓ choose · Enter/→ open. Tab cycles focus.\nAlbums: Enter opens; a opens whole-album actions. Songs: Enter opens actions. Settings: ↑/↓ then Enter changes a value.\nSearch: Tab/F2 changes songs/albums/artists/My library. PgUp/PgDn changes pages.\nEQ: e opens; ↑/↓ selects; ←/→ edits; b bypasses; r resets.\n← returns from an album or lyrics to previous results. Alt-←/→ seeks.\nQueue: Delete removes and advances; J/K reorder; :clear upcoming; u Undo.\nCommands: :stop :undo :cancel :retry :login :logout :profile MODE :theme NAME\n:sleep MINUTES :recent :history :queue :audio :help\nCtrl-C always exits. ~/.config/rustune/config.toml customizes bindings.");s}
        };
        f.render_widget(
            Paragraph::new(text)
                .style(Style::default().fg(p.fg))
                .wrap(Wrap { trim: false }),
            a,
        );
    }
}
fn time(s: f64) -> String {
    let n = s.max(0.0) as u64;
    format!("{}:{:02}", n / 60, n % 60)
}
pub fn render_smoke(f: &mut Frame) {
    App::new(Config::default()).render(f);
}

pub async fn run(config: Config, paths: Paths) -> Result<()> {
    anyhow::ensure!(
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        "TUI requires a terminal; use rustune --help for CLI commands"
    );
    let mut app = App::new(config.clone());
    app.token_stored = auth::has_user_token(&paths);
    app.status = Some(serde_json::from_value(
        ipc::request(&paths, &Request::Status).await?,
    )?);
    let (jobs, work) = mpsc::channel::<Request>(8);
    let (done, mut responses) = mpsc::channel(8);
    let worker_paths = paths.clone();
    let auth_paths = paths.clone();
    let (auth_jobs, mut auth_work) = mpsc::channel::<LocalAuth>(1);
    let (auth_done, mut auth_results) = mpsc::channel(1);
    let auth_worker = tokio::spawn(async move {
        while let Some(job) = auth_work.recv().await {
            let paths = auth_paths.clone();
            let result = tokio::task::spawn_blocking(move || {
                auth::apply(&paths, job)?;
                Ok::<_, anyhow::Error>(auth::has_user_token(&paths))
            })
            .await;
            let state = result.ok().and_then(Result::ok);
            if auth_done.send(state).await.is_err() {
                break;
            }
        }
    });
    let worker = tokio::spawn(runtime::requests(worker_paths, work, done));
    let (updates, mut status) = mpsc::channel::<Result<Status>>(1);
    let subscription = tokio::spawn(async move {
        let result = async {
            let mut stream = tokio::net::UnixStream::connect(paths.socket()).await?;
            ipc::write_frame(&mut stream, &Request::Subscribe).await?;
            loop {
                let b = ipc::read_frame(&mut stream).await?;
                let value = serde_json::from_slice::<Reply>(&b)?.value()?;
                let s = serde_json::from_value(value)?;
                if updates.send(Ok(s)).await.is_err() {
                    break;
                }
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if let Err(e) = result {
            let _ = updates.send(Err(e)).await;
        }
    });
    // Fullscreen pixels need their RGB foreground/background pairs, including
    // with the monochrome UI theme. Inherited NO_COLOR otherwise turns covers
    // into unrecognizable block glyphs. This only affects the TUI process.
    crossterm::style::force_color_output(true);
    let mut terminal = ratatui::init();
    let (artwork_jobs, mut covers, artwork_worker) = artwork::worker();
    let mut last_artwork = None;
    let (lyrics_jobs, mut lyric_results, lyrics_worker) = lyrics::worker();
    let mut last_lyrics = None;
    let result=async {
        if config.mouse { execute!(stdout(), EnableMouseCapture)?; }
        execute!(stdout(), crossterm::event::EnableBracketedPaste)?;
        let mut events=Events::new()?;
        let mut resize=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())?;
        loop {
            app.viewport=terminal.size()?;
            if let Some(job)=app.auth_job.take() && auth_jobs.try_send(job).is_err(){app.note="Token storage is busy; try login again.".into();}
            let request=app.artwork_request(terminal.size()?);
            if request!=last_artwork {
                last_artwork=request.clone();
                app.cover=None;
                let _=artwork_jobs.send(request);
            }
            let request=app.lyrics_request();if request!=last_lyrics {last_lyrics=request.clone();app.lyrics_result=None;app.lyrics_scroll=None;let _=lyrics_jobs.send(request);}
            terminal.draw(|f|app.render(f))?;
            tokio::select! {
                _=resize.recv()=>{},
                Some(result)=lyric_results.recv()=>{if Some(&result.request)==last_lyrics.as_ref(){app.accept_lyrics(result);}},
                Some(result)=auth_results.recv()=>match result{Some(saved)=>{app.token_stored=saved;app.note=if saved{auth::SAVED_MESSAGE.into()}else{"Local token removed; Apple account authorization unchanged.".into()};},None=>app.note="Could not update the private token file; check config-directory permissions.".into()},
                Some(cover)=covers.recv()=>{if Some(&cover.request)==last_artwork.as_ref(){app.accept_artwork(cover);}}
                event=events.next()=>{
                    let Some(event)=event else{break;};let event=event?;let mut request=None;
                    match event {
                        Event::Paste(text)=>{if let Some(input)=app.token_input.as_mut(){input.append(&text);}else if app.playlist_dialog.is_some(){app.playlist_paste(&text);}else if app.input!=Input::None && app.text.len()+text.len()<=512 {app.text.extend(text.chars().filter(|c|!c.is_control()));}},
                        Event::Overflow=>{if let Some(input)=app.token_input.as_mut(){input.reject_paste();}app.note="Paste exceeds 16 KiB or is invalid. Ctrl-U clears token input.".into();},
                        Event::Key(k) if k.is_press()=>match app.key(k){Ok((quit,r))=>{if quit{break;}request=r;},Err(e)=>{app.note=e.to_string();app.input=Input::None;}},
                        Event::Mouse(m) if app.token_input.is_none() && app.song_actions.is_none() && app.playlist_dialog.is_none()=>match m.kind {
                            MouseEventKind::ScrollDown=>app.selected=(app.selected+1).min(app.items.len().saturating_sub(1)),
                            MouseEventKind::ScrollUp=>app.selected=app.selected.saturating_sub(1),
                            MouseEventKind::Down(MouseButton::Left) if matches!(app.screen,1|8) && !app.sidebar=>{
                                let size=terminal.size()?;let a=player::areas(Rect::new(0,0,size.width,size.height),app.config.artwork);
                                if let Some(i)=player::buttons(a.controls).iter().position(|r|r.contains(ratatui::layout::Position::new(m.column,m.row))){
                                    app.control_focus=Some(i);let (quit,r)=app.player_action(i);if quit{break;}request=r;
                                }
                            },
                            MouseEventKind::Down(MouseButton::Left) if terminal.size()?.width>=85 && m.column<20 && m.row>=3=>{let i=(m.row-3) as usize;if i<SCREENS.len(){request=app.go(i);}},
                            _=>{}
                        },
                        _=>{}
                    }
                    if let Some(r)=request{match jobs.try_send(r){Ok(())=>app.busy=true,Err(error)=>{app.request_failed(&error.into_inner());app.note="Command queue is full; wait for the current request.".into();}}}

                }
                Some((r,result))=responses.recv()=>{
                    app.busy=false;
                    if let Request::Browse {kind,term,..}=&r && (kind!=&app.kind || term!=&app.query || !matches!(app.screen,2..=6)) {continue;}
                    if let Request::QueuePage {offset}=&r && (app.screen!=7 || *offset!=app.queue_offset) {continue;}
                    match result {Ok(v)=>{if let Err(e)=app.accept(&r,v){app.note=e.to_string();}
                    if app.screen==7 && !matches!(r,Request::QueuePage{..}){let _=jobs.try_send(app.page_request());}},Err(e)=>{app.request_failed(&r);app.note=e.to_string();}}

                }
                Some(update) = status.recv() => match update {
                    Ok(s) => {
                        if let Some(r) = app.on_status(s) && jobs.try_send(r).is_err() {
                            app.note = "Command queue is full; reopen Queue to refresh.".into();
                        }
                    },
                    Err(_) => app.note = "Service disconnected. Quit and reopen Rustune to reconnect.".into(),
                }
            }
        }Ok::<(),anyhow::Error>(())
    }.await;
    worker.abort();
    subscription.abort();
    artwork_worker.abort();
    lyrics_worker.abort();
    auth_worker.abort();
    let _ = execute!(stdout(), crossterm::event::DisableBracketedPaste);
    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

#[cfg(test)]
mod login_tests {
    use super::*;
    #[test]
    fn token_input_shows_dots_and_supports_editing_without_showing_the_secret() {
        let mut app = App::new(Config::default());
        let key = |c| crossterm::event::KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        app.key(key('L')).unwrap();
        for c in "secret".chars() {
            app.key(key(c)).unwrap();
        }
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        let mut rendered = |app: &App| {
            t.draw(|f| app.render(f)).unwrap();
            t.backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>()
        };
        let text = rendered(&app);
        assert!(text.contains("••••••") && text.contains("6 characters"));
        assert!(!text.contains("secret"));
        app.key(crossterm::event::KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::NONE,
        ))
        .unwrap();
        assert!(rendered(&app).contains("5 characters"));
        app.key(crossterm::event::KeyEvent::new(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL,
        ))
        .unwrap();
        let cleared = rendered(&app);
        assert!(cleared.contains("No token entered") && !cleared.contains("••"));
    }
    #[test]
    fn screens_do_not_offer_removed_browser_setup() {
        let mut app = App::new(Config::default());
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(142, 39)).unwrap();
        for screen in 0..SCREENS.len() {
            app.screen = screen;
            t.draw(|f| app.render(f)).unwrap();
            let text = t
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            for removed in ["Firefox", "Chromium", ":connect-browser", "add-on"] {
                assert!(
                    !text.contains(removed),
                    "screen {screen} advertises {removed}"
                );
            }
        }
    }
    #[test]
    fn external_queue_reorder_requests_one_refresh_without_a_status_feedback_loop() {
        let mut app = App::new(Config::default());
        app.screen = 7;
        let value = |revision| serde_json::json!({"playback":{},"queue_len":30,"current":0,"repeat":"off","shuffle":false,"profile":"balanced","pid":1,"browser_pid":null,"backend":"test","playback_available":false,"browser_shared":false,"uptime_secs":0,"cache_entries":0,"message":"","sleep_remaining":null,"queue_revision":revision});
        app.on_status(serde_json::from_value(value(0)).unwrap());
        let changed: Status = serde_json::from_value(value(1)).unwrap();
        assert!(
            matches!(
                app.on_status(changed.clone()),
                Some(Request::QueuePage { offset: 0 })
            ),
            "same-length external reorders must refresh visible items"
        );
        assert!(
            app.on_status(changed).is_none(),
            "the refresh status response must not trigger another refresh"
        );
        app.queue_offset = 25;
        let mut shrunk = value(2);
        shrunk["queue_len"] = 3.into();
        assert!(
            matches!(
                app.on_status(serde_json::from_value(shrunk).unwrap()),
                Some(Request::QueuePage { offset: 0 })
            ),
            "removing the current page must return to a valid queue page"
        );
    }
    #[test]
    fn first_manual_lyric_scroll_moves_the_visible_text() {
        let mut app = App::new(Config::default());
        app.screen = 7;
        app.viewport = Size::new(142, 39);
        app.accept(&Request::QueuePage{offset:0},serde_json::json!({"items":[{"id":"fixture","title":"Our song","artist":"Our artist"}]})).unwrap();
        app.key(crossterm::event::KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::NONE,
        ))
        .unwrap();
        let text = (0..80)
            .map(|i| format!("Preview line {i:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let lyrics = lyrics::Lyrics::parse(Some(&text), None, false).unwrap();
        app.accept_lyrics(LyricsResult {
            request: app.lyrics_request().unwrap(),
            state: lyrics::LyricsState::Found(std::sync::Arc::new(lyrics)),
        });
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(142, 39)).unwrap();
        t.draw(|f| app.render(f)).unwrap();
        let before: String = t
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(before.contains("Preview line 00"));
        app.key(crossterm::event::KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
        ))
        .unwrap();
        t.draw(|f| app.render(f)).unwrap();
        let after: String = t
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            !after.contains("Preview line 00"),
            "the first j should move the top lyric row"
        );
        app.key(crossterm::event::KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        ))
        .unwrap();
        t.draw(|f| app.render(f)).unwrap();
        let back: String = t
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(back.contains("Preview line 00"));
        for _ in 0..80 {
            app.key(crossterm::event::KeyEvent::new(
                KeyCode::Char('j'),
                KeyModifiers::NONE,
            ))
            .unwrap();
        }
        app.viewport = Size::new(142, 48);
        t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(142, 48)).unwrap();
        t.draw(|f| app.render(f)).unwrap();
        let bottom = t.backend().buffer().clone();
        app.key(crossterm::event::KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        ))
        .unwrap();
        t.draw(|f| app.render(f)).unwrap();
        assert!(
            &bottom != t.backend().buffer(),
            "the first k after increasing viewport height must move visible lyrics"
        );
    }
    #[test]
    fn selected_queue_song_can_open_lyrics_without_a_playback_backend() {
        let mut app = App::new(Config::default());
        app.screen = 7;
        app.accept(&Request::QueuePage{offset:0},serde_json::json!({"items":[{"id":"saved-song","title":"Our saved song","artist":"Our artist","duration_ms":100000}]})).unwrap();
        let result = app
            .key(crossterm::event::KeyEvent::new(
                KeyCode::Char('l'),
                KeyModifiers::NONE,
            ))
            .unwrap();
        assert!(result.1.is_none());
        assert_eq!(app.screen, 8);
        assert_eq!(app.lyrics_request().unwrap().query.title, "Our saved song");
        assert!(app.status.is_none());
    }
    #[test]
    fn long_lyric_lines_can_be_scrolled_to_their_end() {
        let mut app = App::new(Config::default());
        app.status=Some(serde_json::from_value(serde_json::json!({"playback":{"track":{"id":"sample","title":"Our song","artist":"Our artist"}},"queue_len":1,"current":0,"repeat":"off","shuffle":false,"profile":"balanced","pid":1,"browser_pid":null,"backend":"test","playback_available":false,"browser_shared":false,"uptime_secs":0,"cache_entries":0,"message":"","sleep_remaining":null})).unwrap());
        let text = format!("{} FINALWORDS", "Our long sample line ".repeat(40));
        let lyrics = lyrics::Lyrics::parse(Some(&text), None, false).unwrap();
        app.accept_lyrics(LyricsResult {
            request: app.lyrics_request().unwrap(),
            state: lyrics::LyricsState::Found(std::sync::Arc::new(lyrics)),
        });
        app.viewport = Size::new(40, 24);
        for _ in 0..100 {
            app.key(crossterm::event::KeyEvent::new(
                KeyCode::Char('j'),
                KeyModifiers::NONE,
            ))
            .unwrap();
        }
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 24)).unwrap();
        t.draw(|f| app.render(f)).unwrap();
        let all: String = t
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(all.contains("FINALWORDS"));
    }
    #[test]
    fn token_entry_is_hidden_and_submits_only_to_local_storage() {
        let mut app = App::new(Config::default());
        let key = |c| crossterm::event::KeyEvent::new(c, KeyModifiers::NONE);
        assert!(app.key(key(KeyCode::Char('L'))).unwrap().1.is_none());
        assert!(app.token_input.is_some());
        for c in "artificial-secret-test".chars() {
            assert!(app.key(key(KeyCode::Char(c))).unwrap().1.is_none());
        }
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        t.draw(|f| app.render(f)).unwrap();
        let all: String = t
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(!all.contains("artificial-secret-test"));
        assert!(app.text.is_empty());
        assert!(app.key(key(KeyCode::Enter)).unwrap().1.is_none());
        assert!(app.token_input.is_none());
        assert!(matches!(app.auth_job, Some(LocalAuth::Save(_))));
    }

    #[tokio::test]
    #[ignore = "visual QA export with public artwork and explicitly synthetic lyrics; no audio"]
    async fn export_player_layout_for_visual_review() {
        let mut app = App::new(Config::default());
        let track=Track{id:"1445734224".into(),kind:"songs".into(),title:"Simple Man".into(),artist:"Lynyrd Skynyrd".into(),album:"Pronounced Leh-Nerd Skin-Nerd (Bonus Tracks)".into(),duration_ms:357933,url:None,artwork:Some("https://is1-ssl.mzstatic.com/image/thumb/Music115/v4/1e/5c/85/1e5c85bb-2b43-3057-da09-39446321a191/06UMGIM14960.rgb.jpg/{w}x{h}bb.jpg".into()),..Track::default()};
        app.status=Some(serde_json::from_value(serde_json::json!({"playback":{"track":track,"position":262.0,"duration":357.933,"volume":0.7},"queue_len":1,"current":0,"repeat":"off","shuffle":false,"profile":"balanced","pid":1,"browser_pid":null,"backend":"layout preview","playback_available":false,"browser_shared":false,"uptime_secs":0,"cache_entries":0,"message":"","sleep_remaining":null})).unwrap());
        app.note = "LAYOUT PREVIEW · no audio · synthetic text · real public album artwork".into();
        let (jobs, mut results, worker) = artwork::worker();
        jobs.send(app.artwork_request(Size::new(142, 39))).unwrap();
        let cover = tokio::time::timeout(std::time::Duration::from_secs(10), results.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(cover.image.is_some());
        app.accept_artwork(cover);
        worker.abort();
        let text = (0..28)
            .map(|i| {
                format!(
                    "[{:02}:{:02}.00]{}",
                    (210 + i * 3) / 60,
                    (210 + i * 3) % 60,
                    [
                        "A line of sample text for this layout",
                        "Centered words beside the album cover",
                        "The current verse follows the song clock",
                        "Use j and k to read at your own pace"
                    ][i % 4]
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let lyrics = lyrics::Lyrics::parse(None, Some(&text), false).unwrap();
        app.accept_lyrics(LyricsResult {
            request: app.lyrics_request().unwrap(),
            state: lyrics::LyricsState::Found(std::sync::Arc::new(lyrics)),
        });
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(142, 39)).unwrap();
        t.draw(|f| app.render(f)).unwrap();
        let color = |c: Color, default| match c {
            Color::Rgb(r, g, b) => [r, g, b],
            _ => default,
        };
        let cells:Vec<_>=t.backend().buffer().content.iter().map(|c|serde_json::json!({"text":c.symbol(),"fg":color(c.fg,[255,255,255]),"bg":color(c.bg,[0,0,0]),"bold":c.modifier.contains(ratatui::style::Modifier::BOLD)})).collect();
        std::fs::write(
            "/tmp/rustune-player-preview.json",
            serde_json::to_vec(&serde_json::json!({"width":142,"height":39,"cells":cells}))
                .unwrap(),
        )
        .unwrap();
    }
}
