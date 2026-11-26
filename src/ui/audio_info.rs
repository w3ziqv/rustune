use super::*;

fn body(app: &App) -> Vec<String> {
    let mut lines = if let Some(report) = &app.diagnostics {
        let mut lines: Vec<_> = report.checks().into();
        let quality = app.status.as_ref().filter(|_| app.connected).map_or_else(
            || report.source_quality().into(),
            |status| status.playback.quality_label(),
        );
        lines.push(quality);
        if !report.next_steps().is_empty() {
            lines.push(String::new());
            lines.extend(report.next_steps().iter().map(|step| format!("• {step}")));
        }
        lines
    } else {
        vec!["Checking local playback setup…".into()]
    };
    lines.push(String::new());
    lines
        .push("The backend selects quality. Manual selection and lossless are unavailable.".into());
    lines.push("Source bit depth unknown.".into());
    lines
}

fn areas(area: Rect) -> [Rect; 2] {
    let heading = area.height.saturating_sub(1).min(2);
    Layout::vertical([Constraint::Length(heading), Constraint::Fill(1)]).areas(area)
}

fn paragraph(app: &App) -> Paragraph<'static> {
    scroll::paragraph(body(app).into_iter().map(Line::from).collect::<Vec<_>>())
}

pub(super) fn scroll(app: &mut App, area: Rect, direction: Direction) {
    let content = areas(area)[1];
    app.audio_info_scroll =
        scroll::text_offset(paragraph(app), content, app.audio_info_scroll, direction);
}

pub(super) fn hit(area: Rect, point: ratatui::layout::Position) -> bool {
    areas(area)[1].contains(point)
}

pub(super) fn render(app: &App, frame: &mut Frame, area: Rect, palette: Palette) {
    let [heading, content] = areas(area);
    frame.render_widget(
        Paragraph::new("AUDIO INFORMATION\n↑/↓ Scroll · R Refresh · Esc Back")
            .style(Style::default().fg(palette.accent)),
        heading,
    );
    scroll::render_text(
        frame,
        paragraph(app).style(Style::default().fg(palette.fg)),
        content,
        app.audio_info_scroll,
        palette,
    );
}
