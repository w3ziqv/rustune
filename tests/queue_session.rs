use rustune::{
    model::{Repeat, Track},
    queue::{Queue, QueueContext},
};
fn song(id: &str) -> Track {
    Track {
        id: id.into(),
        kind: "songs".into(),
        ..Track::default()
    }
}
fn ids(q: &Queue) -> Vec<&str> {
    q.tracks.iter().map(|e| e.id.as_str()).collect()
}
fn session() -> Queue {
    let mut q = Queue::default();
    q.replace(
        vec![song("a"), song("b"), song("c")],
        0,
        false,
        QueueContext::single(),
    )
    .unwrap();
    q
}
#[test]
fn priority_blocks_are_fifo_even_when_shuffled_and_idle_add_does_not_select() {
    let mut q = session();
    q.set_shuffle(true);
    q.insert_next(vec![song("x"), song("x2")], QueueContext::single())
        .unwrap();
    q.insert_next(vec![song("y")], QueueContext::single())
        .unwrap();
    for id in ["x", "x2", "y"] {
        q.advance(true);
        assert_eq!(q.current_track().unwrap().id, id);
    }
    let mut idle = Queue::default();
    idle.insert_next(vec![song("x")], QueueContext::single())
        .unwrap();
    idle.append(vec![song("y")], QueueContext::single())
        .unwrap();
    assert_eq!(idle.current, None);
    assert_eq!(ids(&idle), ["x", "y"]);
}
#[test]
fn duplicate_occurrences_have_distinct_durable_ids_and_moves_preserve_current() {
    let mut q = session();
    q.append(vec![song("a")], QueueContext::single()).unwrap();
    let first = q.tracks[0].entry_id;
    let duplicate = q.tracks[3].entry_id;
    assert_ne!(first, duplicate);
    q.move_item(0, 2);
    assert_eq!(q.current_entry_id(), Some(first));
    let restored: Queue = serde_json::from_slice(&serde_json::to_vec(&q).unwrap()).unwrap();
    assert_eq!(restored.tracks[3].entry_id, duplicate);
}
#[test]
fn shuffle_is_one_stable_cycle_and_previous_next_uses_heard_history() {
    let mut q = session();
    q.set_shuffle(true);
    let order = ids(&q).into_iter().map(str::to_owned).collect::<Vec<_>>();
    q.advance(true);
    let second = q.current_entry_id();
    q.advance(true);
    let third = q.current_entry_id();
    q.previous();
    assert_eq!(q.current_entry_id(), second);
    q.advance(true);
    assert_eq!(q.current_entry_id(), third);
    assert!(q.advance(true).is_none());
    assert_eq!(
        ids(&q),
        order.iter().map(String::as_str).collect::<Vec<_>>()
    );
    q.repeat = Repeat::All;
    assert_eq!(q.advance(true), Some(0));
}
#[test]
fn clear_upcoming_preserves_modes_and_overrides_repeat_until_new_addition() {
    let mut q = session();
    q.repeat = Repeat::One;
    q.set_shuffle(true);
    let current = q.current_entry_id();
    q.clear_upcoming();
    assert_eq!(q.current_entry_id(), current);
    assert!(q.shuffle);
    assert_eq!(q.repeat, Repeat::One);
    assert!(q.candidate(false).is_none());
    q.append(vec![song("z")], QueueContext::single()).unwrap();
    q.repeat = Repeat::Off;
    q.advance(false);
    assert_eq!(q.current_track().unwrap().id, "z");
}
#[test]
fn album_start_from_middle_keeps_earlier_entries_for_repeat_all() {
    let mut q = Queue::default();
    q.replace(
        (0..225).map(|i| song(&i.to_string())).collect(),
        124,
        false,
        QueueContext::single(),
    )
    .unwrap();
    assert_eq!(q.tracks.len(), 225);
    for i in 125..225 {
        q.advance(false);
        assert_eq!(q.current_track().unwrap().id, i.to_string());
    }
    assert!(q.advance(false).is_none());
    q.repeat = Repeat::All;
    q.advance(false);
    assert_eq!(q.current_track().unwrap().id, "0");
}

#[test]
fn shuffle_toggle_preserves_priority_and_manual_reorder_changes_next() {
    let mut q = session();
    q.set_shuffle(true);
    q.insert_next(vec![song("x"), song("y")], QueueContext::single())
        .unwrap();
    q.set_shuffle(false);
    assert_eq!(ids(&q), ["a", "x", "y", "b", "c"]);
    q.move_item(4, 1);
    q.advance(true);
    assert_eq!(q.current_track().unwrap().id, "c");
    q.repeat = Repeat::One;
    assert_eq!(q.advance(false), Some(1));
    q.advance(true);
    assert_eq!(q.current_track().unwrap().id, "x");
    let x = q.current_entry_id().unwrap();
    q.previous();
    assert_eq!(q.current_track().unwrap().id, "c");
    q.advance(true);
    assert_eq!(q.current_entry_id(), Some(x));
}

#[test]
fn new_priority_after_previous_precedes_forward_history_without_losing_it() {
    let mut q = session();
    q.advance(true);
    q.advance(true);
    q.previous();
    q.insert_next(vec![song("x")], QueueContext::single())
        .unwrap();
    q.advance(true);
    assert_eq!(q.current_track().unwrap().id, "x");
    q.advance(true);
    assert_eq!(q.current_track().unwrap().id, "c");
}

#[test]
fn ordinary_previous_after_manual_jump_uses_adjacent_entry() {
    let mut q = session();
    q.select(2);
    q.previous();
    assert_eq!(q.current_track().unwrap().id, "b");
    q.advance(true);
    assert_eq!(q.current_track().unwrap().id, "c");
}

#[test]
fn projected_order_exposes_forward_history_and_priority_in_what_will_play() {
    let mut q = session();
    q.set_shuffle(true);
    q.select(2);
    q.previous();
    let target = q.tracks[2].entry_id;
    q.insert_next(vec![song("x")], QueueContext::single())
        .unwrap();
    let x = q.tracks[1].entry_id;
    assert_eq!(q.upcoming_ids(), vec![x, target]);
}
