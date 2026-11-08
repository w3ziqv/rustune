use rustune::{
    cache::Cache,
    config::{Config, PerformanceMode},
    model::{Repeat, Track},
    queue::Queue,
};
fn track(id: &str) -> Track {
    Track {
        id: id.into(),
        kind: "songs".into(),
        ..Track::default()
    }
}
#[test]
fn config_rejects_invalid_volume_and_unknown_keys() {
    assert!(Config::parse("volume = 2.0").is_err());
    assert!(Config::parse("volume = nan").is_err());
    assert!(Config::parse("typo = true").is_err());
    assert_eq!(Config::parse("").unwrap().volume, 0.7);
}
#[test]
fn profiles_change_cache_and_work_frequency() {
    let m = PerformanceMode::Minimal.limits();
    let b = PerformanceMode::Balanced.limits();
    let e = PerformanceMode::Enhanced.limits();
    assert!(m.cache_entries < b.cache_entries && b.cache_entries < e.cache_entries);
    assert!(m.refresh_ms > b.refresh_ms && b.refresh_ms > e.refresh_ms);
}
#[test]
fn queue_mutations_preserve_current_identity() {
    let mut q = Queue::default();
    for id in ["a", "b", "c"] {
        q.push(track(id)).unwrap();
    }
    q.current = Some(1);
    assert!(q.move_item(1, 2));
    assert_eq!(q.current, Some(2));
    assert_eq!(q.tracks[q.current.unwrap()].id, "b");
    q.remove(0);
    assert_eq!(q.current, Some(1));
    q.remove(1);
    assert_eq!(q.current, None);
    assert!(q.remove(999).is_none());
}
#[test]
fn repeat_one_only_applies_to_automatic_advance() {
    let mut q = Queue::default();
    for id in ["a", "b"] {
        q.push(track(id)).unwrap();
    }
    q.current = Some(0);
    q.repeat = Repeat::One;
    assert_eq!(q.advance(false), Some(0));
    assert_eq!(q.advance(true), Some(1));
    q.repeat = Repeat::Off;
    assert_eq!(q.advance(false), None);
    q.repeat = Repeat::All;
    assert_eq!(q.advance(false), Some(0));
}
#[test]
fn cache_is_bounded_and_replaces_duplicate_keys() {
    let mut c = Cache::new(2);
    c.insert("a", 1);
    c.insert("a", 2);
    c.insert("b", 3);
    c.insert("c", 4);
    assert_eq!(c.entries.len(), 2);
    assert_eq!(c.get(&"a"), None);
    assert_eq!(c.get(&"c"), Some(&4));
    let mut off = Cache::new(0);
    off.insert(1, 1);
    assert!(off.entries.is_empty());
}

#[test]
fn removed_or_cleared_selection_cannot_resume_old_browser_metadata() {
    let mut q = Queue::default();
    let a = track("a");
    q.push(a.clone()).unwrap();
    q.current = Some(0);
    assert!(q.can_resume(Some(&a)));
    q.remove(0);
    assert!(!q.can_resume(Some(&a)));
    q.push(track("b")).unwrap();
    q.current = Some(0);
    assert!(!q.can_resume(Some(&a)));
}
#[test]
fn ten_thousand_song_queue_is_bounded_and_preserves_selection_under_reordering() {
    let mut q = Queue::default();
    for i in 0..10_000 {
        q.push(track(&i.to_string())).unwrap();
    }
    assert!(q.push(track("overflow")).is_err());
    q.current = Some(9000);
    q.move_item(9000, 5);
    q.remove(0);
    assert_eq!(q.tracks[q.current.unwrap()].id, "9000");
}

#[test]
fn monochrome_is_default_and_tokyo_night_is_selectable() {
    assert_eq!(Config::default().theme, "monochrome");
    assert!(Config::parse("theme = 'tokyo_night'").is_ok());
}

#[test]
fn sixel_is_rejected_without_verified_terminal_pixel_geometry() {
    assert!(Config::parse("artwork_protocol = 'sixel'").is_err());
}
