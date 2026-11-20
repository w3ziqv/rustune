//! Mouse hit testing uses the same rectangles as rendering.
use super::*;
use crossterm::event::MouseEvent;
impl App {
    pub(super) fn mouse(&mut self, event: MouseEvent) -> (bool, Option<Request>) {
        let point = ratatui::layout::Position::new(event.column, event.row);
        let shell = navigation::layout(self.viewport, self.screen);
        match event.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let down = event.kind == MouseEventKind::ScrollDown;
                if self.screen == Screen::AudioInfo && shell.main.contains(point) {
                    self.focus = Focus::Content;
                    audio_info::scroll(self, down);
                } else if matches!(self.screen, Screen::Lyrics | Screen::NowPlaying)
                    && player::lyrics_area(self, self.viewport).contains(point)
                {
                    self.focus = Focus::Content;
                    let action = if down { "down" } else { "up" };
                    let default = if down { "Down" } else { "Up" };
                    if let Some(key) = keys::parse_key(
                        self.config
                            .keybindings
                            .get(action)
                            .map_or(default, String::as_str),
                    ) {
                        return self.key(key).unwrap_or((false, None));
                    }
                } else if shell.main.contains(point) {
                    let count = if self.screen == Screen::Settings {
                        self.settings_rows().len()
                    } else {
                        self.items.len()
                    };
                    self.selected = if down {
                        (self.selected + 1).min(count.saturating_sub(1))
                    } else {
                        self.selected.saturating_sub(1)
                    };
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(nav) = shell.nav.filter(|nav| nav.contains(point)) {
                    let index = usize::from(event.row.saturating_sub(nav.y + 1));
                    if event.row > nav.y && index < Screen::MAIN.len() {
                        if Screen::MAIN[index] == Screen::Lyrics {
                            self.inspected_track = None;
                            self.remember_view();
                        }
                        return (false, self.go(Screen::MAIN[index]));
                    }
                }
                if self.screen == Screen::NowPlaying {
                    let areas = player::areas(
                        shell.main,
                        self.config.artwork && self.config.performance != PerformanceMode::Minimal,
                    );
                    if let Some(i) = player::buttons(areas.controls)
                        .iter()
                        .position(|r| r.contains(point))
                    {
                        self.focus = Focus::Player(i);
                        return self.player_action(i);
                    }
                }
                if matches!(self.screen, Screen::Lyrics | Screen::NowPlaying) {
                    let lyrics = player::lyrics_area(self, self.viewport);
                    if lyrics.contains(point) && event.row == lyrics.bottom().saturating_sub(1) {
                        self.lyrics_scroll = None;
                    }
                }
            }
            _ => {}
        }
        (false, None)
    }
}
