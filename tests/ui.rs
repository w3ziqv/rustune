use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rustune::{
    keys::{action_for, parse_key, validate_bindings},
    ui::{render_smoke, visible_range},
};
#[test]
fn bindings_support_named_keys_and_override() {
    assert_eq!(parse_key("Space").unwrap().code, KeyCode::Char(' '));
    assert_eq!(
        parse_key("ctrl-x").unwrap(),
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)
    );
    assert!(parse_key("nonsense").is_none());
    let mut b = std::collections::BTreeMap::new();
    b.insert("quit".into(), "ctrl-x".into());
    assert_eq!(action_for(parse_key("ctrl-x").unwrap(), &b), Some("quit"));
    assert_eq!(action_for(parse_key("q").unwrap(), &b), None);
    b.insert("unknown".into(), "q".into());
    assert!(validate_bindings(&b).is_err());
}
#[test]
fn large_lists_only_construct_visible_rows() {
    let r = visible_range(9000, 10000, 20);
    assert_eq!(r.len(), 20);
    assert!(r.contains(&9000));
    assert!(visible_range(100, 0, 20).is_empty());
    assert!(visible_range(0, 100, 0).is_empty());
}
#[test]
fn terminal_layout_survives_tiny_and_large_sizes() {
    for (w, h) in [(1, 1), (20, 4), (40, 10), (80, 24), (160, 48), (0, 0)] {
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        t.draw(render_smoke).unwrap();
        if w >= 20 && h >= 4 {
            assert!(
                t.backend()
                    .buffer()
                    .content
                    .iter()
                    .any(|c| c.symbol() == "R")
            );
        }
    }
}

#[test]
fn default_frame_uses_black_background_and_white_text_and_borders() {
    use ratatui::{Terminal, backend::TestBackend, style::Color};
    let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
    t.draw(render_smoke).unwrap();
    let b = t.backend().buffer();
    assert_eq!(b[(1, 0)].fg, Color::Rgb(255, 255, 255));
    assert_eq!(b[(1, 0)].bg, Color::Rgb(0, 0, 0));
    assert_eq!(b[(0, 1)].fg, Color::Rgb(255, 255, 255));
}

fn playing_app() -> rustune::ui::App {
    let mut app = rustune::ui::App::new(rustune::config::Config::default());
    app.screen = 1;
    app.status = Some(serde_json::from_value(serde_json::json!({
        "playback":{"track":{"id":"song","title":"A song","artwork":"https://is1-ssl.mzstatic.com/a/{w}x{h}.jpg"}},
        "queue_len":1,"current":0,"repeat":"off","shuffle":false,"profile":"balanced",
        "pid":1,"browser_pid":null,"backend":"native","browser_shared":false,
        "uptime_secs":0,"cache_entries":0,"message":"","sleep_remaining":null
    })).unwrap());
    app
}

#[test]
fn artwork_is_lazy_disabled_for_minimal_and_stale_results_are_ignored() {
    use ratatui::{Terminal, backend::TestBackend, layout::Size, style::Color};
    use rustune::{artwork::CoverResult, config::PerformanceMode};
    let mut app = playing_app();
    let size = Size::new(100, 30);
    let request = app.artwork_request(size).unwrap();
    assert!(app.artwork_request(Size::new(40, 10)).is_none());
    let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        64,
        64,
        image::Rgb([255, 0, 0]),
    ));
    let protocol = ratatui_image::picker::Picker::halfblocks()
        .new_protocol(image, Size::new(24, 12), ratatui_image::Resize::Fit(None))
        .unwrap();
    app.accept_artwork(CoverResult {
        request: request.clone(),
        image: Some(std::sync::Arc::new(protocol)),
    });
    let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
    t.draw(|f| app.render(f)).unwrap();
    assert!(
        t.backend()
            .buffer()
            .content
            .iter()
            .any(|c| c.fg == Color::Rgb(255, 0, 0) || c.bg == Color::Rgb(255, 0, 0))
    );
    app.status
        .as_mut()
        .unwrap()
        .playback
        .track
        .as_mut()
        .unwrap()
        .artwork = Some("https://is1-ssl.mzstatic.com/b.jpg".into());
    t.draw(|f| app.render(f)).unwrap();
    assert!(
        !t.backend()
            .buffer()
            .content
            .iter()
            .any(|c| c.fg == Color::Rgb(255, 0, 0) || c.bg == Color::Rgb(255, 0, 0))
    );
    app.status.as_mut().unwrap().profile = PerformanceMode::Minimal;
    assert!(app.artwork_request(size).is_none());
}
