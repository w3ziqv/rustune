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

pub(super) fn scroll(app: &mut App, down: bool) {
    app.audio_info_scroll = if down {
        (app.audio_info_scroll + 1).min(body(app).len().saturating_sub(1))
    } else {
        app.audio_info_scroll.saturating_sub(1)
    };
}

pub(super) fn render(app: &App, frame: &mut Frame, area: Rect, palette: Palette) {
    let [heading, content] =
        Layout::vertical([Constraint::Length(2), Constraint::Fill(1)]).areas(area);
    frame.render_widget(
        Paragraph::new("AUDIO INFORMATION\n↑/↓ Scroll · R Refresh · Esc Back")
            .style(Style::default().fg(palette.accent)),
        heading,
    );
    let lines = body(app);
    let offset = app.audio_info_scroll.min(lines.len().saturating_sub(1));
    frame.render_widget(
        Paragraph::new(
            lines
                .into_iter()
                .skip(offset)
                .map(Line::from)
                .collect::<Vec<_>>(),
        )
        .style(Style::default().fg(palette.fg))
        .wrap(Wrap { trim: false }),
        content,
    );
}
