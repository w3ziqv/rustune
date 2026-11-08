use rustune::lyrics::Lyrics;
#[test]
fn lrc_handles_multiple_timestamps_fractional_seconds_and_global_offset() {
    let lyrics = Lyrics::parse(
        None,
        Some("[ar:Example]\n[00:01.5][00:03.250]First line\n[00:02.00]Middle\n[offset:-250]"),
        false,
    )
    .unwrap();
    assert!(lyrics.synced);
    assert_eq!(lyrics.lines.len(), 3);
    assert_eq!(lyrics.lines[0].at_ms, Some(1250));
    assert_eq!(lyrics.lines[2].at_ms, Some(3000));
    assert_eq!(lyrics.active_line(0.5), None);
    assert_eq!(lyrics.active_line(1.25), Some(0));
    assert_eq!(lyrics.active_line(2.0), Some(1));
    assert_eq!(lyrics.active_line(3.1), Some(2));
}
#[test]
fn invalid_timing_falls_back_to_plain_and_control_sequences_are_removed() {
    let lyrics = Lyrics::parse(
        Some("Our first line\nOur second line\u{1b}"),
        Some("[99:99]invalid timing"),
        false,
    )
    .unwrap();
    assert!(!lyrics.synced);
    assert_eq!(lyrics.lines[1].text, "Our second line");
    assert_eq!(lyrics.active_line(99.0), None);
    assert!(Lyrics::parse(Some(&"a".repeat(256 * 1024)), None, false).is_err());
}
#[test]
fn instrumental_is_distinct_from_missing_lyrics() {
    let lyrics = Lyrics::parse(None, None, true).unwrap();
    assert!(lyrics.instrumental);
    assert!(lyrics.lines.is_empty());
    assert!(Lyrics::parse(None, None, false).is_err());
}

#[tokio::test]
#[ignore = "real free LRCLIB endpoint; no credentials, browser or audio"]
async fn real_lrclib_lookup_returns_lyrics_without_an_api_key() {
    let track = rustune::model::Track {
        title: "Simple Man".into(),
        artist: "Lynyrd Skynyrd".into(),
        album: "(Pronounced 'Leh-'Nérd 'Skin-'Nérd)".into(),
        duration_ms: 357000,
        ..Default::default()
    };
    let query = rustune::lyrics::LyricsQuery::from_track(&track).unwrap();
    let lyrics = rustune::lyrics::Provider::new()
        .unwrap()
        .lookup(&query)
        .await
        .unwrap()
        .expect("no matching lyric record");
    assert!(!lyrics.lines.is_empty());
    assert!(lyrics.source_url.starts_with("https://lrclib.net/api/get/"));
    println!(
        "LRCLIB returned {} lines; synchronized={}; song text withheld from test output",
        lyrics.lines.len(),
        lyrics.synced
    );
}
