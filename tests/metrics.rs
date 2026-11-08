use rustune::metrics::{parse_memory, parse_stat};
#[test]
fn parses_proc_units_and_unknown_pss() {
    let m = parse_memory("Rss: 1024 kB\nPss: 512 kB\nPrivate_Clean: 4 kB\n");
    assert_eq!(m.rss_kib, 1024);
    assert_eq!(m.pss_kib, Some(512));
    assert_eq!(parse_memory("Rss: 42 kB").pss_kib, None);
}
#[test]
fn proc_stat_handles_spaces_and_parentheses_in_process_names() {
    let s = "123 (chrome (renderer)) S 12 123 1 0 0 0 0 0 0 0 30 20 0 0";
    assert_eq!(parse_stat(s), Some((12, 123, 50)));
    assert_eq!(parse_stat("malformed"), None);
}

#[test]
fn exited_process_ticks_do_not_erase_live_cpu_and_new_processes_are_counted() {
    let before = [(1, 100), (2, 100)].into_iter().collect();
    let after = [(1, 110), (3, 5)].into_iter().collect();
    assert_eq!(rustune::metrics::tick_delta(&before, &after), 15);
}
