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
        app.nav, 1,
        "Down must move the navigation, not the song list"
    );
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Right);
    assert_eq!(app.screen, Screen::Search);
    press(&mut app, KeyCode::Char('/'));
    assert!(app.input == Input::Search);
    assert!(app.focus != Focus::Navigation);
}
#[test]
fn settings_are_actionable_using_arrows_and_enter() {
    let mut app = App::new(Config::default());
    app.go(Screen::Settings);
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
    app.screen = Screen::Search;
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
    app.screen = Screen::NowPlaying;
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
    app.screen = Screen::Search;
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
    assert!(app.focus != Focus::Navigation);
    assert_eq!(app.items[0].id, "123");
}
#[test]
fn page_up_returns_to_previous_catalog_page() {
    let mut app = App::new(Config::default());
    app.screen = Screen::Search;
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
        !matches!(app.focus, Focus::Player(_)),
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
    app.screen = Screen::Queue;
    app.items.push(Track {
        id: "old".into(),
        kind: "songs".into(),
        ..Track::default()
    });
    press(&mut app, KeyCode::Char('l'));
    assert_eq!(app.screen, Screen::Lyrics);
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
    app.screen = Screen::Search;
    let value = serde_json::json!({"items":[{"id":"old","kind":"songs","title":"Old queue"}]});
    app.accept(&Request::QueuePage { offset: 0 }, value.clone())
        .unwrap();
    assert!(
        app.items.is_empty(),
        "queue response must not replace current public search"
    );
    app.screen = Screen::Queue;
    app.queue_offset = 25;
    app.accept(&Request::QueuePage { offset: 0 }, value)
        .unwrap();
    assert!(
        app.items.is_empty(),
        "queue response offset must match the current page"
    );
}

fn assert_previous_catalog_page(screen: Screen) {
    let mut app = App::new(Config {
        performance: crate::config::PerformanceMode::Enhanced,
        ..Config::default()
    });
    app.screen = screen;
    app.account_generation = Some(42);
    app.kind = if screen == Screen::Library {
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
        serde_json::json!({"items":[], "next":"music:pl:75", "previous":"music:pl:25", "account_generation":42}),
    )
    .unwrap();
    assert!(
        matches!(press(&mut app, KeyCode::PageUp), Some(Request::Browse { next:Some(ref cursor),.. }) if cursor=="music:pl:25"),
        "screen {screen:?} must return one actual catalog page"
    );
}
#[test]
fn library_page_up_returns_to_the_previous_page() {
    assert_previous_catalog_page(Screen::Library);
}
#[test]
fn enhanced_search_page_up_returns_one_actual_catalog_page() {
    assert_previous_catalog_page(Screen::Search);
}
#[test]
fn equalizer_preserves_modified_seek_bindings_and_plain_arrows_edit_bands() {
    let mut app = App::new(Config::default());
    app.go(Screen::Equalizer);
    app.eq_selected = 2;
    app.status = Some(serde_json::from_value(serde_json::json!({"playback":{"position":50.0,"duration":120.0},"queue_len":0,"current":null,"repeat":"off","shuffle":false,"profile":"balanced","pid":1,"browser_pid":null,"backend":"test","browser_shared":false,"uptime_secs":0,"cache_entries":0,"message":"","sleep_remaining":null})).unwrap());
    for (key, expected) in [(KeyCode::Right, 10_000_000), (KeyCode::Left, -10_000_000)] {
        let request = app.key(KeyEvent::new(key, KeyModifiers::ALT)).unwrap().1;
        assert!(matches!(request, Some(Request::SeekBy(offset)) if offset==expected));
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
    app.screen = Screen::Search;
    app.focus = Focus::Content;
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
    app.screen = Screen::Search;
    app.focus = Focus::Content;
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
    app.screen = Screen::Lyrics;
    app.focus = Focus::Content;
    assert!(press(&mut app, KeyCode::Char('R')).is_none());
    assert_eq!(app.lyrics_generation, 1);
}

#[test]
fn playlists_open_whole_context_and_queue_save_requires_name_and_confirmation() {
    let mut app = App::new(Config::default());
    app.account_generation = Some(42);
    let request = app.go(Screen::Playlists).expect("Playlists loads metadata");
    assert!(matches!(request,Request::Browse{ref kind,..} if kind=="playlists"));
    app.accept(&request,serde_json::json!({"items":[{"id":"p.fixture","kind":"library-playlists","title":"Fixture","provider":"music","storefront":"pl","can_edit":true}],"next":null,"storefront":"pl","context":"Library","account_generation":42})).unwrap();
    let open = press(&mut app, KeyCode::Enter).unwrap();
    assert!(matches!(open, Request::Browse { next: None, .. }));
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
    app.screen = Screen::Queue;
    app.focus = Focus::Content;
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
        matches!(press(&mut app,KeyCode::Enter),Some(Request::PlaylistWrite{account_generation:Some(42),source:ipc::PlaylistWriteSource::Queue{..},action:crate::catalog::playlist_write::WriteAction::Create{name,..},..}) if name=="My queue")
    );
}
#[test]
fn cancelling_playlist_destination_restores_source_and_clears_pending_export() {
    let mut app = App::new(Config::default());
    app.screen = Screen::Search;
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
    assert_eq!(app.screen, Screen::Search);
    assert!(app.playlist_target.is_none());
}
#[test]
fn readonly_destination_and_cancelled_confirmation_never_emit_account_write() {
    let mut app = App::new(Config::default());
    app.go(Screen::Playlists);
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
    app.go(Screen::Playlists);
    app.previous = Some("music:pl:25".into());
    assert!(
        matches!(press(&mut app,KeyCode::PageUp),Some(Request::Browse{next:Some(ref cursor),..}) if cursor=="music:pl:25")
    );
}
#[test]
fn search_from_playlist_discards_previous_context_before_playing_result() {
    let mut app = App::new(Config::default());
    app.go(Screen::Playlists);
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
        account_generation: Some(42),
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

#[test]
fn clean_navigation_has_seven_destinations_and_lyrics_never_requests_artwork() {
    let mut app = App::new(Config::default());
    assert_eq!(
        SCREENS,
        &[
            "Search",
            "Library",
            "Playlists",
            "Queue",
            "Now Playing",
            "Lyrics",
            "Settings"
        ]
    );
    press(&mut app, KeyCode::Char('6'));
    app.inspected_track = Some(Track {
        id: "fixture".into(),
        title: "Song".into(),
        artist: "Artist".into(),
        artwork: Some("https://is1-ssl.mzstatic.com/cover.jpg".into()),
        ..Track::default()
    });
    assert!(app.artwork_request(Size::new(142, 39)).is_none());
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(142, 39)).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("LYRICS"));
    assert!(!text.contains("Loading artwork"));
}

