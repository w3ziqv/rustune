use rustune::config::Config;

#[test]
fn old_artwork_protocols_migrate_and_explicit_style_wins() {
    for (protocol, style) in [
        ("auto", "pixelated"),
        ("halfblocks", "pixelated"),
        ("kitty", "smooth"),
        ("iterm2", "smooth"),
    ] {
        let c = Config::parse(&format!(
            "artwork_protocol = '{protocol}'\ntheme = 'monochrome'"
        ))
        .unwrap();
        let saved = toml::to_string(&c).unwrap();
        assert!(saved.contains(&format!("artwork_style = \"{style}\"")));
        assert_eq!(c.theme, "monochrome");
    }
    assert!(Config::parse("artwork_style = 'smooth'").is_ok());
    assert!(Config::parse("artwork_style = 'invented'").is_err());
    let c = Config::parse("artwork_protocol = 'kitty'\nartwork_style = 'pixelated'").unwrap();
    assert!(
        toml::to_string(&c)
            .unwrap()
            .contains("artwork_style = \"pixelated\"")
    );
}

#[test]
fn smooth_artwork_never_silently_falls_back_to_halfblocks() {
    let mut app = rustune::ui::App::new(Config::parse("artwork_style='smooth'").unwrap());
    app.screen = rustune::ui::Screen::NowPlaying;
    app.status = Some(serde_json::from_value(serde_json::json!({"playback":{"track":{"id":"one","title":"Song","artist":"Artist","artwork":"https://is1-ssl.mzstatic.com/x.jpg"}},"queue_len":1,"current":0,"repeat":"off","shuffle":false,"profile":"balanced","pid":1,"browser_pid":null,"backend":"test","browser_shared":false,"uptime_secs":0,"cache_entries":0,"message":"","sleep_remaining":null})).unwrap());
    assert!(
        app.artwork_request(ratatui::layout::Size::new(142, 39))
            .is_none()
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(142, 39)).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("Smooth artwork unavailable"));
}
