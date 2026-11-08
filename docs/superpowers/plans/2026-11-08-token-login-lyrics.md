# Token login, lyrics and player layout implementation plan

> **For agentic workers:** Use executing-plans inline, TDD per task, one independent whole-branch review before publication.

**Goal:** Add hidden token entry to `login`, free real lyrics, and the user's selected amcli-like player layout.

**Architecture:** Credentials are stored locally in a private file, never command arguments, IPC payloads or logs. A separate bounded LRCLIB worker sends only track metadata and returns synchronized/plain lyrics. Ratatui presents large artwork left, metadata and centered lyrics right, progress and controls below. Existing browser controllers remain opt-in; no native playback success is fabricated.

**Tech Stack:** Existing Rust 2024 / Rust 1.99.1, Ratatui 0.30, Crossterm, Tokio, Reqwest; small zeroize/httpdate dependencies if required.

**Spec:** User messages requesting token login, free external lyrics and the attached amcli screenshot; existing AGENTS.md constraints.

## Global constraints

- Linux only; black background, white text/borders; optional Tokyo Night.
- No DRM bypass, browser-token extraction, Music Assistant dependency or automatic browser fallback.
- No tokens in logs, history, IPC or the repository. Hidden prompt must restore terminal state on cancel/error.
- Keep the user's queue and active listening session intact; no automatic sound controls.
- LRCLIB requests must be bounded, identified, sequential and honor Retry-After; no Apple credentials go there.
- Verify token storage separately from server acceptance and actual full-song playback.

## Review focus

- Secret input pasted, cancelled, oversized or invalid: no echo, no accidental history/commands, no file replacement on failure.
- Track changes during network lookup: never show a previous song's lyrics/artwork.
- Invalid LRC/Unicode/timestamps/offsets or very large responses: bounded parsing and safe rendering.
- LRCLIB missing data, instrumental tracks, 429/503 and network failure: useful distinct states and no request loop.
- Narrow/resized terminals: unclipped geometry, usable navigation/controls and no panic.

### Task 1 — Local token login

Files: src/auth.rs, src/main.rs, src/ui.rs, tests/auth.rs.
Interfaces: SecretToken / TokenInput, store_user_token(&Paths, &str), prompt_login(&Paths); TUI login modal and local submission worker.
- [x] Write and observe failing validation/private-write tests and a real PTY login/no-echo/cancel test.
- [x] Implement bounded opaque token validation, private atomic storage, hidden CLI prompt and `login --browser` for explicit legacy authorization.
- [x] Implement TUI `:login` / L entry with a hidden buffer, paste handling and cancellation, without token IPC/history.
- [x] Run full suite and record exact behavior, including unavailable native playback.

### Task 2 — Free synchronized lyrics

Files: src/lyrics.rs, src/config.rs, tests/lyrics.rs.
Interfaces: LyricsQuery::from_track, Lyrics::{parse,active_line}, Provider::lookup, watch-channel lyrics worker.
- [x] Write and observe failing LRC offset/multiple-time/unknown/plain/instrumental tests and HTTP contract/rate-limit tests.
- [x] Implement LRCLIB exact lookup plus tightly matched search fallback, bounded bodies/cache, User-Agent, timeouts and Retry-After.
- [x] Add identified opt-in real network smoke; report only source/counts, not song text.
- [x] Run full suite; distinguish synthetic sync checks from live audio.

### Task 3 — Selected player layout and integration

Files: src/ui/player.rs, src/ui.rs, src/artwork.rs, src/keys.rs, tests/ui.rs, docs.
- [x] Write and observe failing geometry/lyrics-highlight/no-secret-render tests at 142×39 and compact sizes.
- [x] Make Now Playing the primary view; large square artwork left, metadata and centered scrolling lyrics right, progress and clickable bordered controls below.
- [x] Make artwork encoding size-aware and bounded; lazily fetch lyrics when visible, reject stale results, support refresh and disabling lyrics.
- [x] Run real PTY smoke without audio controls, network lyrics/artwork smoke, fmt/Clippy/full tests/release build and independent review.
- [x] Update docs, install binary atomically, audit secrets and publish source under existing authorization; no native-player release tag.

## Decisions and evidence

- Ruling: amcli is an MIT macOS AppleScript controller, not a Linux token playback implementation. Use its visual arrangement and LRCLIB approach as references; write Rustune's modules independently.
- Ruling: The attached screenshot selects layout; existing black/white theme preference remains. Real covers are fetched, never generated from the screenshot. Web-prototype/image-generation workflows do not apply to this native terminal app.
- Ruling: Current `codex/` branch is an isolated clean feature branch in the user's existing workspace; do not relocate their active checkout/session.
- Native token-only playback remains blocked on establishing a compatible DRM integration under the existing no-bypass constraint. This plan adds real login/storage and lyrics, not a false playback claim.

## Execution ledger

- Auth storage, CLI/TUI prompts and synthetic PTY cancellation/hidden-entry tests: implemented; RED→GREEN.
- LRCLIB parser and HTTP contracts: RED→GREEN. Separate real lookup returned 37 synced lines for the user-referenced song without printing its text.
- Primary 142×39 player layout, dynamic cover size, centered current verse and stale-result checks: RED→GREEN.
- Review identified prompt-before-no-echo race, crossterm's unbounded paste accumulator and obsolete browser-login hints. Fix with bounded input parsing owned by Rustune, raw mode before prompt, accurate hints, then full suite.
- Ruling: Treat inaccessible overlong lyric lines as Important for a lyrics reader; wrap them and keep the active line centered. This is within the requested working-lyrics scope.

- Final: fixed echo ordering and unbounded paste input through a private bounded raw reader; one-million-byte paste and Ctrl-C tests RED→GREEN. Terminal no-echo/storage/cancel tests and full 53-test suite pass.
- Final: fixed login hints and overlong lyric accessibility; wrapped-row/end-of-line test RED→GREEN. No remaining deferred review findings.
- Final: visual inspection found a small image not filling the large artwork allocation; encoded cell-size regression RED→GREEN, actual buffer inspected again.
- Final validation: fmt, Clippy -D warnings, 53 tests, release build, cargo audit, isolated real PTY and separate real LRCLIB/artwork smoke passed. Native playback remains unimplemented.

- Selected queue lyrics preview: RED→GREEN regression, no backend or playback command required. This makes free lyrics independently usable while native audio remains unavailable. Full suite now 54 tests.
- Source update 9975171 published to main and GitHub CI passed for stable/1.99.1. Updated binary installed atomically; the user session was not restarted.