#[test]
fn escape_restores_search_after_settings_without_losing_results() {
    let mut app = App::new(Config::default());
    app.query = "remember".into();
    app.items = vec![Track {
        id: "one".into(),
        title: "Preserved result".into(),
        ..Track::default()
    }];
    press(&mut app, KeyCode::Char(','));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.query, "remember");
    assert_eq!(app.items.len(), 1);
}

#[test]
fn failed_catalog_page_can_retry_without_discarding_results() {
    let mut app = App::new(Config::default());
    app.query = "song".into();
    app.items = vec![Track {
        id: "one".into(),
        ..Track::default()
    }];
    app.next = Some("music:pl:25".into());
    let request = press(&mut app, KeyCode::PageDown).unwrap();
    app.request_failed(&request);
    assert_eq!(app.items[0].id, "one");
    assert!(
        matches!(press(&mut app, KeyCode::Char('R')), Some(Request::Browse { next: Some(cursor), .. }) if cursor == "music:pl:25")
    );
}

#[test]
fn returning_to_pending_and_failed_search_preserves_retry_request() {
    let mut app = App::new(Config::default());
    app.query = "pending".into();
    let request = Request::Browse {
        kind: app.kind.clone(),
        term: app.query.clone(),
        next: Some("music:pl:25".into()),
    };
    app.browse_state = BrowseState::Loading;
    app.browse_retry = Some(request.clone());
    app.go(Screen::Settings);
    assert!(
        matches!(press(&mut app, KeyCode::Esc), Some(Request::Browse { next: Some(cursor), .. }) if cursor == "music:pl:25")
    );
    app.request_failed(&request);
    app.go(Screen::Settings);
    press(&mut app, KeyCode::Esc);
    assert!(
        matches!(press(&mut app, KeyCode::Char('R')), Some(Request::Browse { next: Some(cursor), .. }) if cursor == "music:pl:25")
    );
}

