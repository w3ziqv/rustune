//! Destinations and keyboard focus are independent of menu ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Search,
    Library,
    Playlists,
    Queue,
    NowPlaying,
    Lyrics,
    Settings,
    Equalizer,
    AudioInfo,
    Help,
}
impl Screen {
    pub const MAIN: [Self; 7] = [
        Self::Search,
        Self::Library,
        Self::Playlists,
        Self::Queue,
        Self::NowPlaying,
        Self::Lyrics,
        Self::Settings,
    ];
    pub fn nav_index(self) -> usize {
        Self::MAIN.iter().position(|s| *s == self).unwrap_or(6)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Search => "Search",
            Self::Library => "Library",
            Self::Playlists => "Playlists",
            Self::Queue => "Queue",
            Self::NowPlaying => "Now Playing",
            Self::Lyrics => "Lyrics",
            Self::Settings => "Settings",
            Self::Equalizer => "Settings / Equalizer",
            Self::AudioInfo => "Settings / Audio information",
            Self::Help => "Help",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Focus {
    #[default]
    Content,
    Navigation,
    Player(usize),
}

use ratatui::{
    layout::{Constraint, Layout, Rect, Size},
    widgets::{Block, Borders, Padding},
};

pub(super) fn usable(size: Size) -> bool {
    size.width >= 20 && size.height >= 5
}

pub(super) fn nav_block() -> Block<'static> {
    Block::new()
        .borders(Borders::RIGHT)
        .padding(Padding::top(1))
}
pub(super) struct ShellAreas {
    pub top: Rect,
    pub nav: Option<Rect>,
    pub main: Rect,
    pub detail: Option<Rect>,
    pub player: Rect,
    pub status: Rect,
}
pub(super) fn layout(size: Size, screen: Screen) -> ShellAreas {
    let [top, body, player, status] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Fill(1),
        Constraint::Length(if screen == Screen::NowPlaying || size.height < 10 {
            0
        } else {
            3
        }),
        Constraint::Length(1),
    ])
    .areas(Rect::new(0, 0, size.width, size.height));
    let (nav, main) = if size.width >= 100 {
        let [nav, main] =
            Layout::horizontal([Constraint::Length(18), Constraint::Fill(1)]).areas(body);
        (Some(nav), main)
    } else {
        (None, body)
    };
    let (main, detail) = if size.width >= 130
        && matches!(
            screen,
            Screen::Search | Screen::Library | Screen::Playlists | Screen::Queue
        ) {
        let [main, detail] =
            Layout::horizontal([Constraint::Fill(1), Constraint::Length(29)]).areas(main);
        (main, Some(detail))
    } else {
        (main, None)
    };
    ShellAreas {
        top,
        nav,
        main: main.inner(ratatui::layout::Margin::new(2, u16::from(main.height > 3))),
        detail,
        player,
        status,
    }
}
