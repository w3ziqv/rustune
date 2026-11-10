//! Linux /proc measurements. RSS is summed; PSS is reported only when all reads succeed.
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    time::{Duration, Instant},
};
#[derive(Debug, Default, Clone, Serialize)]
pub struct Memory {
    pub rss_kib: u64,
    pub pss_kib: Option<u64>,
}
pub fn parse_memory(s: &str) -> Memory {
    let mut m = Memory::default();
    for line in s.lines() {
        let mut w = line.split_whitespace();
        match w.next() {
            Some("Rss:") => m.rss_kib = w.next().and_then(|n| n.parse().ok()).unwrap_or(0),
            Some("Pss:") => m.pss_kib = w.next().and_then(|n| n.parse().ok()),
            _ => {}
        }
    }
    m
}
#[cfg(test)]
mod helper_tests {
    use super::*;
    #[test]
    fn native_measurement_includes_the_separate_audio_helper() {
        let status:crate::service::Status=serde_json::from_value(serde_json::json!({"playback":{},"queue_len":0,"current":null,"repeat":"off","shuffle":false,"profile":"balanced","pid":11,"audio_pid":12,"browser_pid":null,"backend":"native","browser_shared":false,"uptime_secs":0,"cache_entries":0,"message":"","sleep_remaining":null})).unwrap();
        let map = BTreeMap::from([(11, (1, 11, 0)), (12, (11, 12, 0))]);
        let (native, browser) = members(&map, Some(&status));
        assert!(native.contains(&11) && native.contains(&12));
        assert!(browser.is_empty());
    }
}
/// (parent PID, process group, user + system ticks); comm may contain ')' or spaces.
pub fn parse_stat(s: &str) -> Option<(u32, u32, u64)> {
    let end = s.rfind(')')?;
    let w: Vec<_> = s[end + 1..].split_whitespace().collect();
    Some((
        w.get(1)?.parse().ok()?,
        w.get(2)?.parse().ok()?,
        w.get(11)?
            .parse::<u64>()
            .ok()?
            .checked_add(w.get(12)?.parse().ok()?)?,
    ))
}
fn memory(pid: u32) -> Option<Memory> {
    if let Ok(s) = fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) {
        return Some(parse_memory(&s));
    }
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let rss = status
        .lines()
        .find(|l| l.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some(Memory {
        rss_kib: rss,
        pss_kib: None,
    })
}
fn processes() -> BTreeMap<u32, (u32, u32, u64)> {
    let mut map = BTreeMap::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return map;
    };
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if let Ok(s) = fs::read_to_string(entry.path().join("stat"))
            && let Some(stat) = parse_stat(&s)
        {
            map.insert(pid, stat);
        }
    }
    map
}
fn sum_memory(pids: &[u32]) -> Memory {
    let mut total = Memory {
        rss_kib: 0,
        pss_kib: Some(0),
    };
    for pid in pids {
        if let Some(m) = memory(*pid) {
            total.rss_kib += m.rss_kib;
            total.pss_kib = total.pss_kib.zip(m.pss_kib).map(|(a, b)| a + b);
        } else {
            total.pss_kib = None;
        }
    }
    total
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub methodology: &'static str,
    pub native_pids: Vec<u32>,
    pub browser_pids: Vec<u32>,
    pub native: Memory,
    pub browser: Memory,
    pub total: Memory,
    pub cpu_percent_one_core: f64,
    pub native_cpu_percent_one_core: f64,
    pub browser_cpu_percent_one_core: f64,
    pub cpu_sample_seconds: f64,
    pub backend: String,
    pub browser_shared: bool,
    pub uptime_seconds: u64,
    pub cache_entries: usize,
}
pub fn tick_delta(before: &BTreeMap<u32, u64>, after: &BTreeMap<u32, u64>) -> u64 {
    after
        .iter()
        .map(|(pid, ticks)| ticks.saturating_sub(before.get(pid).copied().unwrap_or(0)))
        .sum()
}
fn members(
    map: &BTreeMap<u32, (u32, u32, u64)>,
    status: Option<&crate::service::Status>,
) -> (Vec<u32>, Vec<u32>) {
    let exe = std::env::current_exe().ok();
    let me = std::process::id();
    let mut native: Vec<_> = map
        .keys()
        .filter(|pid| {
            **pid != me
                && exe.is_some()
                && fs::read_link(format!("/proc/{pid}/exe"))
                    .ok()
                    .and_then(|p| {
                        p.file_name().map(|v| {
                            v.to_string_lossy()
                                .trim_end_matches(" (deleted)")
                                .to_owned()
                        })
                    })
                    .as_deref()
                    == Some("rustune")
        })
        .copied()
        .collect();
    if let Some(s) = status
        && map.contains_key(&s.pid)
        && !native.contains(&s.pid)
    {
        native.push(s.pid);
    }
    if let Some(pid) = status.and_then(|s| s.audio_pid)
        && map.contains_key(&pid)
        && !native.contains(&pid)
    {
        native.push(pid);
    }
    if native.is_empty() {
        native.push(me);
    }
    let mut browser = Vec::new();
    if let Some(root) = status.and_then(|s| s.browser_pid) {
        if map.contains_key(&root) {
            browser.push(root);
        }
        for _ in 0..32 {
            let mut added = false;
            for (pid, (parent, group, _)) in map {
                if !browser.contains(pid)
                    && (browser.contains(parent)
                        || (status.is_some_and(|s| !s.browser_shared) && *group == root))
                {
                    browser.push(*pid);
                    added = true;
                }
            }
            if !added {
                break;
            }
        }
    }
    browser.retain(|pid| !native.contains(pid));
    (native, browser)
}
pub async fn sample(status: Option<&crate::service::Status>, duration: Duration) -> Report {
    let mut map = processes();
    let (mut native, mut browser) = members(&map, status);
    let counters = |map: &BTreeMap<u32, (u32, u32, u64)>,
                    native: &Vec<u32>,
                    browser: &Vec<u32>|
     -> BTreeMap<u32, u64> {
        native
            .iter()
            .chain(browser)
            .filter_map(|pid| map.get(pid).map(|(_, _, ticks)| (*pid, *ticks)))
            .collect()
    };
    let mut previous = counters(&map, &native, &browser);
    let started = Instant::now();
    let mut native_ticks = 0u64;
    let mut browser_ticks = 0u64;
    while started.elapsed() < duration {
        tokio::time::sleep(
            duration
                .saturating_sub(started.elapsed())
                .min(Duration::from_secs(1)),
        )
        .await;
        map = processes();
        (native, browser) = members(&map, status);
        let current = counters(&map, &native, &browser);
        let delta = |pids: &Vec<u32>| {
            let values = current
                .iter()
                .filter(|(pid, _)| pids.contains(pid))
                .map(|(pid, t)| (*pid, *t))
                .collect();
            tick_delta(&previous, &values)
        };
        native_ticks += delta(&native);
        browser_ticks += delta(&browser);
        previous = current;
    }
    let seconds = started.elapsed().as_secs_f64();
    let clock = std::process::Command::new("getconf")
        .arg("CLK_TCK")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .unwrap_or(100.0);
    let n = sum_memory(&native);
    let b = sum_memory(&browser);
    let total = Memory {
        rss_kib: n.rss_kib + b.rss_kib,
        pss_kib: n.pss_kib.zip(b.pss_kib).map(|(n, b)| n + b),
    };
    Report {
        methodology: "/proc/PID/smaps_rollup KiB; PSS only if all reads succeed, otherwise VmRSS fallback. Process trees refreshed each <=1 second and at final measurement. CPU sums per-PID utime+stime deltas / CLK_TCK / elapsed, 100% = one core. New processes count since birth; processes exiting between samples can be missed, so CPU under churn is a lower bound. Native = Rustune executable family (including installed/deleted binary copies), excluding this measurement command; browser excludes native helper PIDs. Shared Firefox includes all its tabs; not incremental Rustune cost.",
        native_pids: native,
        browser_pids: browser,
        native: n,
        browser: b,
        total,
        cpu_percent_one_core: (native_ticks + browser_ticks) as f64 / clock / seconds * 100.0,
        native_cpu_percent_one_core: native_ticks as f64 / clock / seconds * 100.0,
        browser_cpu_percent_one_core: browser_ticks as f64 / clock / seconds * 100.0,
        cpu_sample_seconds: seconds,
        backend: status.map_or_else(|| "No playback service".into(), |s| s.backend.clone()),
        browser_shared: status.is_some_and(|s| s.browser_shared),
        uptime_seconds: status.map_or(0, |s| s.uptime_secs),
        cache_entries: status.map_or(0, |s| s.cache_entries),
    }
}