#[test]
fn lyrics_preview_and_scroll_survive_settings_round_trip() {
    let mut app = App::new(Config::default());
    app.screen = Screen::Lyrics;
    app.inspected_track = Some(Track {
        id: "preview".into(),
        title: "Preview song".into(),
        artist: "Artist".into(),
        ..Track::default()
    });
    app.lyrics_scroll = Some(8);
    app.go(Screen::Settings);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.displayed_track().unwrap().id, "preview");
    assert_eq!(app.lyrics_scroll, Some(8));
}

#[test]
fn reconnect_clears_stale_disconnected_banner() {
    let mut app = App::new(Config::default());
    app.connected = false;
    app.note = "Service disconnected; reconnecting…".into();
    let status=serde_json::from_value(serde_json::json!({"playback":{},"queue_len":0,"current":null,"repeat":"off","shuffle":false,"profile":"balanced","pid":1,"browser_pid":null,"backend":"test","browser_shared":false,"uptime_secs":0,"cache_entries":0,"message":"","sleep_remaining":null})).unwrap();
    app.on_status(status);
    assert!(app.connected);
    assert!(!app.status_message().contains("disconnected"));
}

#[test]
fn audio_failure_does_not_hide_command_input() {
    let mut app = App::new(Config::default());
    app.status = Some(serde_json::from_value(serde_json::json!({"playback":{},"queue_len":0,"current":null,"repeat":"off","shuffle":false,"profile":"balanced","pid":1,"browser_pid":null,"backend":"native","browser_shared":false,"uptime_secs":0,"cache_entries":0,"message":"","sleep_remaining":null,"audio_error":"Playback failed"})).unwrap());
    for screen in [Screen::Search, Screen::NowPlaying] {
        app.screen = screen;
        press(&mut app, KeyCode::Char(':'));
        for c in "help".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains(":help"));
        press(&mut app, KeyCode::Esc);
    }
}

#[test]
fn explicit_lyrics_preview_is_labeled_even_for_the_current_song() {
    let mut app = App::new(Config::default());
    app.status = Some(serde_json::from_value(serde_json::json!({"playback":{"track":{"id":"song","title":"Song"}},"queue_len":1,"current":0,"repeat":"off","shuffle":false,"profile":"balanced","pid":1,"browser_pid":null,"backend":"native","browser_shared":false,"uptime_secs":0,"cache_entries":0,"message":"","sleep_remaining":null})).unwrap());
    app.items = vec![Track {
        id: "song".into(),
        title: "Song".into(),
        ..Track::default()
    }];
    press(&mut app, KeyCode::Char('l'));
    assert!(app.lyrics_preview());
}

#[tokio::test]
async fn audio_information_shows_local_checks_without_claiming_playback() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths {
        config: dir.path().join("config"),
        data: dir.path().join("data"),
        cache: dir.path().join("cache"),
        runtime: dir.path().join("runtime"),
    };
    crate::config::private_write(&paths.config.join("music-user-token"), b"fixture-token").unwrap();
    let mut app = App::new(Config::default());
    app.go(Screen::Settings);
    app.selected = app
        .settings_rows()
        .iter()
        .position(|row| row == "Audio information")
        .unwrap();
    assert!(press(&mut app, KeyCode::Enter).is_none());
    assert_eq!(app.screen, Screen::AudioInfo);
    app.diagnostics = Some(crate::diagnostics::inspect(&paths).await);

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| app.render(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    let rendered = (0..30)
        .map(|y| {
            (0..100)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains("saved locally · account acceptance unverified"));
    assert!(rendered.contains("CDM file: missing"));
    assert!(rendered.contains("Audio helper:"));
    assert!(rendered.contains("Service: not running"));
    assert!(rendered.contains("Session D-Bus:"));
    assert!(rendered.contains("Source quality unknown"));
    assert!(rendered.contains("drm/libwidevinecdm.so"));
    assert!(!rendered.contains("fixture-token"));
    press(&mut app, KeyCode::Down);
    assert_eq!(app.audio_info_scroll, 1);
    assert!(press(&mut app, KeyCode::Char('R')).is_none());
    assert!(app.diagnostics.is_none());
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Settings);
}

