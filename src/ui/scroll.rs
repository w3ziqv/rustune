//! Viewport geometry and paging for mouse and keyboard navigation.
use super::*;
use ratatui::widgets::{Scrollbar, ScrollbarState};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Direction {
    Up,
    Down,
}
impl Direction {
    pub(super) fn step(self, position: usize, maximum: usize) -> usize {
        match self {
            Self::Up => position.saturating_sub(1),
            Self::Down => position.saturating_add(1).min(maximum),
        }
    }
}

#[derive(Clone)]
struct PageSelection {
    offset: usize,
    cursor: Option<String>,
    row: usize,
    id: String,
    entry_id: Option<u64>,
}

#[derive(Clone, Copy)]
enum PagePlacement {
    Scroll(Direction),
    Select,
    Reveal,
}

#[derive(Clone, Default)]
pub(super) struct ListViewport {
    pub top: Option<usize>,
    cursor: Option<String>,
    selection: Option<PageSelection>,
    placement: Option<PagePlacement>,
    advance_after_append: bool,
}

pub(super) fn range(top: usize, len: usize, height: usize) -> Range<usize> {
    let height = height.min(len);
    let top = top.min(len - height);
    top..top + height
}

pub(super) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

pub(super) fn menu_rows(inner: Rect) -> [Rect; 3] {
    let hint = inner.height.saturating_sub(2).min(2);
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(hint),
    ])
    .areas(inner)
}

pub(super) fn indicator(f: &mut Frame, area: Rect, position: usize, maximum: usize, p: Palette) {
    if area.width < 2 || area.height < 2 || maximum == 0 {
        return;
    }
    let mut state = ScrollbarState::new(maximum + 1).position(position.min(maximum));
    f.render_stateful_widget(
        Scrollbar::default().style(Style::default().fg(p.muted)),
        area,
        &mut state,
    );
}

pub(super) fn paragraph<'a>(text: impl Into<ratatui::text::Text<'a>>) -> Paragraph<'a> {
    Paragraph::new(text).wrap(Wrap { trim: false })
}

pub(super) fn text_area(area: Rect) -> Rect {
    Rect::new(
        area.x,
        area.y,
        area.width - u16::from(area.width > 1),
        area.height,
    )
}

fn text_max(widget: &Paragraph<'_>, area: Rect) -> usize {
    if area.height == 0 {
        return 0;
    }
    widget
        .line_count(text_area(area).width)
        .saturating_sub(usize::from(area.height))
}

pub(super) fn text_offset(
    widget: Paragraph<'_>,
    area: Rect,
    offset: usize,
    direction: Direction,
) -> usize {
    let max = text_max(&widget, area);
    direction.step(offset.min(max), max)
}

pub(super) fn render_text(
    f: &mut Frame,
    widget: Paragraph<'_>,
    area: Rect,
    offset: usize,
    p: Palette,
) {
    let max = text_max(&widget, area);
    let offset = offset.min(max);
    f.render_widget(widget.scroll((offset as u16, 0)), text_area(area));
    indicator(f, area, offset, max, p);
}

impl App {
    pub(super) fn catalog_screen(&self) -> bool {
        matches!(
            self.screen,
            Screen::Search | Screen::Library | Screen::Playlists | Screen::Queue
        )
    }

    pub(super) fn catalog_areas(&self, area: Rect) -> [Rect; 2] {
        let height = self
            .catalog_header_height()
            .min(area.height.saturating_sub(1));
        Layout::vertical([Constraint::Length(height), Constraint::Fill(1)]).areas(area)
    }

    pub(super) fn list_range(&self, height: usize) -> Range<usize> {
        if let Some(top) = self.list_view.top {
            range(top, self.items.len(), self.list_height(height))
        } else if self.playlist_tracks() {
            self.playlist_range(height)
        } else {
            visible_range(self.selected, self.items.len(), height)
        }
    }

    fn list_height(&self, height: usize) -> usize {
        if self.playlist_tracks() {
            height.min(playlists::TRACK_WINDOW)
        } else {
            height
        }
    }

    pub(super) fn selection_on_page(&self) -> bool {
        self.list_view
            .selection
            .as_ref()
            .is_none_or(|selection| selection.offset == self.row_offset())
    }

    pub(super) fn selected_number(&self) -> usize {
        self.list_view.selection.as_ref().map_or_else(
            || self.row_offset() + self.selected + 1,
            |selection| selection.offset + selection.row + 1,
        )
    }

