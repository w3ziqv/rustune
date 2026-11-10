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
        Some(Request::QueueAction {
            action: ipc::QueueAction::Append { .. },
            ..
        })
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

#[test]
fn album_actions_are_visible_from_results_and_details_and_song_play_keeps_album() {
    let mut app = App::new(Config::default());
    app.screen = 2;
    app.sidebar = false;
    app.kind = "search:albums".into();
    app.query = "Album".into();
    app.items = vec![Track {
        id: "42".into(),
        kind: "albums".into(),
        title: "Album".into(),
        provider: "music".into(),
        storefront: "pl".into(),
        ..Track::default()
    }];
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.album_actions.is_some());
    let (_, request) = app
        .key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(matches!(
        request,
        Some(Request::Album {
            action: crate::ipc::AlbumAction::Play { shuffle: false, .. },
            ..
        })
    ));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(app.kind.starts_with("open:albums:"));
    app.accept(&Request::Browse{kind:app.kind.clone(),term:String::new(),next:None},serde_json::json!({"items":[{"id":"12","kind":"songs","title":"Song 12"}],"next":"music:pl:25","storefront":"pl","context":"Album"})).unwrap();
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    let (_, request) = app
        .key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(
        matches!(request,Some(Request::Album{action:crate::ipc::AlbumAction::Play{start_song:Some(id),..},..}) if id=="12")
    );
    assert_eq!(app.next.as_deref(), Some("music:pl:25"));
    app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE))
        .unwrap();
    app.request_failed(&Request::Browse {
        kind: app.kind.clone(),
        term: String::new(),
        next: Some("music:pl:25".into()),
    });
    assert!(matches!(
        app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE))
            .unwrap()
            .1,
        Some(Request::Browse { next: Some(_), .. })
    ));
}

#[test]
fn opening_album_pins_details_to_selected_source_and_storefront() {
    let mut app = App::new(Config::default());
    app.screen = 2;
    app.sidebar = false;
    app.kind = "search:albums".into();
    app.items = vec![Track {
        id: "42".into(),
        kind: "albums".into(),
        provider: "itunes".into(),
        storefront: "us".into(),
        ..Track::default()
    }];
    assert!(
        matches!(press(&mut app,KeyCode::Enter),Some(Request::Browse{next:Some(cursor),..}) if cursor=="itunes:us:0")
    );
}

#[test]
fn album_retry_binding_preserves_lyrics_reload() {
    let mut app = App::new(Config::default());
    app.screen = 8;
    app.sidebar = false;
    assert!(press(&mut app, KeyCode::Char('R')).is_none());
    assert_eq!(app.lyrics_generation, 1);
}