#[test]
fn playlist_window_prefetches_and_preserves_duplicate_occurrences() {
    let mut app = App::new(Config::default());
    app.viewport = Size::new(120, 42);
    app.screen = Screen::Playlists;
    app.kind = "open:library-playlists:p.fixture".into();
    app.account_generation = Some(42);
    app.playlist_view = Some(crate::catalog::playlist::PlaylistRef {
        id: "p.fixture".into(),
        storefront: "pl".into(),
        title: "Fixture".into(),
        can_edit: false,
    });
    let payload = |start, end, next: Option<&str>| {
        let items = (start..end)
            .map(|i| {
                let id = match i {
                    25 => 24,
                    50 => 49,
                    _ => i,
                };
                serde_json::json!({"id":id.to_string(),"kind":"library-songs","source_index":i})
            })
            .collect::<Vec<_>>();
        serde_json::json!({"items":items,"next":next,"storefront":"pl","account_generation":42})
    };
    let initial = app.page_request();
    app.accept(
        &initial,
        payload(0, 25, Some("music:pl:1234567890abcdef:25")),
    )
    .unwrap();
    let (jobs, mut work) = mpsc::channel(1);
    for _ in 0..19 {
        assert!(press(&mut app, KeyCode::Down).is_none());
    }
    let second = press(&mut app, KeyCode::Down).expect("prefetch before last row");
    app.dispatch(&jobs, second.clone());
    assert!(work.try_recv().is_ok());
    for _ in 20..24 {
        assert!(press(&mut app, KeyCode::Down).is_none());
    }
    app.request_failed(&second);
    assert_eq!(app.selected, 24);
    assert_eq!(app.playlist_range(30), 0..25);
    assert!(press(&mut app, KeyCode::Down).is_none());
    let retry = press(&mut app, KeyCode::Char('R')).unwrap();
    assert_eq!(
        serde_json::to_value(&retry).unwrap(),
        serde_json::to_value(&second).unwrap()
    );
    app.accept(
        &retry,
        payload(25, 50, Some("music:pl:1234567890abcdef:50")),
    )
    .unwrap();
    assert_eq!(app.playlist_range(30), 0..25);
    assert!(press(&mut app, KeyCode::Down).is_none());
    assert_eq!(app.playlist_range(30), 1..26);
    assert_eq!(app.items[24].id, app.items[25].id);
    assert!(press(&mut app, KeyCode::Down).is_none());
    assert_eq!(app.playlist_range(30), 2..27);

    for _ in 26..44 {
        assert!(press(&mut app, KeyCode::Down).is_none());
    }
    let third = press(&mut app, KeyCode::Down).expect("prefetch third page");
    app.dispatch(&jobs, third.clone());
    assert!(work.try_recv().is_ok());
    for _ in 45..49 {
        assert!(press(&mut app, KeyCode::Down).is_none());
    }
    app.accept(&third, payload(50, 56, None)).unwrap();
    assert_eq!(app.playlist_range(30), 25..50);
    assert!(press(&mut app, KeyCode::Down).is_none());
    assert_eq!(app.playlist_range(30), 26..51);
    assert_eq!(app.items[49].id, app.items[50].id);
    let selected = app.song_request(app.items[app.selected].clone(), 2);
    assert!(matches!(
        selected,
        Request::Playlist {
            action: ipc::ContextAction::PlayFromPosition {
                source_index: 50,
                ..
            },
            ..
        }
    ));
    for _ in 0..8 {
        assert!(press(&mut app, KeyCode::Down).is_none());
    }
    assert_eq!(app.selected, 55);
    assert_eq!(app.playlist_range(30), 31..56);
    for _ in 31..55 {
        press(&mut app, KeyCode::Up);
    }
    assert_eq!(app.playlist_range(30), 31..56);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.playlist_range(30), 30..55);
    for _ in 0..31 {
        press(&mut app, KeyCode::Up);
    }
    assert_eq!(app.selected, 0);
    assert_eq!(app.playlist_range(30), 0..25);
    app.accept(&initial, payload(0, 7, None)).unwrap();
    assert_eq!(app.playlist_range(30), 0..7);
}

