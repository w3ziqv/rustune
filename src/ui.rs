//! Event-driven terminal client. Only visible rows are constructed.
mod player;
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
    pub config: Config,
    pub status: Option<Status>,
    pub screen: usize,
    pub selected: usize,
    sidebar: bool,
    nav: usize,
    items: Vec<Track>,
    next: Option<String>,
    queue_offset: usize,
    input: Input,
    text: String,
    query: String,
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
    pub fn new(config: Config) -> Self {
        Self {
            config,
            status: None,
            screen: 1,
            selected: 0,
            sidebar: false,
            nav: 1,
            items: Vec::new(),
            next: None,
            queue_offset: 0,
            input: Input::None,
            text: String::new(),
            query: String::new(),
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
    fn displayed_track(&self) -> Option<&Track> {
        if self.screen == 8 {
            self.inspected_track
                .as_ref()
                .or_else(|| self.status.as_ref()?.playback.track.as_ref())
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
    fn page_request(&self) -> Request {
        if self.screen == 7 {
            Request::QueuePage {
                offset: self.queue_offset,
            }
        } else {
            Request::Browse {
                kind: self.kind.clone(),
                term: self.query.clone(),
                next: None,
            }
        }
    }
    fn go(&mut self, screen: usize) -> Option<Request> {
        if screen != 8 {
            self.inspected_track = None;
        }
        self.screen = screen;
        self.nav = screen;
        self.selected = 0;
        self.items.clear();
        self.next = None;
        self.sidebar = false;
        self.query.clear();
        self.queue_offset = 0;
        self.kind = match screen {
            3 => "library",
            4 => "albums",
            5 => "artists",
            6 => "playlists",
            _ => "search:songs",
        }
        .into();
        matches!(screen, 3..=7).then(|| self.page_request())
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
            "connect-browser" => Request::Login,
            "play" => Request::Play,
            "pause" => Request::Pause,
            "next" => Request::Next,
            "previous" => Request::Previous,
            "mute" => Request::Mute,
            "clear" => Request::Clear,
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
                        self.screen = 2;
                        self.nav = 2;
                        self.sidebar = false;
                        self.selected = 0;
                        self.query = self.text.trim().into();
                        self.kind = format!(
                            "search:{}",
                            ["songs", "albums", "artists", "playlists"][self.search_type]
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
                KeyCode::F(2) if self.input == Input::Search => {
                    self.search_type = (self.search_type + 1) % 4
                }
                _ => {}
            }
            return Ok((false, None));
        }
        let action = keys::action_for(k, &self.config.keybindings);
        if matches!(action, Some("down" | "up")) && matches!(self.screen, 1 | 8) && !self.sidebar {
            if let Some(lyrics::LyricsState::Found(lyrics)) =
                self.lyrics_result.as_ref().map(|r| &r.state)
            {
                let area = player::areas(
                    Rect::new(0, 0, self.viewport.width, self.viewport.height),
                    self.config.artwork,
                )
                .lyrics;
                let rows = player::lyric_rows(lyrics, area.width);
                let current = self.lyrics_scroll.unwrap_or_else(|| {
                    lyrics
                        .active_line(self.displayed_position())
                        .and_then(|index| rows.iter().position(|(line, _)| *line == index))
                        .unwrap_or(0)
                });
                self.lyrics_scroll = Some(if action == Some("down") {
                    current.saturating_add(1).min(rows.len().saturating_sub(1))
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
                    self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1));
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
                self.sidebar = !self.sidebar;
                self.nav = self.screen;
            }
            Some("back") => {
                self.sidebar = true;
            }
            Some("select") => {
                if self.sidebar {
                    req = self.go(self.nav);
                } else if let Some(track) = self.items.get(self.selected).cloned() {
                    if self.screen == 7 {
                        req = Some(Request::PlayIndex(self.queue_offset + self.selected));
                    } else if track.kind == "songs" || track.kind == "library-songs" {
                        req = Some(Request::Enqueue {
                            track,
                            next: false,
                            play: true,
                        });
                    } else {
                        self.kind = format!("open:{}:{}", track.kind, track.id);
                        self.query.clear();
                        self.selected = 0;
                        req = Some(self.page_request());
                    }
                }
            }
            Some("enqueue") | Some("play_next") => {
                if let Some(track) = self.items.get(self.selected).cloned() {
                    req = Some(Request::Enqueue {
                        track,
                        next: action == Some("play_next"),
                        play: false,
                    });
                }
            }
            Some("remove") if self.screen == 7 => {
                req = Some(Request::Remove(self.queue_offset + self.selected))
            }
            Some("move_up") if self.screen == 7 && self.queue_offset + self.selected > 0 => {
                let from = self.queue_offset + self.selected;
                req = Some(Request::Move { from, to: from - 1 });
                self.selected = self.selected.saturating_sub(1);
            }
            Some("move_down") if self.screen == 7 => {
                let from = self.queue_offset + self.selected;
                if self.status.as_ref().is_some_and(|s| from + 1 < s.queue_len) {
                    req = Some(Request::Move { from, to: from + 1 });
                    self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1));
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
                        req = Some(self.page_request());
                    }
                } else if let Some(next) = self.next.take() {
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
            Some("visualizer") => self.note =
                "Visualizer unavailable: protected audio samples are not exposed by this backend."
                    .into(),
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
        match r {
            Request::Browse { .. } => {
                let p: Page = serde_json::from_value(value)?;
                self.items = p.items;
                self.next = p.next;
                self.note = format!(
                    "{} results • Enter play/open • a queue • A play next • PgDn more",
                    self.items.len()
                );
            }
            Request::QueuePage { .. } => {
                self.items = serde_json::from_value(value["items"].clone())?;
                self.selected = self.selected.min(self.items.len().saturating_sub(1));
            }
            Request::History => {
                self.note = serde_json::from_value::<Vec<String>>(value)?.join("  ·  ")
            }
            _ => {
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
        if self.token_input.is_some() {
            let a = f.area();
            let w = a.width.min(70);
            let h = a.height.min(11);
            let modal = Rect::new(a.x + (a.width - w) / 2, a.y + (a.height - h) / 2, w, h);
            let p = palette(&self.config);
            f.render_widget(Clear, modal);
            f.render_widget(Paragraph::new("Paste token (hidden)\n\nMusic User Token\n[ input is never displayed ]\n\nEnter save · Esc cancel · Ctrl-U clear\n\nStorage is local; account acceptance is not verified.")
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
            f.render_widget(Paragraph::new("RUSTUNE · L login · q quit"), area);
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
                    "Navigation: {}\n j/k choose · Enter open · Tab return",
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
            let info = format!(
                "SESSION\n\n{}\n\nQueue: {} tracks\nRepeat: {:?}\nShuffle: {}\n\n{}\n\n:login   Save token\n:profile Change resources\n:sleep   Sleep timer\n\nTab  Switch focus\nj/k  Navigate\n?    Help",
                if s.is_some_and(|s| s.playback.authorized) {
                    "Apple Music authorized"
                } else {
                    "Apple Music not authorized"
                },
                s.map_or(0, |s| s.queue_len),
                s.map_or(crate::model::Repeat::Off, |s| s.repeat),
                s.is_some_and(|s| s.shuffle),
                if s.and_then(|s| s.browser_pid).is_some() {
                    "Browser connected"
                } else {
                    "No playback process connected"
                }
            );
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
            || "No track playing  ·  / search to begin".into(),
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
                    "  [F2 type · Enter search]"
                } else {
                    "  [Enter run · Esc cancel]"
                }
            )
        } else {
            format!(" {}", self.note)
        };
        f.render_widget(
            Paragraph::new(text).style(Style::default().fg(p.accent).bg(p.panel)),
            status,
        );
    }
    fn content(&self, f: &mut Frame, a: Rect, p: Palette) {
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
            let title = if self.screen == 2 {
                format!(
                    "SEARCH  {}  ·  {}",
                    ["songs", "albums", "artists", "playlists"][self.search_type],
                    self.query
                )
            } else {
                self.kind.to_uppercase()
            };
            let [header, list] =
                Layout::vertical([Constraint::Length(2), Constraint::Fill(1)]).areas(a);
            f.render_widget(
                Paragraph::new(title).style(Style::default().fg(p.accent).bold()),
                header,
            );
            if self.items.is_empty() {
                f.render_widget(
                    Paragraph::new(if self.screen == 2 {
                        "Press / to search. F2 changes songs / albums / artists / playlists."
                    } else {
                        "No items loaded. :connect-browser connects the optional legacy backend. L saves a local token."
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
                                if i == self.selected { "›" } else { " " },
                                i + 1 + self.queue_offset
                            ),
                            Style::default().fg(p.muted),
                        ),
                        Span::raw(format!("{}  ", t.title)),
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
            0=>format!("YOUR MUSIC. YOUR TERMINAL.\n\n{}\n\nTab switches navigation focus. 1–9 open screens.\n/ Search · Enter play/open · a Add to queue\n:theme monochrome | tokyo_night changes the palette.\n:profile minimal disables artwork and reduces caches.\n\nEXPERIMENTAL · See Audio Settings for available capabilities.",if self.config.backend==crate::config::BackendKind::Disabled {"Native Apple Music playback is not available in this build.\nA Music User Token alone does not provide a playback engine.\nNo browser backend is started by default."}else{"An optional browser playback backend is configured.\n:connect-browser connects it; / searches Apple Music.\nA subscription and compatible protected-content support are required."}),
            1=>self.status.as_ref().and_then(|s|s.playback.track.as_ref()).map_or("Nothing playing. No audio quality has been reported.".into(),|t|format!("NOW PLAYING\n\n{}\n{}\n{}\n\n{}\n\nSpace  Play/pause\n← / →  Seek 10 seconds\n+ / -  Volume\nn / p  Next / previous\ns      Shuffle\nr      Repeat\nm      Mute\n\n:sleep MINUTES sets a sleep timer (0 cancels).",t.title,t.artist,t.album,self.status.as_ref().map(|s|s.playback.quality_label()).unwrap_or_default())),

            9=>"EQUALIZER · UNAVAILABLE\n\n31  62  125  250  500  1k  2k  4k  8k  16k Hz\n\nProtected playback does not expose a verified PCM signal to Rustune.\nEQ, preamp, bass, treble, normalization, crossfade and visualizer are disabled.\n\nUse your existing PipeWire DSP chain (for example EasyEffects) externally.\nNo processing or gapless guarantees are claimed.".into(),
            10=>format!("AUDIO SETTINGS\n\n● Auto               Backend chooses stream quality\n○ Data Saver         Unavailable\n○ Standard           Unavailable\n○ High Quality       Unavailable\n○ Lossless           Unavailable\n○ Hi-Res Lossless    Unavailable\n\nBackend: {}\n{}\n\nSource codec        Unknown\nSource sample rate  Unknown\nSource bit depth    Unknown\nSource channels     Unknown\nBuffer status       Unknown\n\nThe MusicKit target is a requested/adaptive rate, not a measurement.\nPipeWire output format does not establish source quality.",self.status.as_ref().map_or("No playback process",|s|s.backend.as_str()),self.status.as_ref().map_or("Source quality unknown".into(),|s|s.playback.quality_label())),
            11=>format!("SETTINGS\n\nTheme: {}\nResources: {:?}\n\n:theme dark | light | catppuccin_mocha | catppuccin_latte\n:theme gruvbox | nord | monochrome | tokyo_night | custom\n:profile minimal | balanced | enhanced\n\n~/.config/rustune/config.toml\nKeybindings, custom colors, Chromium path, notifications, mouse\nand daemon behavior can be configured there.\n\n:login  :logout  :sleep MINUTES  :clear  :history\nDaemon mode keeps playback running when the TUI closes.",self.config.theme,self.status.as_ref().map_or(self.config.performance,|s|s.profile)),
            _=>{let mut s="HELP · DEFAULT BINDINGS\n\n".to_owned();for (action,key) in keys::DEFAULTS {s.push_str(&format!("{key:<12}{action}   "));if s.lines().last().is_some_and(|l|l.len()>65){s.push('\n');}}s.push_str("\n\nTab → j/k → Enter: choose a screen. 1–9 direct navigation.\nSearch: F2 changes resource type. PgDn loads the next page.\nQueue: Delete removes; J/K reorder; :clear clears.\nCommands: :login :logout :profile MODE :theme NAME\n:sleep MINUTES :recent :history :queue :audio :help\nCtrl-C always exits. ~/.config/rustune/config.toml customizes bindings.");s}
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
    let (jobs, mut work) = mpsc::channel::<Request>(8);
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
    let worker = tokio::spawn(async move {
        while let Some(r) = work.recv().await {
            let result = ipc::request(&worker_paths, &r).await;
            if done.send((r, result)).await.is_err() {
                break;
            }
        }
    });
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
                Some(result)=auth_results.recv()=>match result{Some(saved)=>{app.token_stored=saved;app.note=if saved{auth::SAVED_MESSAGE.into()}else{"Local token removed; browser/account authorization unchanged.".into()};},None=>app.note="Could not update the private token file; check config-directory permissions.".into()},
                Some(cover)=covers.recv()=>{if Some(&cover.request)==last_artwork.as_ref(){app.accept_artwork(cover);}}
                event=events.next()=>{
                    let Some(event)=event else{break;};let event=event?;let mut request=None;
                    match event {
                        Event::Paste(text)=>{if let Some(input)=app.token_input.as_mut(){input.append(&text);}else if app.input!=Input::None && app.text.len()+text.len()<=512 {app.text.extend(text.chars().filter(|c|!c.is_control()));}},
                        Event::Overflow=>{if let Some(input)=app.token_input.as_mut(){input.reject_paste();}app.note="Paste exceeds 16 KiB or is invalid. Ctrl-U clears token input.".into();},
                        Event::Key(k) if k.is_press()=>match app.key(k){Ok((quit,r))=>{if quit{break;}request=r;},Err(e)=>{app.note=e.to_string();app.input=Input::None;}},
                        Event::Mouse(m) if app.token_input.is_none()=>match m.kind {
                            MouseEventKind::ScrollDown=>app.selected=(app.selected+1).min(app.items.len().saturating_sub(1)),
                            MouseEventKind::ScrollUp=>app.selected=app.selected.saturating_sub(1),
                            MouseEventKind::Down(MouseButton::Left) if matches!(app.screen,1|8) && !app.sidebar=>{
                                let size=terminal.size()?;let a=player::areas(Rect::new(0,0,size.width,size.height),app.config.artwork);
                                if let Some(i)=player::buttons(a.controls).iter().position(|r|r.contains(ratatui::layout::Position::new(m.column,m.row))){
                                    let action=player::ACTIONS[i];let key=keys::DEFAULTS.iter().find(|(a,_)|*a==action).map_or("",|(_,k)|*k);let key=app.config.keybindings.get(action).map_or(key,String::as_str);
                                    if let Some(key)=keys::parse_key(key){let (quit,r)=app.key(key)?;if quit{break;}request=r;}
                                }
                            },
                            MouseEventKind::Down(MouseButton::Left) if terminal.size()?.width>=85 && m.column<20 && m.row>=3=>{let i=(m.row-3) as usize;if i<SCREENS.len(){request=app.go(i);}},
                            _=>{}
                        },
                        _=>{}
                    }
                    if let Some(r)=request{if jobs.try_send(r).is_ok(){app.busy=true;}else{app.note="Command queue is full; wait for the current request.".into();}}

                }
                Some((r,result))=responses.recv()=>{
                    app.busy=false;
                    match result {Ok(v)=>{if let Err(e)=app.accept(&r,v){app.note=e.to_string();}
                    if app.screen==7 && !matches!(r,Request::QueuePage{..}){let _=jobs.try_send(app.page_request());}},Err(e)=>app.note=e.to_string()}

                }
                Some(update)=status.recv()=>{match update{Ok(s)=>{app.status=Some(s);},Err(_)=>app.note="Service disconnected. Quit and reopen Rustune to reconnect.".into()}}
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
        let track=Track{id:"1445734224".into(),kind:"songs".into(),title:"Simple Man".into(),artist:"Lynyrd Skynyrd".into(),album:"Pronounced Leh-Nerd Skin-Nerd (Bonus Tracks)".into(),duration_ms:357933,artwork:Some("https://is1-ssl.mzstatic.com/image/thumb/Music115/v4/1e/5c/85/1e5c85bb-2b43-3057-da09-39446321a191/06UMGIM14960.rgb.jpg/{w}x{h}bb.jpg".into())};
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
