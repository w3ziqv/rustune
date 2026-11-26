//! Mouse hit testing uses the same rectangles as rendering.
use super::*;
use crossterm::event::MouseEvent;
use ratatui::layout::Position;

// Track hit targets rather than repaint status. A playback-clock redraw can
// leave the row under the pointer unchanged, but a page replacement cannot.
#[derive(PartialEq, Eq)]
pub(super) struct ClickLayout {
    screen: Screen,
    size: Size,
    nav_top: Option<usize>,
    catalog: Option<(Rect, Range<usize>, u64)>,
    blocked: bool,
}

impl App {
    pub(super) fn nav_range(&self, area: Rect) -> Range<usize> {
        let height = usize::from(area.height);
        self.nav_top.map_or_else(
            || visible_range(self.nav, Screen::MAIN.len(), height),
            |top| scroll::range(top, Screen::MAIN.len(), height),
        )
    }

    pub(super) fn click_layout(&self) -> ClickLayout {
        let shell = navigation::layout(self.viewport, self.screen);
        let blocked = self.token_input.is_some()
            || self.playlist_dialog.is_some()
            || self.album_actions.is_some()
            || self.song_actions.is_some()
            || self.input != Input::None;
        let catalog = (self.catalog_screen()
            && (shell.nav.is_some() || self.focus != Focus::Navigation))
            .then(|| {
                let list = self.catalog_areas(shell.main)[1];
                (
                    list,
                    self.list_range(usize::from(list.height)),
                    self.rows_revision,
                )
            });
        ClickLayout {
            screen: self.screen,
            size: self.viewport,
            nav_top: shell
                .nav
                .map(|nav| self.nav_range(navigation::nav_block().inner(nav)).start),
            catalog,
            blocked,
        }
    }

