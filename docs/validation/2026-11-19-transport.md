# Rustune v0.4 validation on Linux, November 19–20

## Installed native playback

Two adjacent executables from `cargo build --release` ran in a temporary installation using system FFmpeg, separate XDG directories, a private session bus, the local subscribed credentials and the installed CDM. The system's ALSA/PipeWire device provided audio output. The balanced-profile TUI ran in a PTY with artwork and lyrics disabled for the measurement.

The CLI selected The Beatles’ “Her Majesty (Takes 1–3)” (catalog ID 1474834623). Space in the TUI paused audio. MPRIS Seek with offsets +1 s and −1 s produced Seeked at 3.134694 s and 2.113016 s respectively while paused. Each signal was within 50 ms of its requested position. The helper reports the timestamp of the first retained audio frame. Setting the paused position to the full advertised length produced Seeked at exactly 93994550 µs. A subsequent −1 s seek confirmed playback could move back from that endpoint.

MPRIS Stop released the helper. A subsequent `rustune ctl play` started a new helper at **0.000 s**. The song ran continuously to EOF at **94.017596 s** against a manifest duration of **93.994550 s**. The helper processed 4,049 encrypted packets, decoded 4,049 frames, reported 7,937,197 nonzero output samples and exited. The backend reported AAC and full-stream authorization.

| Owned process | Peak RSS (KiB) | Peak PSS (KiB) | CPU (% of one core) |
|---|---:|---:|---:|
| Service | 9,672 | 4,864 | 0.11 |
| TUI | 8,228 | 3,716 | 0.11 |
| Audio helper including the CDM | 141,128 | 118,699 | 1.59 |
| Combined | 159,028 | 127,279 | 1.80 |

There were 373 samples over 93.824 seconds. RSS and PSS came from each process’s `/proc/PID/smaps_rollup`. CPU uses the difference in user and system ticks over that window at 100 ticks/s. The combined peaks use simultaneous samples. The process set included zero owned browsers. The private test bus, measurement driver and system sound server are outside the application totals.

| Installed executable | SHA-256 |
|---|---|
| rustune | `22900af3b07aee4694bf44d64543f7303118d97a014e832586e03b2034b493ca` |
| rustune-audio | `03b6a6577711979b111f1e16d8e3cabd51fa32351b2e3e6e2a23781541531ac5` |

An earlier candidate using the same system FFmpeg started “Simple Man” (1440838314) and passed interactive controls. Its full-song attempt stopped at 18.889 s with the local diagnostic `Unsupported CENC packet metadata`. Full playback of that track remains unverified on v0.4. The [machine-readable measurements](2026-11-19-transport-live.json) belong to the completed Beatles track.

## Offline regression checks

Formatting, Clippy with warnings denied, the default test suite (203 passed, three opt-ins skipped) and the release build passed. The service tests used a controlled audio backend and private D-Bus to exercise short and paused seek signals, invalid positions, stale track occurrences, stale helper events, navigation while paused or stopped, and Stop followed by Play. Property tests counted Volume notifications for a small change and a repeated identical write. The helper tests covered seek coalescing, frame timestamps, version negotiation and child cleanup after timeout or mismatch.

The Unix socket rejected the header-only 65,537-byte request before waiting for a body. Four held catalog requests and eight concurrent subscriptions left room for Pause and Status. Requests queued behind incomplete headers resumed after reader capacity became available. A blocked response writer hit its own deadline.

Doctor ran against an isolated missing helper, a pre-negotiation helper, controlled protocol responders and a matching executable pair. An old daemon alongside a current pair produced restart guidance. The CLI and Settings checks probed the local installation without acquiring a playback license.

## PTY and public network checks

The release TUI rendered Help, survived four terminal resizes and exited normally. The separate keyboard PTY run completed 52 checks including public search, queue and Settings navigation. Individual public LRCLIB and Apple artwork tests passed. These checks exercised their respective services independently of subscribed playback.
