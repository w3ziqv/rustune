# Measurements — 2026-11-08

Arch Linux x86_64, stable Rust 1.99.1. These are single-machine samples, not
guaranteed limits. Crucially, the current native engine is unavailable: idle
browser-free measurements are **not active Apple Music playback measurements**.

## Isolated idle TUI + service

Run `python3 scripts/measure-idle.py target/release/rustune`. The script creates
temporary XDG directories and its own D-Bus session. It samples only its TUI and
service PIDs using `/proc/PID/smaps_rollup` and `utime+stime`, leaving any listening
session alone. It exercises Help, four terminal sizes and quit. Browser processes
owned in this scenario: zero. Audio tested: no. Profile: balanced, 100 × 30 cells.

| Build | Native RSS KiB | Native PSS KiB | Idle CPU, one core | First TUI frame |
|---|---:|---:|---:|---:|
| release, thin LTO | 15,060 | 6,913 | 0.0% / 5.00 s | 10.42 ms |
| compact, size optimization | 14,524 | 6,270 | 0.0% / 5.00 s | 10.31 ms |

CPU precision is limited by kernel ticks; 0.0% means no additional sampled ticks,
not that every future idle interval is free. First frame uses a warm filesystem
cache and a 10 ms PTY observation interval. True cold-cache startup is unmeasured.

## Build and visible-row comparison

`rustune benchmark --seconds 1` measures 11 `--version` subprocess launches and
1,000 synthetic formatting passes, either all 10,000 rows or the visible 20 rows.
It does not measure browser startup, streaming latency or complete navigation.

| Build | Binary bytes | First CLI launch | Mean next 10 launches | All rows | Visible rows |
|---|---:|---:|---:|---:|---:|
| release | 8,994,064 | 1.056 ms | 0.940 ms | 845.608 ms | 1.429 ms |
| compact | 6,526,464 | 1.100 ms | 1.031 ms | 779.650 ms | 1.320 ms |

The benefit of constructing visible rows is clear in this synthetic workload;
the small release/compact speed difference is not statistically established.
Release remains the default. Image fetching/decode/encode runs off the render
path. Real-cover network smoke passed separately; cache growth under extended
listening is not measured.

## Legacy browser accounting

An earlier shared-Firefox sample recorded native service/host RSS 12,188 KiB,
PSS 3,542 KiB, shared browser-tree RSS 4,699,808 KiB and PSS 2,912,831 KiB.
Combined RSS was 4,711,996 KiB; PSS 2,916,373 KiB. CPU was 399.324% of one core
over 5 seconds. The browser tree included all existing Firefox tabs and process
churn, so these values are **not incremental Rustune cost** and are not comparable
to an otherwise empty dedicated browser. There was no updated-build active-song
measurement or one-hour stability run. Native/browser CPU splitting was added
after that sample and is not retroactively invented.

`rustune stats --seconds 5` reports actual RSS/PSS and CPU separately for native,
browser and combined processes. Native accounting includes the Rustune executable
family and can include another running Rustune instance. All owned Chromium
subprocesses are included; shared Firefox includes the user's full process tree.
Membership refreshes each second and at completion. Short-lived processes exiting
between samples can be missed, making CPU under churn a lower bound. Missing PSS
reads are reported as unknown, never replaced with a guessed PSS.
