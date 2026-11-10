# Measurements — 2026-11-10

The native audio helper and minimal private FFmpeg reduce the measured playback
memory on this Linux machine. These are single-run observations, not limits.

Same public song (Simple Man), balanced profile, 142×39 terminal, real cover,
EQ off, 3 s warm-up then 30 s sampling. Isolated XDG paths and private D-Bus;
all three owned processes (TUI, service, audio/CDM helper) are summed. Browsers:0.

| Measurement | Before | After |
|---|---:|---:|
| Peak PSS | 141.23 MiB | 110.46 MiB |
| Peak summed RSS | 234.45 MiB | 132.26 MiB |
| Mean PSS | 141.18 MiB | 110.41 MiB |
| CPU, one core | 2.78% | 2.00% |

Peak PSS fell by 30.78 MiB (21.79%). A prior paired run measured a20.04%
reduction; CDM/allocator working sets vary between runs. Summed RSS counts shared mappings multiple
times; PSS is the better comparison for total physical memory. CPU differences
in this short sample are not established speedups. The CDM remains included.
The paired samples verify genuine encrypted playback, not a complete song.

[Before](validation/2026-11-10-ram-before.json) and
[after](validation/2026-11-10-ram-after.json) record exact executable hashes,
individual process accounting and samples. Repeat with the opt-in
`scripts/measure-playback.py --live --seconds 30 BINARY REPORT` using a test-only
Python environment with pyte. Keep the helper beside the new binary. Do not
rebuild/replace either binary during a run.

Separately, a complete 359.75-second song with Vocal EQ enabled passed, including
real cover/lyrics, packet/decode/output evidence and process cleanup. It peaked
at PSS111.89 MiB and summed RSS136.05 MiB; the EQ-off paired comparison above is
the optimization measurement. This does not establish hour-long memory stability.

The UI/service no longer link FFmpeg/ALSA/CDM. The audio helper uses checksum-pinned
FFmpeg9.0.2 with only AAC/MOV, conversion and HTTPS support. Standard workspace
builds still support system FFmpeg; run `scripts/build-slim-audio.sh` last for
the measured slim variant. Private build artifacts are outside the repository.

## Historical measurements — 2026-11-08

Arch Linux x86_64, stable Rust 1.99.1. These are single-machine samples, not
guaranteed limits. The older idle samples below predate the native engine and are distinct from
the active playback measurements above.

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


## Token/lyrics/player update

Repeated isolated balanced idle measurement after the hidden token prompt,
bounded input reader and player/lyrics update: native TUI + service RSS 15,416
KiB, PSS 7,260 KiB, zero additional CPU ticks over 5.00 seconds, first frame
10.37 ms. Browser processes: zero. This is still an idle/no-audio measurement,
not active native Apple Music playback. The real PTY Help/resize/quit checks passed.

After the functional scrolling/queue fixes, the installed release binary repeated
the isolated idle scenario: RSS 17,560 KiB, PSS 8,163 KiB, zero additional CPU
ticks over 5.00 seconds, first frame 10.35 ms. All owned browser subprocesses:
zero (RSS/PSS zero). This remains an idle/no-audio sample with a warm filesystem
cache. The [machine-readable sample](validation/2026-11-08-idle.json) records the
exact values; the separate online PTY scenario exercises real lyrics/artwork,
but does not measure active streaming or native playback.

## Native-only revision

After browser removal and masked token feedback, the installed release repeated
the isolated no-audio scenario: RSS 17,272 KiB, PSS 5,878 KiB, zero additional
CPU ticks over 5.00 s, first frame 10.35 ms, zero owned browser processes.
See the [exact sample](validation/2026-11-08-native-only-idle.json). Earlier
browser figures above are historical and do not describe an available feature.
