//! Event-driven terminal client. Only visible rows are constructed.
mod album;
mod audio_info;
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
    diagnostics,
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

mod graphics;
mod lyric_layout;
mod mouse;
mod navigation;
use navigation::Focus;
pub use navigation::Screen;
pub const SCREENS: &[&str] = &[
    "Search",
    "Library",
    "Playlists",
    "Queue",
    "Now Playing",
    "Lyrics",
    "Settings",
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
        "graphite" => (0x161619, 0xe6e3df, 0xaaa6ad, 0xb8a7d9, 0x202024),
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
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum BrowseState {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed,
}
pub struct App {
    eq_selected: usize,
    eq_preset: usize,
    eq_pending: Option<rustune_core::equalizer::EqSettings>,
    eq_confirmed: rustune_core::equalizer::EqSettings,
    pub config: Config,
    pub status: Option<Status>,
    pub screen: Screen,
    pub selected: usize,
    focus: Focus,
    song_actions: Option<interaction::SongActions>,
    album_actions: Option<album::AlbumActions>,
    album_view: Option<crate::catalog::album::AlbumRef>,
    playlist_view: Option<crate::catalog::playlist::PlaylistRef>,
    playlist_top: usize,
    playlist_bytes: usize,
    account_generation: Option<u64>,
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
    browse_state: BrowseState,
    browse_retry: Option<Request>,
    connected: bool,
    lyric_layout: lyric_layout::WrappedLyrics,
    cover: Option<CoverResult>,
    graphics: graphics::Graphics,
    token_input: Option<TokenInput>,
    auth_job: Option<LocalAuth>,
    token_stored: bool,
    diagnostics: Option<diagnostics::Report>,
    diagnostics_generation: u64,
    audio_info_scroll: usize,
    lyrics_result: Option<LyricsResult>,
    lyrics_generation: u64,
    lyrics_scroll: Option<usize>,
    restore_lyrics_scroll: bool,
    viewport: Size,
    inspected_track: Option<Track>,
}
impl App {
    fn dispatch(&mut self, jobs: &mpsc::Sender<Request>, request: Request) {
        if matches!(request, Request::Browse { .. }) {
            self.browse_state = BrowseState::Loading;
            self.browse_retry = Some(request.clone());
        }
        match jobs.try_send(request) {
            Ok(()) => self.busy = true,
            Err(error) => {
                self.request_failed(&error.into_inner());
                self.note = "Command queue is full; wait for the current request.".into();
            }
        }
    }
    fn request_failed(&mut self, request: &Request) {
        if let Request::Browse { kind, term, .. } = request
            && kind == &self.kind
            && term == &self.query
        {
            self.browse_state = BrowseState::Failed;
            self.browse_retry = Some(request.clone());
        }
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
            screen: Screen::Search,
            selected: 0,
            focus: Focus::Content,
            song_actions: None,
            album_actions: None,
            album_view: None,
            playlist_view: None,
            playlist_top: 0,
            playlist_bytes: 0,
            account_generation: None,
            playlist_dialog: None,
            playlist_target: None,
            queue_rows: Vec::new(),
            queue_upcoming: Vec::new(),
            nav: 0,
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
            browse_state: BrowseState::Idle,
            browse_retry: None,
            connected: true,
            lyric_layout: Default::default(),
            cover: None,
            graphics: graphics::Graphics::default(),
            token_input: None,
            auth_job: None,
            token_stored: false,
            diagnostics: None,
            diagnostics_generation: 0,
            audio_info_scroll: 0,
            lyrics_result: None,
            lyrics_generation: 0,
            lyrics_scroll: None,
            restore_lyrics_scroll: false,
            viewport: Size::new(100, 30),
            inspected_track: None,
        }
    }
    pub fn artwork_request(&self, size: Size) -> Option<CoverRequest> {
        let profile = self
            .status
            .as_ref()
            .map_or(self.config.performance, |s| s.profile);
        if self.screen == Screen::Lyrics
            || !self.config.artwork
            || profile.limits().artwork_entries == 0
        {
            return None;
        }
        let cover_size = if self.screen == Screen::NowPlaying {
            player::cover_size(self, size)?
        } else if navigation::layout(size, self.screen).detail.is_some() && size.height >= 28 {
            artwork::COVER_SIZE
        } else {
            return None;
        };
        let protocol = match self.config.artwork_style {
            crate::config::ArtworkStyle::Pixelated => "halfblocks",
            crate::config::ArtworkStyle::Smooth => self.graphics.protocol.filter(|p| {
                self.config.artwork_protocol == "auto"
                    || self.config.artwork_protocol == "halfblocks"
                    || self.config.artwork_protocol == *p
            })?,
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
            protocol: protocol.into(),
            cell: if protocol == "halfblocks" {
                (10, 20)
            } else {
                self.graphics.cell
            },
            image_id: self.graphics.image_id,
            background,
        })
    }
    fn smooth_unavailable(&self) -> bool {
        self.config.artwork_style == crate::config::ArtworkStyle::Smooth
            && self.graphics.protocol.is_none_or(|p| {
                !["auto", "halfblocks", p].contains(&self.config.artwork_protocol.as_str())
            })
    }
    pub fn accept_artwork(&mut self, cover: CoverResult) {
        self.cover = Some(cover);
    }
    pub fn lyrics_request(&self) -> Option<LyricsRequest> {
        if !self.config.lyrics || !matches!(self.screen, Screen::NowPlaying | Screen::Lyrics) {
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
        let account_changed = self.account_generation != status.account_generation;
        if account_changed {
            self.account_generation = status.account_generation;
            self.invalidate_private_views();
        }
        if !self.connected {
            self.connected = true;
            self.note = "Connected to service".into();
        }
        if let Some(appearance) = &status.appearance {
            self.config.theme = appearance.theme.clone();
            self.config.artwork = appearance.artwork;
            self.config.artwork_style = appearance.artwork_style;
            self.config.lyrics = appearance.lyrics;
        }
        self.config.performance = status.profile;
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
            if self.screen == Screen::Queue
                && self.queue_view_revision != Some(status.queue_revision)
            {
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
        (self.screen == Screen::Queue && changed
            || account_changed && matches!(self.screen, Screen::Library | Screen::Playlists))
        .then(|| self.page_request())
    }
    fn displayed_track(&self) -> Option<&Track> {
        if self.screen == Screen::Lyrics {
            self.inspected_track
                .as_ref()
                .or_else(|| self.status.as_ref()?.playback.track.as_ref())
        } else if matches!(
            self.screen,
            Screen::Search | Screen::Library | Screen::Playlists | Screen::Queue
        ) {
            self.items.get(self.selected)
        } else {
            self.status.as_ref()?.playback.track.as_ref()
        }
    }
    fn lyrics_preview(&self) -> bool {
        self.screen == Screen::Lyrics && self.inspected_track.is_some()
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
                Paragraph::new(if self.smooth_unavailable() {
                    "Smooth artwork unavailable. Enable pixelated artwork in Settings or use a compatible terminal."
                } else if cover.is_some() {
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
        if self.screen == Screen::Queue {
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
                next: self
                    .album_view
                    .as_ref()
                    .filter(|_| self.kind.starts_with("open:albums:"))
                    .map(|album| format!("{}:{}:0", album.provider, album.storefront)),
            }
        }
    }
    fn refresh_diagnostics(&mut self) {
        self.diagnostics = None;
        self.diagnostics_generation = self.diagnostics_generation.wrapping_add(1);
        self.audio_info_scroll = 0;
    }
    fn go(&mut self, screen: Screen) -> Option<Request> {
        if screen == Screen::AudioInfo {
            self.refresh_diagnostics();
        }
        if screen != self.screen && screen != Screen::Lyrics && self.playlist_target.is_none() {
            self.remember_view();
        }
        if screen != Screen::Lyrics {
            self.inspected_track = None;
        }
        self.screen = screen;
        self.nav = screen.nav_index();
        self.focus = Focus::Content;
        self.song_actions = None;
        self.album_actions = None;
        self.album_view = None;
        self.playlist_view = None;
        self.playlist_dialog = None;
        if screen != Screen::Playlists {
            self.playlist_target = None;
        }
        self.queue_rows.clear();
        self.selected = 0;
        self.items.clear();
        self.next = None;
        self.previous = None;
        self.focus = Focus::Content;
        self.query.clear();
        self.browse_state = BrowseState::Idle;
        self.browse_retry = None;
        self.queue_offset = 0;
        self.catalog_offset = 0;
        self.kind = match screen {
            Screen::Library => "library",
            Screen::Playlists => "playlists",
            _ => "search:songs",
        }
        .into();
        self.input = Input::None;
        if screen == Screen::Search {
            self.search_type = 0;
            self.text.clear();
            self.note = "/ Search · Tab focus · ? Help".into();
        }
        matches!(screen, Screen::Library | Screen::Playlists | Screen::Queue)
            .then(|| self.page_request())
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
            "playlists" => return Ok(self.go(Screen::Playlists)),
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
                return Ok(self.go(Screen::Queue));
            }
            "audio" => {
                if self.screen != Screen::Settings {
                    self.go(Screen::Settings);
                }
                self.go(Screen::AudioInfo);
                return Ok(None);
            }
            "help" => {
                self.go(Screen::Help);
                return Ok(None);
            }
            "recent" => {
                self.go(Screen::Library);
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
                        if !matches!(self.screen, Screen::Search) {
                            self.screen = Screen::Search;
                        }
                        self.nav = self.screen.nav_index();
                        self.focus = Focus::Content;
                        self.items.clear();
                        self.album_view = None;
                        self.playlist_view = None;
                        self.playlist_target = None;
                        self.next = None;
                        self.previous = None;
                        self.catalog_offset = 0;
                        self.back_stack.clear();
                        self.note = "Searching…".into();
                        self.browse_state = BrowseState::Loading;
                        self.browse_retry = None;
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
        if self.input == Input::None && self.focus != Focus::Navigation {
            match k.code {
                KeyCode::Char('a')
                    if matches!(
                        self.screen,
                        Screen::Search | Screen::Library | Screen::Playlists
                    ) =>
                {
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
                KeyCode::Char('N') if self.screen == Screen::Playlists => {
                    self.playlist_name(false);
                    return Ok((false, None));
                }
                KeyCode::Char('S') if matches!(self.screen, Screen::Playlists | Screen::Queue) => {
                    self.playlist_name(true);
                    return Ok((false, None));
                }
                KeyCode::Char('A') if self.screen == Screen::Playlists => {
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
                KeyCode::Char('u') if self.screen == Screen::Queue => {
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
                KeyCode::F(5) if self.screen == Screen::Playlists => {
                    return Ok((false, Some(self.page_request())));
                }
                KeyCode::Char('R') if self.screen == Screen::AudioInfo => {
                    self.refresh_diagnostics();
                    return Ok((false, None));
                }
                KeyCode::Char('R')
                    if !matches!(self.screen, Screen::Lyrics | Screen::NowPlaying) =>
                {
                    return Ok((
                        false,
                        self.browse_retry.clone().or(Some(Request::RetryOperation)),
                    ));
                }
                _ => {}
            }
        }
        let action = keys::action_for(k, &self.config.keybindings);
        if self.screen == Screen::Equalizer
            && k.modifiers.is_empty()
            && self.focus != Focus::Navigation
            && !matches!(self.focus, Focus::Player(_))
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
        if let Focus::Player(index) = self.focus {
            match action {
                Some("menu_left") => {
                    if index > 0 {
                        self.focus = Focus::Player(index - 1);
                    } else {
                        self.focus = Focus::Content;
                        self.focus = Focus::Navigation;
                        self.nav = self.screen.nav_index();
                    }
                    return Ok((false, None));
                }
                Some("menu_right") => {
                    self.focus = Focus::Player((index + 1).min(6));
                    return Ok((false, None));
                }
                Some("select") => return Ok(self.player_action(index)),
                Some("up" | "down") => {
                    self.focus = Focus::Content;
                    return Ok((false, None));
                }
                _ => {}
            }
        }
        if action == Some("down")
            && k.code == KeyCode::Down
            && self.screen == Screen::NowPlaying
            && self.focus != Focus::Navigation
            && !matches!(
                self.lyrics_result.as_ref().map(|r| &r.state),
                Some(lyrics::LyricsState::Found(_))
            )
        {
            self.focus = Focus::Player(0);
            return Ok((false, None));
        }

        if matches!(action, Some("down" | "up"))
            && matches!(self.screen, Screen::NowPlaying | Screen::Lyrics)
            && self.focus != Focus::Navigation
        {
            if let Some(lyrics::LyricsState::Found(lyrics)) =
                self.lyrics_result.as_ref().map(|r| &r.state)
            {
                let area = player::lyrics_area(self, self.viewport);
                let rows = self.lyric_layout.rows(lyrics, area.width);
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
        if self.screen == Screen::AudioInfo
            && self.focus != Focus::Navigation
            && matches!(action, Some("up" | "down"))
        {
            audio_info::scroll(self, action == Some("down"));
            return Ok((false, None));
        }

        let mut req = None;
        match action {
            Some("lyrics_follow") => self.lyrics_scroll = None,
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
                self.focus = Focus::Content;
                self.input = Input::Search;
                self.text = self.query.clone();
            }
            Some("command") => {
                self.input = Input::Command;
                self.text.clear();
            }
            Some("down") => {
                if self.focus == Focus::Navigation {
                    self.nav = (self.nav + 1) % SCREENS.len();
                } else {
                    self.selected = (self.selected + 1).min(if self.screen == Screen::Settings {
                        self.settings_rows().len() - 1
                    } else {
                        self.items.len().saturating_sub(1)
                    });
                }
            }
            Some("up") => {
                if self.focus == Focus::Navigation {
                    self.nav = self.nav.saturating_sub(1);
                } else {
                    self.selected = self.selected.saturating_sub(1);
                }
            }
            Some("focus") => {
                if self.focus == Focus::Navigation {
                    self.focus = Focus::Content;
                    if self.screen == Screen::NowPlaying {
                        self.focus = Focus::Player(0);
                    }
                } else if matches!(self.focus, Focus::Player(_)) {
                    self.focus = Focus::Content;
                } else {
                    self.focus = Focus::Navigation;
                    self.nav = self.screen.nav_index();
                }
            }
            Some("back" | "menu_left")
                if self.focus != Focus::Navigation && self.restore_view() =>
            {
                if self.screen == Screen::Queue {
                    req = Some(self.page_request());
                } else if self.browse_state == BrowseState::Loading {
                    req = self.browse_retry.clone();
                }
            }
            Some("back" | "menu_left") => {
                self.focus = Focus::Content;
                self.focus = Focus::Navigation;
                self.nav = self.screen.nav_index();
            }
            Some("menu_right")
                if self.focus != Focus::Navigation && self.screen == Screen::NowPlaying =>
            {
                self.focus = Focus::Player(0);
            }

            Some("select" | "menu_right") => {
                if self.focus == Focus::Navigation {
                    if Screen::MAIN[self.nav] == Screen::Lyrics {
                        self.inspected_track = None;
                        self.remember_view();
                    }
                    req = self.go(Screen::MAIN[self.nav]);
                } else if self.screen == Screen::Settings {
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
                        self.playlist_top = 0;
                        self.playlist_bytes = 0;
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
            Some("remove") if self.screen == Screen::Queue => {
                req = self.checked_queue_edit(self.queue_offset + self.selected, None)
            }
            Some("move_up")
                if self.screen == Screen::Queue && self.queue_offset + self.selected > 0 =>
            {
                let from = self.queue_offset + self.selected;
                req = self.checked_queue_edit(from, Some(from - 1));
                self.selected = self.selected.saturating_sub(1);
            }
            Some("move_down") if self.screen == Screen::Queue => {
                let from = self.queue_offset + self.selected;
                if self.status.as_ref().is_some_and(|s| from + 1 < s.queue_len) {
                    req = self.checked_queue_edit(from, Some(from + 1));
                    self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1));
                }
            }
            Some("page_back") => {
                if self.playlist_tracks() {
                    self.selected = self.selected.saturating_sub(playlists::TRACK_WINDOW);
                } else if self.screen == Screen::Queue && self.queue_offset > 0 {
                    self.queue_offset = self
                        .queue_offset
                        .saturating_sub(self.config.performance.limits().page_size);
                    self.selected = 0;
                    self.items.clear();
                    self.queue_view_revision = None;
                    req = Some(self.page_request());
                } else if matches!(
                    self.screen,
                    Screen::Search | Screen::Library | Screen::Playlists
                ) {
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
                if self.playlist_tracks() {
                    self.selected = self
                        .selected
                        .saturating_add(playlists::TRACK_WINDOW)
                        .min(self.items.len().saturating_sub(1));
                    req = self.playlist_page(true);
                } else if self.screen == Screen::Queue {
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
                if self.status.is_some() {
                    let offset = if action == Some("seek_forward") {
                        10_000_000
                    } else {
                        -10_000_000
                    };
                    req = Some(Request::SeekBy(offset));
                }
            }
            Some("shuffle") => req = Some(Request::Shuffle),
            Some("repeat") => req = Some(Request::Repeat),
            Some("mute") => req = Some(Request::Mute),
            Some("help") => {
                self.go(Screen::Help);
            }
            Some("settings") => {
                self.go(Screen::Settings);
            }
            Some("equalizer") => {
                if self.screen != Screen::Settings {
                    self.go(Screen::Settings);
                }
                self.go(Screen::Equalizer);
            }
            Some("lyrics") => {
                if matches!(
                    self.screen,
                    Screen::Search | Screen::Library | Screen::Playlists | Screen::Queue
                ) {
                    self.remember_view();
                }
                self.inspected_track = if matches!(
                    self.screen,
                    Screen::Search | Screen::Library | Screen::Playlists | Screen::Queue
                ) {
                    self.items.get(self.selected).cloned()
                } else {
                    None
                };
                self.go(Screen::Lyrics);
                if self.lyrics_preview() {
                    self.note = "Lyrics preview for the selected song; playback unchanged.".into();
                }
            }
            Some("visualizer") => self.note = "Visualizer is not implemented in this build.".into(),
            _ => match k.code {
                KeyCode::Char(c @ '1'..='7') => {
                    let destination = Screen::MAIN[c as usize - '1' as usize];
                    if destination == Screen::Lyrics {
                        self.inspected_track = None;
                        self.remember_view();
                    }
                    req = self.go(destination);
                }
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1))
                }
                _ => {}
            },
        }
        self.update_playlist_top();
        if req.is_none() && self.focus == Focus::Content && matches!(action, Some("down" | "page"))
        {
            req = self.playlist_page(false);
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
                let append = self.playlist_tracks() && next.is_some();
                if append && next != &self.next {
                    return Ok(());
                }
                let p: Page = serde_json::from_value(value)?;
                if crate::catalog::playlist::private_kind(kind) {
                    anyhow::ensure!(
                        p.account_generation.is_some()
                            && p.account_generation == self.account_generation,
                        "Account credentials changed; reopen the library"
                    );
                }
                if let Some(playlist) = &self.playlist_view
                    && p.storefront
                        .as_deref()
                        .is_some_and(|region| !region.eq_ignore_ascii_case(&playlist.storefront))
                {
                    anyhow::bail!("Account region changed; reopen the playlist list");
                }
                if self.playlist_tracks() {
                    let bytes = if append { self.playlist_bytes } else { 0 };
                    let bytes = bytes + serde_json::to_vec(&p.items)?.len();
                    let count = if append { self.items.len() } else { 0 } + p.items.len();
                    anyhow::ensure!(
                        bytes <= crate::catalog::album::MAX_ALBUM_BYTES
                            && count <= crate::queue::MAX_QUEUE,
                        "Playlist exceeds the local viewing limit"
                    );
                    self.playlist_bytes = bytes;
                }
                self.catalog_offset = next
                    .as_deref()
                    .and_then(|n| n.rsplit(':').next()?.parse().ok())
                    .unwrap_or(0);
                self.browse_state = BrowseState::Ready;
                self.browse_retry = None;
                self.catalog_context = p.context.unwrap_or_else(|| "Apple/iTunes catalog".into());
                if append {
                    self.items.extend(p.items);
                } else {
                    self.items = p.items;
                    if self.playlist_tracks() {
                        self.selected = 0;
                        self.playlist_top = 0;
                    }
                }
                self.next = p.next;
                self.previous = p.previous;
                self.note = format!(
                    "{} results · {} · Enter actions/open · PgDn more",
                    self.items.len(),
                    self.catalog_context
                );
            }
            Request::QueuePage { offset } => {
                if self.screen != Screen::Queue || *offset != self.queue_offset {
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
                    Request::ArtworkStyle(style) => self.config.artwork_style = *style,
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
        let navigation::ShellAreas {
            top,
            nav,
            main,
            detail,
            player,
            status,
        } = navigation::layout(Size::new(area.width, area.height), self.screen);
        let mode = self
            .status
            .as_ref()
            .map_or(self.config.performance, |s| s.profile);
        let title = Line::from(vec![
            Span::styled(" RUSTUNE ", Style::default().fg(p.accent).bold()),
            Span::styled(
                format!("/ {}", self.screen.label()),
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
        if let Some(nav) = nav {
            let rows: Vec<_> = SCREENS
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    ListItem::new(format!(
                        " {} {}",
                        if self.focus == Focus::Navigation && self.nav == i {
                            ">"
                        } else if self.screen.nav_index() == i {
                            "•"
                        } else {
                            " "
                        },
                        s
                    ))
                    .style(
                        if ((self.focus == Focus::Navigation) && self.nav == i)
                            || self.screen.nav_index() == i
                        {
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
        let content = main;
        if self.focus == Focus::Navigation && nav.is_none() {
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
            let detail = if self.config.artwork
                && mode != PerformanceMode::Minimal
                && area.height >= 28
                && self
                    .displayed_track()
                    .and_then(|t| t.artwork.as_ref())
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
            if matches!(self.screen, Screen::Search) {
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
            || "Nothing playing  ·  / Search".into(),
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
        let text = if self.input != Input::None {
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
    fn catalog_header_height(&self) -> u16 {
        if matches!(self.screen, Screen::Playlists | Screen::Queue)
            && self
                .status
                .as_ref()
                .is_some_and(|s| s.playlist_write.is_some())
        {
            3
        } else {
            2
        }
    }
    fn content(&self, f: &mut Frame, a: Rect, p: Palette) {
        if self.screen == Screen::Equalizer {
            self.render_equalizer(f, a, p);
            return;
        }
        if self.screen == Screen::Settings {
            self.render_settings(f, a, p);
            return;
        }
        if self.screen == Screen::AudioInfo {
            audio_info::render(self, f, a, p);
            return;
        }
        if self.screen == Screen::Lyrics {
            let [header, text] = player::standalone_lyrics_areas(a);
            let title = self.displayed_track().map_or_else(
                || "LYRICS".into(),
                |t| {
                    format!(
                        "LYRICS · {} — {}{}",
                        t.title,
                        t.artist,
                        if self.lyrics_preview() {
                            " · PREVIEW"
                        } else {
                            ""
                        }
                    )
                },
            );
            f.render_widget(Paragraph::new(title).wrap(Wrap { trim: false }), header);
            player::render_lyrics(self, f, text, p);
            return;
        }
        if self.screen == Screen::NowPlaying {
            player::render(self, f, a, p);
            return;
        }
        let a = if self.screen == Screen::NowPlaying
            && self
                .artwork_request(Size::new(f.area().width, f.area().height))
                .is_some()
        {
            self.render_cover(f, a, p);
            Rect::new(a.x + 28, a.y, a.width.saturating_sub(28), a.height)
        } else {
            a
        };
        if matches!(
            self.screen,
            Screen::Search | Screen::Library | Screen::Playlists | Screen::Queue
        ) {
            let title = if self.screen == Screen::Queue {
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
            } else if self.screen == Screen::Playlists {
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
            } else if matches!(self.screen, Screen::Search) {
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
                .filter(|_| matches!(self.screen, Screen::Playlists | Screen::Queue));
            let title = if let Some(write) = write {
                format!("{title}\nAccount write {}: {}", write.phase, write.message)
            } else {
                title
            };
            let [header, list] = Layout::vertical([
                Constraint::Length(self.catalog_header_height()),
                Constraint::Fill(1),
            ])
            .areas(a);
            f.render_widget(
                Paragraph::new(title).style(Style::default().fg(p.accent).bold()),
                header,
            );
            if self.items.is_empty() {
                f.render_widget(
                    Paragraph::new(if self.browse_state == BrowseState::Failed {
                        "Could not load results. R retries; previous results are preserved."
                    } else if self.browse_state == BrowseState::Loading {
                        "Loading results…"
                    } else if self.browse_state == BrowseState::Ready {
                        "No results for this query. / edits your search."
                    } else if self.screen == Screen::Search {
                        "Type a query and press Enter. Tab/F2 changes songs, albums, artists or My library."
                    } else {
                        if self.screen==Screen::Library {"MY LIBRARY · requires an accepted Music User Token. Search → My library searches your saved songs."}else{"No results loaded. Choose Search and type a query."}
                    })
                    .style(Style::default().fg(p.muted))
                    .wrap(Wrap { trim: false }),
                    list,
                );
                return;
            }
            let range = if self.playlist_tracks() {
                self.playlist_range(usize::from(list.height))
            } else {
                visible_range(self.selected, self.items.len(), usize::from(list.height))
            };
            let rows: Vec<_> = range
                .map(|i| {
                    let t = &self.items[i];
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!(
                                "{} {:>3}  ",
                                if self.screen == Screen::Queue
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
                        Span::raw(if self.screen == Screen::Queue {
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
                            if self.screen == Screen::Queue {
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

            Screen::NowPlaying=>self.status.as_ref().and_then(|s|s.playback.track.as_ref()).map_or("Nothing playing. No audio quality has been reported.".into(),|t|format!("NOW PLAYING\n\n{}\n{}\n{}\n\n{}\n\nSpace  Play/pause\nAlt-← / →  Seek 10 seconds\n+ / -  Volume\nn / p  Next / previous\ns      Shuffle\nr      Repeat\nm      Mute\n\n:sleep MINUTES sets a sleep timer (0 cancels).",t.title,t.artist,t.album,self.status.as_ref().map(|s|s.playback.quality_label()).unwrap_or_default())),

            Screen::Settings=>format!("SETTINGS\n\nTheme: {}\nResources: {:?}\n\n:theme dark | light | catppuccin_mocha | catppuccin_latte\n:theme gruvbox | nord | monochrome | tokyo_night | custom\n:profile minimal | balanced | enhanced\n\n~/.config/rustune/config.toml\nKeybindings, custom colors, notifications, mouse\nand daemon behavior can be configured there.\n\n:login  :logout  :sleep MINUTES  :clear  :history\nDaemon mode keeps playback running when the TUI closes.",self.config.theme,self.status.as_ref().map_or(self.config.performance,|s|s.profile)),
            _=>{let mut s="HELP · YOUR BINDINGS\n\n".to_owned();for (action,default) in keys::DEFAULTS {let key = self.config.keybindings.get(*action).map_or(*default, String::as_str);s.push_str(&format!("{key:<12}{action}   "));if s.lines().last().is_some_and(|l|l.len()>65){s.push('\n');}}s.push_str("\n\n←/Esc menu · ↑/↓ choose · Enter/→ open. Tab cycles focus.\nAlbums: Enter opens; a opens whole-album actions. Songs: Enter opens actions. Settings: ↑/↓ then Enter changes a value.\nSearch: Tab/F2 changes songs/albums/artists/My library. PgUp/PgDn changes pages.\nEQ: e opens; ↑/↓ selects; ←/→ edits; b bypasses; r resets.\n← returns from an album or lyrics to previous results. Alt-←/→ seeks.\nQueue: Delete removes and advances; J/K reorder; :clear upcoming; u Undo.\nCommands: :stop :undo :cancel :retry :login :logout :profile MODE :theme NAME\n:sleep MINUTES :recent :history :queue :audio :help\nCtrl-C always exits. ~/.config/rustune/config.toml customizes bindings.");s}
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
    let initial_status: Status =
        serde_json::from_value(ipc::request(&paths, &Request::Status).await?)?;
    app.account_generation = initial_status.account_generation;
    app.status = Some(initial_status);
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
    let subscription = tokio::spawn(runtime::subscribe(paths.clone(), updates));
    let mut diagnostic_check = tokio::task::JoinSet::new();
    // Fullscreen pixels need their RGB foreground/background pairs, including
    // with the monochrome UI theme. Inherited NO_COLOR otherwise turns covers
    // into unrecognizable block glyphs. This only affects the TUI process.
    crossterm::style::force_color_output(true);
    let mut terminal = ratatui::init();
    let (graphics, initial_input) = graphics::Graphics::detect();
    app.graphics = graphics;
    let (artwork_jobs, mut covers, artwork_worker) = artwork::worker();
    let mut last_artwork = None;
    let (lyrics_jobs, mut lyric_results, lyrics_worker) = lyrics::worker();
    let mut last_lyrics = None;
    let result=async {
        if config.mouse { execute!(stdout(), EnableMouseCapture)?; }
        execute!(stdout(), crossterm::event::EnableBracketedPaste)?;
        let mut events=Events::with_initial(&initial_input)?;
        drop(initial_input);
        let mut resize=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())?;
        let mut terminate=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut hangup=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?;
        let mut dirty = true;
        let mut next_frame = tokio::time::Instant::now();
        loop {
            app.viewport=terminal.size()?;
            if app.screen == Screen::AudioInfo && app.diagnostics.is_none() && diagnostic_check.is_empty() {
                let paths = paths.clone();
                let generation = app.diagnostics_generation;
                diagnostic_check.spawn(async move { (generation, diagnostics::inspect(&paths).await) });
            }
            if let Some(job)=app.auth_job.take() && auth_jobs.try_send(job).is_err(){app.note="Token storage is busy; try login again.".into();}
            let request=app.artwork_request(terminal.size()?);
            if request!=last_artwork {
                last_artwork=request.clone();
                app.graphics.clear();
                app.cover=None;
                let _=artwork_jobs.send(request);
            }
            let request=app.lyrics_request();if request!=last_lyrics {last_lyrics=request.clone();app.lyrics_result=None;app.lyric_layout.clear();if !app.restore_lyrics_scroll { app.lyrics_scroll=None; } app.restore_lyrics_scroll=false;let _=lyrics_jobs.send(request);}
            if dirty && tokio::time::Instant::now() >= next_frame {
                terminal.draw(|f|app.render(f))?;
                dirty = false;
                next_frame = tokio::time::Instant::now() + std::time::Duration::from_millis(16);
            }
            tokio::select! {
                _=tokio::time::sleep_until(next_frame), if dirty => {},
                _=terminate.recv()=>break,
                _=hangup.recv()=>break,
                _=resize.recv()=>{dirty=true;},
                Some(result)=lyric_results.recv()=>{dirty=true;if Some(&result.request)==last_lyrics.as_ref(){app.accept_lyrics(result);}},
                Some(result) = auth_results.recv() => {
                    dirty = true;
                    match result {
                        Some(saved) => {
                            app.token_stored = saved;
                            let generation = auth::token_stamp(&paths).map(auth::TokenStamp::generation);
                            if app.account_generation != generation {
                                app.invalidate_private_views();
                            }
                            app.refresh_diagnostics();
                            app.note = if saved { auth::SAVED_MESSAGE.into() } else {
                                "Local token removed; Apple account authorization unchanged.".into()
                            };
                        }
                        None => app.note = "Could not update the private token file; check config-directory permissions.".into(),
                    }
                }
                Some(result) = diagnostic_check.join_next(), if !diagnostic_check.is_empty() => {
                    if let Ok((generation, report)) = result && generation == app.diagnostics_generation {
                        app.diagnostics = Some(report);
                        dirty = true;
                    }
                }
                Some(cover)=covers.recv()=>{dirty=true;if Some(&cover.request)==last_artwork.as_ref(){app.accept_artwork(cover);}}
                event=events.next()=>{
                    dirty=true;
                    let Some(event)=event else{break;};let event=event?;let mut request=None;
                    match event {
                        Event::Paste(text)=>{if let Some(input)=app.token_input.as_mut(){input.append(&text);}else if app.playlist_dialog.is_some(){app.playlist_paste(&text);}else if app.input!=Input::None && app.text.len()+text.len()<=512 {app.text.extend(text.chars().filter(|c|!c.is_control()));}},
                        Event::Overflow=>{if let Some(input)=app.token_input.as_mut(){input.reject_paste();}app.note="Paste exceeds 16 KiB or is invalid. Ctrl-U clears token input.".into();},
                        Event::Key(k) if k.is_press()=>match app.key(k){Ok((quit,r))=>{if quit{break;}request=r;},Err(e)=>{app.note=e.to_string();app.input=Input::None;}},
                        Event::Mouse(m) if app.token_input.is_none() && app.song_actions.is_none() && app.playlist_dialog.is_none()=> {
                            let (quit,r) = app.mouse(m);
                            if quit { break; }
                            request = r;
                        },
                        _=>{}
                    }
                    if let Some(r) = request { app.dispatch(&jobs, r); }

                }
                Some((r,result))=responses.recv()=>{
                    dirty=true;
                    app.busy=false;
                    if let Request::Browse { kind, term, next } = &r
                        && (!matches!(app.screen, Screen::Search | Screen::Library | Screen::Playlists)
                            || kind != &app.kind || term != &app.query
                            || !matches!(&app.browse_retry, Some(Request::Browse {next: pending, ..}) if pending == next))
                    { continue; }
                    if let Request::QueuePage {offset}=&r && (app.screen!=Screen::Queue || *offset!=app.queue_offset) {continue;}
                    let _publication = if result.is_ok()
                        && matches!(&r, Request::Browse { kind, .. } if crate::catalog::playlist::private_kind(kind))
                    {
                        let Ok(lock) = auth::token_publication(&paths) else {
                            app.request_failed(&r);
                            app.note = "Account credentials are changing; R retry".into();
                            continue;
                        };
                        let current = auth::token_stamp(&paths).map(auth::TokenStamp::generation);
                        if app.account_generation != current {
                            app.invalidate_private_views();
                            continue;
                        }
                        Some(lock)
                    } else {
                        None
                    };
                    match result {Ok(v)=>{
                    if let Err(e)=app.accept(&r,v) {
                        app.request_failed(&r);
                        app.note=e.to_string();
                    } else if matches!(r, Request::Browse { .. }) && let Some(prefetch) = app.playlist_page(false) {
                        app.dispatch(&jobs, prefetch);
                    }
                    if app.screen==Screen::Queue && !matches!(r,Request::QueuePage{..}){let _=jobs.try_send(app.page_request());}},Err(e)=>{
                        app.request_failed(&r);
                        app.note = if matches!(r, Request::Browse { .. }) {
                            format!("{e} · R retry")
                        } else {
                            e.to_string()
                        };
                    }}

                }
                Some(update) = status.recv() => match update {
                    Ok(s) => {
                        dirty |= !app.connected || app.account_generation != s.account_generation || runtime::redraw_needed(app.status.as_ref(), &s, app.screen == Screen::Equalizer);
                        let reconnect = !app.connected;
                        if reconnect {
                            app.refresh_diagnostics();
                        }
                        let refresh = app.on_status(s);
                        let refresh = if reconnect && matches!(app.screen,Screen::Queue | Screen::Library | Screen::Playlists | Screen::Search) && (app.screen != Screen::Search || !app.query.is_empty()) {
                            Some(app.page_request())
                        } else { refresh };
                        if let Some(r) = refresh { app.dispatch(&jobs, r); }
                    },
                    Err(_) => {
                        if app.connected {
                            dirty = true;
                            app.refresh_diagnostics();
                        }
                        app.connected = false;
                        app.note = "Service disconnected; reconnecting…".into();
                    },
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
    app.graphics.clear();
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
        for screen in Screen::MAIN {
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
                    "screen {screen:?} advertises {removed}"
                );
            }
        }
    }
    #[test]
    fn external_queue_reorder_requests_one_refresh_without_a_status_feedback_loop() {
        let mut app = App::new(Config::default());
        app.screen = Screen::Queue;
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
        app.screen = Screen::Queue;
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
        app.screen = Screen::Queue;
        app.accept(&Request::QueuePage{offset:0},serde_json::json!({"items":[{"id":"saved-song","title":"Our saved song","artist":"Our artist","duration_ms":100000}]})).unwrap();
        let result = app
            .key(crossterm::event::KeyEvent::new(
                KeyCode::Char('l'),
                KeyModifiers::NONE,
            ))
            .unwrap();
        assert!(result.1.is_none());
        assert_eq!(app.screen, Screen::Lyrics);
        assert_eq!(app.lyrics_request().unwrap().query.title, "Our saved song");
        assert!(app.status.is_none());
    }
    #[test]
    fn long_lyric_lines_can_be_scrolled_to_their_end() {
        let mut app = App::new(Config::default());
        app.screen = Screen::Lyrics;
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
        app.screen = Screen::NowPlaying;
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