    pub(super) fn row_offset(&self) -> usize {
        if self.screen == Screen::Queue {
            self.queue_offset
        } else if self.playlist_tracks() {
            0
        } else {
            self.catalog_offset
        }
    }

    fn browse_cursor(&self, next: Option<String>) -> Request {
        Request::Browse {
            kind: self.kind.clone(),
            term: self.query.clone(),
            next,
        }
    }

    fn page_turn(&self, direction: Direction) -> Option<Request> {
        if self.screen == Screen::Queue {
            if self.queue_request.is_some() || self.queue_retry.is_some() || self.items.is_empty() {
                return None;
            }
            let status = self.status.as_ref()?;
            let offset = match direction {
                Direction::Down if self.queue_offset + self.items.len() < status.queue_len => {
                    self.queue_offset + self.items.len()
                }
                Direction::Up if self.queue_offset > 0 => self
                    .queue_offset
                    .saturating_sub(status.profile.limits().page_size),
                _ => return None,
            };
            Some(Request::QueuePage { offset })
        } else if self.browse_state != BrowseState::Loading {
            let next = match direction {
                Direction::Up => self.previous.clone().or_else(|| {
                    // Legacy iTunes responses used an unqualified numeric cursor.
                    let offset = self.list_view.cursor.as_deref()?.parse::<usize>().ok()?;
                    (offset > 0).then(|| {
                        offset
                            .saturating_sub(self.config.performance.limits().page_size)
                            .to_string()
                    })
                }),
                Direction::Down => self.next.clone(),
            }?;
            Some(self.browse_cursor(Some(next)))
        } else {
            None
        }
    }

    pub(super) fn keyboard_page(&mut self, direction: Direction) -> Option<Request> {
        let request = self.page_turn(direction)?;
        self.list_view.placement = Some(PagePlacement::Select);
        Some(request)
    }

    pub(super) fn scroll_catalog(
        &mut self,
        area: Rect,
        direction: Direction,
    ) -> (bool, Option<Request>) {
        if self.items.is_empty() || area.is_empty() {
            return (false, None);
        }
        if direction == Direction::Up {
            self.list_view.stop_append();
        }
        let height = self.list_height(usize::from(area.height));
        let maximum = self.items.len().saturating_sub(height);
        let top = self.list_range(height).start;
        let next_top = direction.step(top, maximum);
        if next_top != top {
            self.list_view.top = Some(next_top);
            return (true, self.playlist_page(false));
        }
        if self.playlist_tracks() {
            if direction == Direction::Down
                && self.next.is_some()
                && self.browse_state != BrowseState::Failed
            {
                self.list_view.top = Some(top);
                self.list_view.advance_after_append = true;
                return (true, self.playlist_page(false));
            }
            return (false, None);
        }
        if self.browse_state == BrowseState::Failed {
            return (false, None);
        }
        let Some(request) = self.page_turn(direction) else {
            return (false, None);
        };
        if self.list_view.selection.is_none() {
            self.list_view.selection = self.items.get(self.selected).map(|track| PageSelection {
                offset: self.row_offset(),
                cursor: self.list_view.cursor.clone(),
                row: self.selected,
                id: track.id.clone(),
                entry_id: self
                    .queue_rows
                    .get(self.selected)
                    .filter(|_| self.screen == Screen::Queue)
                    .map(|entry| entry.entry_id),
            });
        }
        self.list_view.placement = Some(PagePlacement::Scroll(direction));
        (true, Some(request))
    }

    pub(super) fn reveal_selection(&mut self) -> Option<Request> {
        if let Some(selection) = &self.list_view.selection {
            if selection.offset == self.row_offset() {
                if self.queue_request.is_none() && self.browse_state != BrowseState::Loading {
                    self.list_view.selection = None;
                    self.list_view.top = None;
                }
                return None;
            }
            if self.queue_request.is_some() || self.browse_state == BrowseState::Loading {
                return None;
            }
            self.list_view.placement = Some(PagePlacement::Reveal);
            return Some(if self.screen == Screen::Queue {
                Request::QueuePage {
                    offset: selection.offset,
                }
            } else {
                self.browse_cursor(selection.cursor.clone())
            });
        }
        self.list_view.top = None;
        self.update_playlist_top();
        None
    }

