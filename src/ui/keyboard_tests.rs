use super::*;
use crossterm::event::KeyEvent;
fn press(app: &mut App, code: KeyCode) -> Option<Request> {
    app.key(KeyEvent::new(code, KeyModifiers::NONE)).unwrap().1
}
#[test]
fn arrow_keys_move_focused_navigation_and_open_search() {
    let mut app = App::new(Config::default());
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Down);
    assert_eq!(
        app.nav, 2,
        "Down must move the navigation, not the song list"
    );
    press(&mut app, KeyCode::Right);
    assert_eq!(app.screen, 2);
    assert!(
        app.input == Input::Search,
        "Search should immediately accept typing"
    );
    assert!(!app.sidebar);
}
#[test]
fn settings_are_actionable_using_arrows_and_enter() {
    let mut app = App::new(Config::default());
    app.go(11);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    assert!(
        matches!(press(&mut app, KeyCode::Enter), Some(Request::Theme(_))),
        "third settings row changes theme"
    );
}
#[test]
fn song_actions_allow_queueing_without_attempting_playback() {
    let mut app = App::new(Config::default());
    app.screen = 2;
    app.items.push(Track {
        id: "123".into(),
        kind: "songs".into(),
        title: "A public song".into(),
        ..Track::default()
    });
    assert!(
        press(&mut app, KeyCode::Enter).is_none(),
        "Enter opens actions instead of attempting unavailable audio"
    );
    press(&mut app, KeyCode::Down);
    assert!(matches!(
        press(&mut app, KeyCode::Enter),
        Some(Request::Enqueue { play: false, .. })
    ));
}
#[test]
fn right_arrow_focuses_player_buttons_and_enter_activates() {
    let mut app = App::new(Config::default());
    assert!(press(&mut app, KeyCode::Right).is_none());
    assert!(matches!(
        press(&mut app, KeyCode::Enter),
        Some(Request::Toggle)
    ));
    press(&mut app, KeyCode::Right);
    assert!(matches!(
        press(&mut app, KeyCode::Enter),
        Some(Request::Next)
    ));
}
#[test]
fn left_arrow_returns_from_album_tracks_to_search_results() {
    let mut app = App::new(Config::default());
    app.screen = 2;
    app.kind = "search:albums".into();
    app.query = "Our album".into();
    app.items.push(Track {
        id: "123".into(),
        kind: "albums".into(),
        title: "Our album".into(),
        ..Track::default()
    });
    assert!(matches!(
        press(&mut app, KeyCode::Enter),
        Some(Request::Browse { .. })
    ));
    assert!(
        app.items.is_empty(),
        "album lookup must not leave its parent rows actionable"
    );
    press(&mut app, KeyCode::Left);
    assert_eq!(app.kind, "search:albums");
    assert!(!app.sidebar);
    assert_eq!(app.items[0].id, "123");
}
#[test]
fn page_up_returns_to_previous_catalog_page() {
    let mut app = App::new(Config::default());
    app.screen = 2;
    app.kind = "search:songs".into();
    app.query = "Our song".into();
    app.accept(
        &Request::Browse {
            kind: app.kind.clone(),
            term: app.query.clone(),
            next: Some("25".into()),
        },
        serde_json::json!({"items":[],"next":"50"}),
    )
    .unwrap();
    assert!(
        matches!(press(&mut app,KeyCode::PageUp),Some(Request::Browse {next:Some(ref cursor),..}) if cursor=="0")
    );
}
#[test]
fn search_leaves_player_focus_and_invalidates_old_results() {
    let mut app = App::new(Config::default());
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Char('/'));
    app.items.push(Track {
        id: "old".into(),
        kind: "songs".into(),
        ..Track::default()
    });
    app.next = Some("50".into());
    app.catalog_offset = 25;
    for c in "new query".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    assert!(matches!(
        press(&mut app, KeyCode::Enter),
        Some(Request::Browse { .. })
    ));
    assert!(
        app.control_focus.is_none(),
        "new results must not activate old player controls"
    );
    assert!(
        app.items.is_empty() && app.next.is_none() && app.catalog_offset == 0,
        "new query must invalidate old actionable rows/pages"
    );
}
#[test]
fn returning_to_queue_requires_fresh_rows_before_actions() {
    let mut app = App::new(Config::default());
    app.screen = 7;
    app.items.push(Track {
        id: "old".into(),
        kind: "songs".into(),
        ..Track::default()
    });
    press(&mut app, KeyCode::Char('l'));
    assert_eq!(app.screen, 8);
    assert!(matches!(
        press(&mut app, KeyCode::Left),
        Some(Request::QueuePage { .. })
    ));
    assert!(
        app.items.is_empty(),
        "restored indices may be stale after external queue changes"
    );
    assert!(press(&mut app, KeyCode::Enter).is_none());
    assert!(app.song_actions.is_none());
}
#[test]
fn stale_queue_pages_do_not_populate_a_search_or_another_page() {
    let mut app = App::new(Config::default());
    app.screen = 2;
    let value = serde_json::json!({"items":[{"id":"old","kind":"songs","title":"Old queue"}]});
    app.accept(&Request::QueuePage { offset: 0 }, value.clone())
        .unwrap();
    assert!(
        app.items.is_empty(),
        "queue response must not replace current public search"
    );
    app.screen = 7;
    app.queue_offset = 25;
    app.accept(&Request::QueuePage { offset: 0 }, value)
        .unwrap();
    assert!(
        app.items.is_empty(),
        "queue response offset must match the current page"
    );
}

