# Library playlists — 10 November 2026

Implemented private library browsing and complete ordered playlist contexts, with source positions for duplicates, plus explicit private creation, queue export and append to editable playlists. Playback uses the existing native driver. No browser, login extraction, native messaging, DRM changes, release tag, publication or installation replacement was added. Previous local albums/queue/artwork work was retained.

Validation of the final code:

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo test`: **172 passed, 0 failed, 3 ignored** across the workspace.
- `cargo build --release`: passed.
- Separate online artwork and LRCLIB tests: each passed (anonymous public endpoints).
- Real TUI/PTY `verify-playlists.py`: list/edit permissions visible, name review and Esc produce no write, one final confirmation emits exactly one synthetic whole-queue request. Temporary XDG paths and concurrent synthetic IPC; zero account POSTs.
- Real isolated service/PTY `verify-albums-queue.py`: all six checks passed, including legacy migration, stable duplicate identities, stale-edit rejection, retry deduplication, clear/undo, four terminal sizes, backup/restart and no autoplay.
- Real terminal-stream artwork regression `verify-artwork-display.py`: inherited NO_COLOR=1, 194 half-block cells, 266 cells preserving image colors, clean quit.

Focused fixtures exercise 225 private tracks across nine pages, private-only routing and account storefront pinning, catalog relationship mapping, duplicate source positions, fresh metadata, read-only validation, snapshot/revision/1000-track limits, exact one-POST payloads, HTTP 403/429 vs ambiguous 408/5xx/lost responses, malformed/symlink journals, cross-writer exclusion, restart replay, and blocked uncertain keys. The create payload explicitly sets isPublic=false. A gated HTTP response test holds the account POST pending while Pause, Volume and Status each complete within 300 ms, preserving the queue and audio worker identity.

The complete synthetic flow drives actual keyboard intents through Unix IPC, private HTTP fixtures, the service disk transaction and a controlled audio driver: it opens the first page, prepares all 45 playlist entries, exports all 45 in one private-create POST with duplicates, then advances through all 45 EOF events. This verifies integration, not Apple DRM/audio output.

A fresh independent review found five material issues, all corrected: playlist PgUp, stale playlist context after a new search, account replacement between pages, lock lifetime across shutdown during blocking receipt I/O, and delayed command acknowledgement regressing the subscription status. Regression tests cover navigation, context/account changes and delayed acknowledgements. The lock now moves into the blocking completion writer itself. The reviewer confirmed all findings closed and found no new material issue.

The final release was launched directly from target/release in the test Kitty window; the service executable inode matches the new build. The existing queue was restored with playback stopped, without a browser or audio helper. A separate real-account **read-only** check through the new IPC confirmed playlist listing and playlist-track browsing with source positions. No real-account POST, new live playback or complete-song verification was performed by this task. Account writes were tested against loopback fixtures only; actual writes require the explicit user's TUI confirmation or CLI command.

Actual process measurements (Linux /proc, final release):

| Scenario | RSS KiB | PSS KiB | CPU % of one core | Sample | Owned browsers |
|---|---:|---:|---:|---:|---:|
| Isolated idle TUI + service, balanced | 18664 | 8852 | 0.0 | 5.0001 s | 0 |
| Queue PTY smoke, TUI + service | 18740 | 5079 | 0.4999 | 2.0004 s | 0 |

The queue smoke enumerated two owned processes, zero exits, zero browser processes and zero audio helpers. Browser RSS/PSS were zero. Idle first frame was 10.35 ms, four resize checks and clean quit passed; cache was warm. Zero sampled CPU means no observed tick increase during that interval. These figures do not measure active playback or waiting on a remote write. The developer's actual session was excluded from these isolated measurements.
