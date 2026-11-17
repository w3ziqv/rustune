//! The selected player composition: cover left, metadata/lyrics right, controls below.
use super::{App, Focus, Palette, Screen, navigation, time, visible_range};
use crate::{keys, lyrics::LyricsState};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect, Size},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Gauge, Paragraph, Wrap},
};

pub(super) const ACTIONS: [&str; 7] = [
    "toggle",
    "next",
    "previous",
    "volume_up",
    "volume_down",
    "mute",
    "quit",
];
pub(super) struct Areas {
    pub cover: Option<Rect>,
    pub metadata: Rect,
    pub lyrics: Rect,
    pub progress: Rect,
    pub controls: Rect,
    pub status: Rect,
}
pub(super) fn areas(area: Rect, artwork: bool) -> Areas {
    let [_header, body, progress, controls, status] = Layout::vertical([
        Constraint::Length(0),
        Constraint::Fill(1),
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(area);
    let body = body.inner(ratatui::layout::Margin::new(1, 1));
    let (cover, right) = if artwork && area.width >= 70 && body.height >= 8 {
        let [left, _, right] = Layout::horizontal([
            Constraint::Percentage(42),
            Constraint::Length(3),
            Constraint::Fill(1),
        ])
        .areas(body);
        let w = left.width.min(body.height.saturating_mul(2)).min(96) / 2 * 2;
        let h = w / 2;
        (
            Some(Rect::new(left.x, left.y + (left.height - h) / 2, w, h)),
            right,
        )
    } else {
        (None, body)
    };
    let [metadata, lyrics] =
        Layout::vertical([Constraint::Length(6), Constraint::Fill(1)]).areas(right);
    Areas {
        cover,
        metadata,
        lyrics,
        progress,
        controls,
        status,
    }
}
pub(super) fn buttons(area: Rect) -> Vec<Rect> {
    Layout::horizontal([Constraint::Fill(1); 7])
        .split(area)
        .to_vec()
}
fn art_enabled(app: &App) -> bool {
    app.config.artwork
        && app
            .status
            .as_ref()
            .map_or(app.config.performance, |s| s.profile)
            .limits()
            .artwork_entries
            > 0
}
pub(super) fn cover_size(app: &App, size: Size) -> Option<Size> {
    areas(navigation::layout(size, app.screen).main, art_enabled(app))
        .cover
        .map(|r| Size::new(r.width, r.height))
}
pub(super) fn lyrics_area(app: &App, size: Size) -> Rect {
    let main = navigation::layout(size, app.screen).main;
    if app.screen == Screen::Lyrics {
        standalone_lyrics_areas(main)[1]
    } else {
        areas(main, art_enabled(app)).lyrics
    }
}

pub(super) fn render(app: &App, f: &mut Frame, area: Rect, p: Palette) {
    let size = Size::new(f.area().width, f.area().height);
    let a = areas(area, art_enabled(app));
    if let Some(cover) = a.cover {
        let key = app.artwork_request(size);
        let result = app
            .cover
            .as_ref()
            .filter(|r| Some(&r.request) == key.as_ref());
        if let Some(image) = result.and_then(|r| r.image.as_ref()) {
            f.render_widget(ratatui_image::Image::new(image), cover);
        } else {
            let text = if app.smooth_unavailable() {
                "Smooth artwork unavailable.\nThis terminal needs Kitty/iTerm2 graphics and cell geometry.\nEnable pixelated artwork in Settings."
            } else if app.displayed_track().is_none() {
                "\nRUSTUNE\n\nNothing playing\n\n← Menu · → Controls\n/  Public search"
            } else if key.is_none() {
                "\nNo artwork reported"
            } else if result.is_some() {
                "\nArtwork unavailable"
            } else {
                "\nLoading artwork…"
            };
            f.render_widget(
                Paragraph::new(text)
                    .alignment(Alignment::Center)
                    .block(Block::bordered().border_style(Style::default().fg(p.fg))),
                cover,
            );
        }
    }
    let track = app.displayed_track();
    let [names, album] =
        Layout::horizontal([Constraint::Percentage(60), Constraint::Fill(1)]).areas(a.metadata);
    let (title, artist, album_name) = track.map_or(("Nothing playing", "—", "—"), |t| {
        (t.title.as_str(), t.artist.as_str(), t.album.as_str())
    });
    f.render_widget(
        Paragraph::new(vec![
            Line::from("TITLE"),
            Line::from(title).style(Style::default().bold()),
            Line::from("ARTIST"),
            Line::from(artist).style(Style::default().bold()),
        ])
        .wrap(Wrap { trim: false }),
        names,
    );
    f.render_widget(
        Paragraph::new(vec![
            Line::from("ALBUM"),
            Line::from(album_name).style(Style::default().bold()),
        ])
        .wrap(Wrap { trim: false }),
        album,
    );
    if a.metadata.height >= 6 {
        f.render_widget(
            Paragraph::new(if app.lyrics_preview() {
                "LYRICS PREVIEW · playback unchanged".into()
            } else {
                app.status
                    .as_ref()
                    .map_or("Source quality unknown".into(), |s| {
                        s.playback.quality_label()
                    })
            }),
            Rect::new(a.metadata.x, a.metadata.y + 5, a.metadata.width, 1),
        );
    }
    render_lyrics(app, f, a.lyrics, p);
    let (position, duration) = if app.lyrics_preview() {
        (0.0, track.map_or(0.0, |t| t.duration_ms as f64 / 1000.0))
    } else {
        app.status
            .as_ref()
            .map_or((0.0, 0.0), |s| (s.playback.position, s.playback.duration))
    };
    let ratio = if duration > 0.0 {
        (position / duration).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let [clock, bar] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(a.progress);
    f.render_widget(
        Paragraph::new(format!(
            " [ {} / {} · {:.0}% ]",
            time(position),
            time(duration),
            ratio * 100.0
        )),
        clock,
    );
    f.render_widget(
        Gauge::default()
            .ratio(ratio)
            .label("")
            .gauge_style(Style::default().fg(p.fg).bg(p.bg)),
        bar,
    );
    for (i, area) in buttons(a.controls).iter().enumerate() {
        let key = keys::DEFAULTS
            .iter()
            .find(|(action, _)| *action == ACTIONS[i])
            .map_or("", |(_, key)| *key);
        let key = app
            .config
            .keybindings
            .get(ACTIONS[i])
            .map_or(key, String::as_str);
        let title = match i {
            0 => {
                if app.status.as_ref().is_some_and(|s| s.playback.playing) {
                    "Pause"
                } else {
                    "Play"
                }
            }
            1 => "Next",
            2 => "Prev",
            3 => "Vol+",
            4 => "Vol−",
            5 => "Mute",
            _ => "Quit",
        };
        let focused = app.focus == Focus::Player(i);
        let block = Block::bordered()
            .border_style(Style::default().fg(p.fg))
            .border_type(if focused {
                ratatui::widgets::BorderType::Double
            } else {
                ratatui::widgets::BorderType::Plain
            });
        f.render_widget(
            Paragraph::new(format!("{title} [{key}]"))
                .style(if focused {
                    Style::default().fg(p.fg).bold().underlined()
                } else {
                    Style::default().fg(p.fg)
                })
                .alignment(Alignment::Center)
                .block(block),
            *area,
        );
    }
    let status = if app.input != super::Input::None {
        format!(
            "{}{}",
            if app.input == super::Input::Command {
                ":"
            } else {
                "/ "
            },
            app.text
        )
    } else {
        app.status_message()
    };
    f.render_widget(Paragraph::new(status), a.status);
}

pub(super) fn render_lyrics(app: &App, f: &mut Frame, area: Rect, p: Palette) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let request = app.lyrics_request();
    let result = app
        .lyrics_result
        .as_ref()
        .filter(|r| Some(&r.request) == request.as_ref());
    let message = if !app.config.lyrics {
        Some("Lyrics disabled in config.")
    } else if request.is_none() {
        Some("Play a track to load lyrics.\n\nL Login · / Search")
    } else {
        match result.map(|r| &r.state) {
            None => Some("Loading lyrics from LRCLIB…"),
            Some(LyricsState::Missing) => Some("No matching lyrics on LRCLIB.\nR retries."),
            Some(LyricsState::Error(e)) => Some(e.message()),
            Some(LyricsState::Found(l)) if l.instrumental => Some("Instrumental · no lyrics"),
            _ => None,
        }
    };
    if let Some(message) = message {
        f.render_widget(
            Paragraph::new(message)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let Some(LyricsState::Found(lyrics)) = result.map(|r| &r.state) else {
        return;
    };
    let position = app.displayed_position();
    let active = lyrics.active_line(position);
    let wrapped = app.lyric_layout.rows(lyrics, area.width);
    let active_row = active
        .and_then(|index| wrapped.iter().position(|(line, _)| *line == index))
        .unwrap_or(0);
    let height = usize::from(area.height.saturating_sub(1));
    let range = if let Some(top) = app.lyrics_scroll {
        let top = top.min(wrapped.len().saturating_sub(height));
        top..(top + height).min(wrapped.len())
    } else {
        visible_range(active_row, wrapped.len(), height)
    };
    let rows: Vec<_> = range
        .map(|i| {
            let (line, range) = &wrapped[i];
            let text = &lyrics.lines[*line].text[range.clone()];
            Line::from(text)
                .alignment(Alignment::Center)
                .style(if active == Some(*line) {
                    Style::default()
                        .fg(p.fg)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    Style::default().fg(p.fg)
                })
        })
        .collect();
    f.render_widget(
        Paragraph::new(rows),
        Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1)),
    );
    f.render_widget(
        Paragraph::new(format!(
            "LRCLIB · {} · {} follow · {} reload",
            if lyrics.synced {
                "synced"
            } else {
                "plain text"
            },
            app.config
                .keybindings
                .get("lyrics_follow")
                .map_or("f", String::as_str),
            app.config
                .keybindings
                .get("lyrics_refresh")
                .map_or("R", String::as_str)
        ))
        .alignment(Alignment::Center),
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
    );
}

pub(super) fn standalone_lyrics_areas(area: Rect) -> [Rect; 2] {
    Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(area)
}