#[test]
fn account_change_discards_private_navigation_and_rejects_old_first_page() {
    let mut app = App::new(Config::default());
    app.query = "Public search".into();
    app.items = vec![Track {
        id: "public".into(),
        ..Track::default()
    }];
    app.account_generation = Some(42);
    let initial = app.go(Screen::Playlists).unwrap();
    let payload = |generation| {
        serde_json::json!({
            "account_generation": generation,
            "storefront": "pl",
            "items": [{"id":"p.private","kind":"library-playlists","storefront":"pl"}],
            "next": null
        })
    };
    app.accept(&initial, payload(42)).unwrap();
    assert_eq!(app.items.len(), 1);
    app.go(Screen::Settings);
    assert_eq!(app.back_stack.len(), 2);
    app.account_generation = Some(43);
    app.invalidate_private_views();
    assert!(app.restore_view());
    assert_eq!(app.screen, Screen::Search);
    assert_eq!(app.items[0].id, "public");

    let next = app.go(Screen::Playlists).unwrap();
    assert!(app.accept(&next, payload(42)).is_err());
    assert!(app.items.is_empty());
    app.accept(&next, payload(43)).unwrap();
    assert_eq!(app.items.len(), 1);
    app.account_generation = None;
    app.invalidate_private_views();
    assert!(app.items.is_empty());
    assert!(app.next.is_none() && app.browse_retry.is_none());
}

#[test]
fn oversized_playlist_stops_loading_and_history_shares_its_budget() {
    let mut app = App::new(Config::default());
    app.screen = Screen::Playlists;
    app.kind = "open:library-playlists:p.old".into();
    app.account_generation = Some(42);
    app.playlist_bytes = crate::catalog::album::MAX_ALBUM_BYTES - 10;
    app.items.push(Track {
        id: "1".into(),
        ..Track::default()
    });
    app.next = Some("music:pl:000000000000002a:25".into());
    let next = press(&mut app, KeyCode::PageDown).unwrap();
    app.accept(&next, serde_json::json!({
        "account_generation":42, "items":[{"id":"2","kind":"library-songs"}], "next":"music:pl:000000000000002a:50"
    })).unwrap();
    assert_eq!(app.items.len(), 1);
    assert!(app.note.contains("viewing limit"));
    assert!(app.next.is_none() && app.browse_retry.is_none());
    assert!(press(&mut app, KeyCode::PageDown).is_none());
    assert!(press(&mut app, KeyCode::Char('R')).is_none());

    app.remember_view();
    assert!(app.items.is_empty());
    app.kind = "open:library-playlists:p.new".into();
    let fresh = app.page_request();
    app.accept(
        &fresh,
        serde_json::json!({
            "account_generation":42, "items":[{"id":"42","kind":"library-songs"}], "next":null
        }),
    )
    .unwrap();
    assert_eq!(app.items[0].id, "42");
    assert!(
        app.back_stack.is_empty(),
        "the prior 12 MiB view would exceed the shared budget"
    );
}

#[test]
fn local_rotation_clears_private_rows_before_drawing_or_accepting_an_action() {
    let directory = tempfile::tempdir().unwrap();
    let paths = crate::catalog::playlist_tests::paths(directory.path());
    let mut app = App::new(Config::default());
    app.account_generation = auth::token_stamp(&paths).map(auth::TokenStamp::generation);
    for revoke_before_action in [false, true] {
        let first = app.go(Screen::Playlists).unwrap();
        app.accept(&first, serde_json::json!({
            "account_generation":app.account_generation, "storefront":"pl", "next":null,
            "items":[{"id":"p.private","title":"Previous account's playlist","kind":"library-playlists","can_edit":true}]
        })).unwrap();
        let token = auth::SecretToken::parse(if revoke_before_action {
            "third-fixture"
        } else {
            "second-fixture"
        })
        .unwrap();
        auth::save_user_token(&paths, &token).unwrap();
        let lock = app.private_view_guard(&paths);
        if revoke_before_action {
            assert!(press(&mut app, KeyCode::Enter).is_none());
            assert!(press(&mut app, KeyCode::Char('a')).is_none());
        } else {
            let backend = ratatui::backend::TestBackend::new(120, 40);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal.draw(|frame| app.render(frame)).unwrap();
            let screen: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(!screen.contains("Previous account's playlist"));
        }
        assert!(app.items.is_empty());
        assert_eq!(
            app.back_stack.len(),
            1,
            "the public Search history remains available"
        );
        drop(lock);
        app.account_generation = auth::token_stamp(&paths).map(auth::TokenStamp::generation);
    }
}