#[test]
fn playlists_open_whole_context_and_queue_save_requires_name_and_confirmation() {
    let mut app = App::new(Config::default());
    let request = app.go(6).expect("Playlists loads metadata");
    assert!(matches!(request,Request::Browse{ref kind,..} if kind=="playlists"));
    app.accept(&request,serde_json::json!({"items":[{"id":"p.fixture","kind":"library-playlists","title":"Fixture","provider":"music","storefront":"pl","can_edit":true}],"next":null,"storefront":"pl","context":"Library"})).unwrap();
    let open = press(&mut app, KeyCode::Enter).unwrap();
    assert!(matches!(open,Request::Browse{next:Some(ref cursor),..} if cursor=="music:pl:0"));
    assert!(app.playlist_view.is_some());
    press(&mut app, KeyCode::Char('a'));
    let play = press(&mut app, KeyCode::Enter).unwrap();
    assert!(matches!(
        play,
        Request::Playlist {
            action: ipc::ContextAction::Play { shuffle: false, .. },
            ..
        }
    ));
    app.screen = 7;
    app.sidebar = false;
    assert!(press(&mut app, KeyCode::Char('S')).is_none());
    assert!(app.playlist_dialog.is_some());
    for c in "My queue".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    assert!(
        press(&mut app, KeyCode::Enter).is_none(),
        "Name entry does not send account POST"
    );
    assert!(
        matches!(press(&mut app,KeyCode::Enter),Some(Request::PlaylistWrite{source:ipc::PlaylistWriteSource::Queue{..},action:crate::catalog::playlist_write::WriteAction::Create{name,..},..}) if name=="My queue")
    );
}
#[test]
fn cancelling_playlist_destination_restores_source_and_clears_pending_export() {
    let mut app = App::new(Config::default());
    app.screen = 2;
    app.kind = "search:songs".into();
    app.items.push(Track {
        id: "1".into(),
        kind: "songs".into(),
        ..Default::default()
    });
    app.playlist_destination(ipc::PlaylistWriteSource::Track {
        track: Box::new(app.items[0].clone()),
    });
    press(&mut app, KeyCode::Left);
    assert_eq!(app.screen, 2);
    assert!(app.playlist_target.is_none());
}
#[test]
fn readonly_destination_and_cancelled_confirmation_never_emit_account_write() {
    let mut app = App::new(Config::default());
    app.go(6);
    app.items.push(Track {
        id: "p.fixture".into(),
        kind: "library-playlists".into(),
        title: "Synthetic".into(),
        storefront: "pl".into(),
        can_edit: Some(false),
        ..Default::default()
    });
    app.playlist_target = Some(ipc::PlaylistWriteSource::Empty);
    assert!(press(&mut app, KeyCode::Enter).is_none());
    assert!(app.playlist_dialog.is_none());
    assert!(app.note.contains("read-only"));
    app.items[0].can_edit = Some(true);
    assert!(press(&mut app, KeyCode::Enter).is_none());
    assert!(app.playlist_dialog.is_some());
    assert!(press(&mut app, KeyCode::Esc).is_none());
    app.playlist_name(false);
    app.playlist_paste(&"ą\n".repeat(300));
    if let Some(playlists::PlaylistDialog::Name { text, .. }) = &app.playlist_dialog {
        assert_eq!(text.len(), 200);
        assert!(!text.contains('\n'));
    } else {
        panic!("name dialog")
    };
    press(&mut app, KeyCode::Enter);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 32)).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    let buffer = terminal.backend().buffer();
    let text = (0..32)
        .flat_map(|y| (0..100).map(move |x| buffer[(x, y)].symbol()))
        .collect::<String>();
    assert!(text.contains("CONFIRM ACCOUNT WRITE"));
    assert!(text.contains("Enter SEND"));
    assert!(press(&mut app, KeyCode::Esc).is_none());
}
#[test]
fn playlist_previous_page_preserves_private_cursor() {
    let mut app = App::new(Config::default());
    app.go(6);
    app.previous = Some("music:pl:25".into());
    assert!(
        matches!(press(&mut app,KeyCode::PageUp),Some(Request::Browse{next:Some(ref cursor),..}) if cursor=="music:pl:25")
    );
}
#[test]
fn search_from_playlist_discards_previous_context_before_playing_result() {
    let mut app = App::new(Config::default());
    app.go(6);
    app.playlist_view = Some(crate::catalog::playlist::PlaylistRef {
        id: "p.old".into(),
        storefront: "pl".into(),
        title: "Old".into(),
        can_edit: true,
    });
    press(&mut app, KeyCode::Char('/'));
    press(&mut app, KeyCode::Char('x'));
    press(&mut app, KeyCode::Enter);
    assert!(app.playlist_view.is_none());
    assert!(app.playlist_target.is_none());
    let request = app.song_request(
        Track {
            id: "7".into(),
            kind: "songs".into(),
            ..Default::default()
        },
        2,
    );
    assert!(matches!(
        request,
        Request::QueueAction {
            action: ipc::QueueAction::PlayContext { .. },
            ..
        }
    ));
}
#[test]
fn delayed_playlist_write_ack_cannot_regress_subscribed_terminal_status() {
    let mut app = App::new(Config::default());
    let value = |phase| serde_json::json!({"playback":{},"queue_len":3,"current":null,"repeat":"off","shuffle":false,"profile":"balanced","pid":1,"browser_pid":null,"backend":"test","browser_shared":false,"uptime_secs":0,"cache_entries":0,"message":"","sleep_remaining":null,"playlist_write":{"id":"key","phase":phase,"tracks":3,"result":null,"message":"Synthetic","can_retry":false}});
    app.on_status(serde_json::from_value(value("completed")).unwrap());
    let request = Request::PlaylistWrite {
        key: "key".into(),
        source: ipc::PlaylistWriteSource::Empty,
        action: crate::catalog::playlist_write::WriteAction::Create {
            name: "Synthetic".into(),
            description: String::new(),
        },
    };
    app.accept(&request, value("preparing")).unwrap();
    assert_eq!(
        app.status
            .as_ref()
            .unwrap()
            .playlist_write
            .as_ref()
            .unwrap()
            .phase,
        "completed"
    );
}