fn assert_previous_catalog_page(screen: usize) {
    let mut app = App::new(Config {
        performance: crate::config::PerformanceMode::Enhanced,
        ..Config::default()
    });
    app.screen = screen;
    app.kind = if screen == 3 {
        "library"
    } else {
        "search:songs"
    }
    .into();
    app.query = "query".into();
    app.accept(
        &Request::Browse {
            kind: app.kind.clone(),
            term: app.query.clone(),
            next: Some("50".into()),
        },
        serde_json::json!({"items":[], "next":"music:pl:75", "previous":"music:pl:25"}),
    )
    .unwrap();
    assert!(
        matches!(press(&mut app, KeyCode::PageUp), Some(Request::Browse { next:Some(ref cursor),.. }) if cursor=="music:pl:25"),
        "screen {screen} must return one actual catalog page"
    );
}
#[test]
fn library_page_up_returns_to_the_previous_page() {
    assert_previous_catalog_page(3);
}
#[test]
fn enhanced_search_page_up_returns_one_actual_catalog_page() {
    assert_previous_catalog_page(2);
}
#[test]
fn equalizer_preserves_modified_seek_bindings_and_plain_arrows_edit_bands() {
    let mut app = App::new(Config::default());
    app.go(9);
    app.eq_selected = 2;
    app.status = Some(serde_json::from_value(serde_json::json!({"playback":{"position":50.0,"duration":120.0},"queue_len":0,"current":null,"repeat":"off","shuffle":false,"profile":"balanced","pid":1,"browser_pid":null,"backend":"test","browser_shared":false,"uptime_secs":0,"cache_entries":0,"message":"","sleep_remaining":null})).unwrap());
    for (key, expected) in [(KeyCode::Right, 60.0), (KeyCode::Left, 40.0)] {
        let request = app.key(KeyEvent::new(key, KeyModifiers::ALT)).unwrap().1;
        assert!(matches!(request, Some(Request::Seek(position)) if position==expected));
        assert_eq!(app.config.equalizer.bands_db[0], 0.0);
    }
    assert!(matches!(
        press(&mut app, KeyCode::Right),
        Some(Request::Equalizer(_))
    ));
    assert_eq!(app.config.equalizer.bands_db[0], 0.5);
}