    pub(super) fn move_list_selection(&mut self, direction: Direction) {
        let main = navigation::layout(self.viewport, self.screen).main;
        let height = usize::from(self.catalog_areas(main)[1].height);
        let visible = if self.screen == Screen::Settings {
            self.settings_range(self.settings_areas(main)[1], self.settings_rows().len())
        } else {
            self.list_range(height)
        };
        if !self.selection_on_page()
            || !visible.contains(&self.selected) && self.list_view.top.is_some()
        {
            self.selected = match direction {
                Direction::Down => visible.start,
                Direction::Up => visible.end.saturating_sub(1),
            };
        } else {
            let count = if self.screen == Screen::Settings {
                self.settings_rows().len()
            } else {
                self.items.len()
            };
            self.selected = direction.step(self.selected, count.saturating_sub(1));
        }
        self.list_view.selection = None;
        self.list_view.top = None;
        self.list_view.advance_after_append = false;
        self.detail_scroll = 0;
    }

    pub(super) fn finish_page(&mut self, cursor: Option<String>, next: Option<&str>) {
        let cursor = cursor.or_else(|| {
            next.and_then(|next| {
                next.rsplit_once(':')
                    .map(|(prefix, _)| format!("{prefix}:0"))
                    .or_else(|| next.parse::<usize>().ok().map(|_| "0".into()))
            })
        });
        self.list_view.cursor = cursor;
        let placement = self.list_view.placement.take();
        if matches!(placement, Some(PagePlacement::Select)) {
            self.list_view.selection = None;
            self.selected = 0;
        }
        if let Some(selection) = &self.list_view.selection
            && selection.offset == self.row_offset()
        {
            if self
                .items
                .get(selection.row)
                .is_some_and(|row| row.id == selection.id)
                && selection.entry_id.is_none_or(|id| {
                    self.queue_rows
                        .get(selection.row)
                        .is_some_and(|entry| entry.entry_id == id)
                })
            {
                self.selected = selection.row;
            } else {
                self.selected = 0;
                self.note = "Selection changed; select a row again".into();
            }
            self.list_view.selection = None;
        }
        self.selected = self.selected.min(self.items.len().saturating_sub(1));
        self.list_view.top = match placement {
            Some(PagePlacement::Scroll(Direction::Down)) => Some(0),
            Some(PagePlacement::Scroll(Direction::Up)) => Some(self.items.len()),
            Some(PagePlacement::Reveal | PagePlacement::Select) => None,
            None => self.list_view.top,
        };
    }

    pub(super) fn finish_playlist_append(&mut self, old_len: usize) {
        if std::mem::take(&mut self.list_view.advance_after_append) && self.items.len() > old_len {
            let main = navigation::layout(self.viewport, self.screen).main;
            let height = self.list_height(usize::from(self.catalog_areas(main)[1].height));
            if height > 0 {
                self.list_view.top = Some(old_len.saturating_sub(height) + 1);
            }
        }
    }
}

pub(super) fn direction(down: bool) -> Direction {
    if down { Direction::Down } else { Direction::Up }
}

impl ListViewport {
    pub(super) fn stop_append(&mut self) {
        self.advance_after_append = false;
    }

    pub(super) fn forget_placement(&mut self) {
        self.placement = None;
    }
}

impl App {
    pub(super) fn catalog_selection_hidden(&self) -> bool {
        if !self.selection_on_page() {
            return true;
        }
        let main = navigation::layout(self.viewport, self.screen).main;
        let list = self.catalog_areas(main)[1];
        !self.items.is_empty()
            && !self
                .list_range(usize::from(list.height))
                .contains(&self.selected)
    }

    pub(super) fn select_clicked_row(&mut self, row: usize) {
        self.selected = row;
        self.list_view.selection = None;
        self.detail_scroll = 0;
        self.focus = Focus::Content;
    }
}

impl App {
    pub(super) fn settings_areas(&self, area: Rect) -> [Rect; 2] {
        let heading = area.height.saturating_sub(1).min(3);
        Layout::vertical([Constraint::Length(heading), Constraint::Fill(1)]).areas(area)
    }

    pub(super) fn settings_range(&self, area: Rect, count: usize) -> Range<usize> {
        let height = usize::from(area.height);
        self.list_view.top.map_or_else(
            || visible_range(self.selected, count, height),
            |top| range(top, count, height),
        )
    }

    pub(super) fn settings_selection_hidden(&self) -> bool {
        let main = navigation::layout(self.viewport, self.screen).main;
        let body = self.settings_areas(main)[1];
        !self
            .settings_range(body, self.settings_rows().len())
            .contains(&self.selected)
    }

    pub(super) fn scroll_settings(&mut self, area: Rect, direction: Direction) {
        let count = self.settings_rows().len();
        self.list_view.top = Some(direction.step(
            self.settings_range(area, count).start,
            count.saturating_sub(usize::from(area.height)),
        ));
    }
}