    pub(super) fn mouse(&mut self, event: MouseEvent) -> (bool, Option<Request>) {
        if !self.config.mouse || !navigation::usable(self.viewport) {
            return (false, None);
        }
        let point = Position::new(event.column, event.row);
        let shell = navigation::layout(self.viewport, self.screen);
        let window = Rect::new(0, 0, self.viewport.width, self.viewport.height);
        match event.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let direction = direction(event.kind == MouseEventKind::ScrollDown);
                let list = self.catalog_areas(shell.main)[1];
                if let Some(input) = &self.token_input {
                    let body = Self::token_block().inner(Self::token_area(window));
                    if body.contains(point) {
                        self.dialog_scroll = scroll::text_offset(
                            Self::token_paragraph(input),
                            body,
                            self.dialog_scroll,
                            direction,
                        );
                    }
                } else if self.playlist_dialog.is_some() {
                    let area = Self::playlist_dialog_area(window);
                    if area.contains(point) {
                        self.scroll_playlist_dialog(area, direction);
                    }
                } else if self.album_actions.is_some() {
                    if Self::album_menu_area(window).contains(point) {
                        self.scroll_album_menu(direction);
                    }
                } else if let Some(menu) = self.song_menu_area(window) {
                    if menu.contains(point) {
                        self.scroll_song_menu(direction);
                    } else if list.contains(point) && self.catalog_screen() {
                        let (moved, request) = self.scroll_catalog(list, direction);
                        if moved {
                            self.song_actions = None;
                        }
                        return (false, request);
                    }
                } else {
                    return (false, self.scroll_panel(&shell, point, direction));
                }
            }
            MouseEventKind::Down(MouseButton::Left)
                if self.token_input.is_none()
                    && self.playlist_dialog.is_none()
                    && self.album_actions.is_none()
                    && self.song_actions.is_none()
                    && self.input == Input::None =>
            {
                if let Some(nav) = shell.nav.map(|nav| navigation::nav_block().inner(nav))
                    && nav.contains(point)
                    && let Some(index) = self.nav_range(nav).nth(usize::from(point.y - nav.y))
                {
                    if Screen::MAIN[index] == Screen::Lyrics {
                        self.inspected_track = None;
                        self.remember_view();
                    }
                    return (false, self.go(Screen::MAIN[index]));
                }
                if shell.nav.is_none() && self.focus == Focus::Navigation {
                    return (false, None);
                }
                if self.catalog_screen() {
                    let list = self.catalog_areas(shell.main)[1];
                    if list.contains(point)
                        && point.x < list.right().saturating_sub(1)
                        && let Some(index) = self
                            .list_range(usize::from(list.height))
                            .nth(usize::from(point.y - list.y))
                        && let Some(track) = self
                            .items
                            .get(index)
                            .filter(|track| {
                                self.screen == Screen::Queue
                                    || matches!(track.kind.as_str(), "songs" | "library-songs")
                            })
                            .cloned()
                    {
                        self.select_clicked_row(index);
                        self.open_song_actions(track);
                    }
                }
                if self.screen == Screen::NowPlaying {
                    let areas = player::areas(shell.main, player::art_enabled(self));
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
                    if lyrics.height > 1 && lyrics.contains(point) && point.y == lyrics.bottom() - 1
                    {
                        self.lyrics_scroll = None;
                    }
                }
            }
            _ => {}
        }
        (false, None)
    }

    fn scroll_panel(
        &mut self,
        shell: &navigation::ShellAreas,
        point: Position,
        direction: Direction,
    ) -> Option<Request> {
        if let Some(nav) = shell.nav.map(|nav| navigation::nav_block().inner(nav))
            && nav.contains(point)
        {
            self.nav_top = Some(direction.step(
                self.nav_range(nav).start,
                Screen::MAIN.len().saturating_sub(usize::from(nav.height)),
            ));
        } else if let Some(detail) = shell.detail
            && let body = Self::detail_block().inner(self.detail_area(detail, self.viewport.height))
            && body.contains(point)
        {
            self.detail_scroll =
                scroll::text_offset(self.detail_paragraph(), body, self.detail_scroll, direction);
        } else if shell.main.contains(point)
            && (shell.nav.is_some() || self.focus != Focus::Navigation)
        {
            let main = shell.main;
            match self.screen {
                Screen::Help => self.scroll_help(main, direction),
                Screen::AudioInfo if audio_info::hit(main, point) => {
                    audio_info::scroll(self, main, direction)
                }
                Screen::Equalizer if equalizer::areas(main)[1].contains(point) => {
                    self.scroll_equalizer(direction)
                }
                Screen::Settings if self.settings_areas(main)[1].contains(point) => {
                    self.scroll_settings(self.settings_areas(main)[1], direction);
                }
                Screen::Lyrics | Screen::NowPlaying => {
                    let area = player::lyrics_area(self, self.viewport);
                    if player::lyrics_hit(self, area, point) {
                        player::scroll_lyrics(self, area, direction);
                    }
                }
                _ if self.catalog_screen() => {
                    let list = self.catalog_areas(main)[1];
                    if list.contains(point) {
                        if self.items.is_empty() {
                            self.empty_scroll = scroll::text_offset(
                                self.empty_paragraph(),
                                list,
                                self.empty_scroll,
                                direction,
                            );
                        } else {
                            return self.scroll_catalog(list, direction).1;
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::Arc;

    fn wheel(app: &mut App, point: Position, direction: Direction) {
        let result = app.mouse(MouseEvent {
            kind: match direction {
                Direction::Up => MouseEventKind::ScrollUp,
                Direction::Down => MouseEventKind::ScrollDown,
            },
            column: point.x,
            row: point.y,
            modifiers: KeyModifiers::NONE,
        });
        assert!(!result.0 && result.1.is_none());
    }

    #[test]
    fn wrapped_lyrics_scroll_in_player_and_preview_independently_of_keys() {
        let mut app = App::new(Config::default());
        app.config.keybindings.insert("down".into(), "x".into());
        app.config.keybindings.insert("up".into(), "z".into());
        app.viewport = Size::new(70, 12);
        let track = Track {
            title: "Test song".into(),
            artist: "Test artist".into(),
            ..Track::default()
        };
        let document = format!(
            "{}\nLast line",
            "A long line of synthetic lyrics. ".repeat(11)
        );
        let lyrics = Arc::new(lyrics::Lyrics::parse(Some(&document), None, false).unwrap());
        app.status = Some(serde_json::from_value(serde_json::json!({
            "playback": {"track": track}, "queue_len": 1, "current": 0,
            "repeat": "off", "shuffle": false, "profile": "balanced", "pid": 1,
            "backend": "test", "browser_shared": false, "uptime_secs": 0, "cache_entries": 0, "message": ""
        })).unwrap());
        // Preview accepts the same document path as current playback.
        for view in [Screen::NowPlaying, Screen::Lyrics] {
            app.screen = view;
            app.inspected_track = Some(track.clone());
            let request = app.lyrics_request().unwrap();
            app.accept_lyrics(LyricsResult {
                request,
                state: lyrics::LyricsState::Found(Arc::clone(&lyrics)),
            });
            let area = player::lyrics_area(&app, app.viewport);
            assert!(!area.is_empty());
            let point = Position::new(area.x, area.y);
            wheel(&mut app, point, Direction::Down);
            assert_eq!(app.lyrics_scroll, Some(1));
            for _ in 0..20 {
                wheel(&mut app, point, Direction::Down);
            }
            let mut terminal =
                Terminal::new(TestBackend::new(app.viewport.width, app.viewport.height)).unwrap();
            terminal.draw(|frame| app.render(frame)).unwrap();
            let rendered = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(rendered.contains("Last line"));
            wheel(&mut app, point, Direction::Up);
            let previous = app.lyrics_scroll;
            wheel(&mut app, Position::new(0, 0), Direction::Down);
            assert_eq!(app.lyrics_scroll, previous);
            app.config.mouse = false;
            wheel(&mut app, point, Direction::Down);
            assert_eq!(app.lyrics_scroll, previous);
            app.config.mouse = true;
            let request = app
                .key(crossterm::event::KeyEvent::new(
                    KeyCode::Char('f'),
                    KeyModifiers::NONE,
                ))
                .unwrap();
            assert_eq!(app.lyrics_scroll, None);
            assert!(!request.0 && request.1.is_none());
        }
    }
}
