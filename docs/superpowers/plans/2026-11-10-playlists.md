# Library playlists Implementation Plan

> **For agentic workers:** Execute inline with executing-plans; finish with an independent code review. Preserve the existing uncommitted albums/queue/artwork work.

**Goal:** Browse and play whole library playlists, create empty playlists or save the queue as a new playlist, and append a selected song/the queue to an editable library playlist.
**Architecture:** Reuse the service-owned context/queue transaction for playlist playback. Library metadata keeps account/storefront pinning and source positions, including duplicates. Account writes run outside the engine, validate a queue snapshot and fresh canEdit, use a bounded private durable request journal, and never blindly retry an uncertain POST.
**Tech Stack:** Linux, Rust 2024 / 1.99.1, Tokio, serde, Ratatui; existing native driver.
**Spec:** User request in this chat, AGENTS.md, docs/queue.md; official Apple Music API library playlist GET/POST endpoints.

## Constraints and review focus

- Keep credentials local; no headers, tokens, raw account responses or playlist names/IDs in logs or repository fixtures.
- No browser, account login, DRM changes, publication or installation replacement.
- No automated writes to the real account. Write fixtures use loopback HTTP and artificial credentials. Actual writes require explicit TUI/CLI commands.
- Preserve queue/audio for browse and account writes; complete context preparation commits once with a revision check.
- Bound reads to 10000 entries/12 MiB; writes to one POST of at most 1000 tracks/128 KiB; fail before sending instead of silently exporting a prefix.
- Do not promise server-side idempotency. Durable Sending/Uncertain keys cannot be retried automatically after timeout/crash; completed keys replay locally. Journal is private and bounded.
- Review: duplicate song positions, missing catalog IDs, account changes, stale clients, read-only playlists, ambiguous network/disk outcomes, concurrent transport, cancelled preparation, and sensitive error sanitization.

## Action semantics

| Action | Queue/audio | Account |
|---|---|---|
| Open / browse playlists | Unchanged | GET only, all pages available |
| Play / shuffle playlist | Complete ordered context; explicit selected source position; queue replacement is undoable | GET only |
| Playlist next / append | One complete FIFO/end block; no autoplay | GET only |
| New empty playlist | Unchanged | One explicit POST after name confirmation |
| Save queue as playlist | Revision-checked whole visible queue snapshot, duplicates retained; audio unchanged | One POST including tracks |
| Add song / queue to playlist | Selected catalog song or checked snapshot; audio unchanged | Fresh canEdit validation, one append POST |
| Retry write | Unchanged | Only definitely rejected/pre-send failures; uncertain outcomes require review |

## Tasks

- [x] 1. RED fixtures for private list/track paging (225+), canEdit, duplicate source positions and catalog mapping; implement bounded playlist metadata/context models and collector. Files: src/model.rs, src/queue.rs, src/catalog/{apple,album,playlist}.rs, src/catalog.rs.
- [x] 2. RED tests for playlist playback from the second duplicate, page failure/cancel and stale revision; generalize existing asynchronous context preparation and add explicit playlist IPC/CLI intents. Files: src/ipc.rs, src/service{.rs,/operations.rs,/session_tests.rs}, src/main.rs.
- [x] 3. RED HTTP/journal tests for create/append payloads, read-only denial, 401/403/429, timeout/5xx ambiguity, disk failure, retries/restart and snapshot limits; implement private write actor and status. Files: new src/catalog/apple/playlist_write.rs, src/service/{playlist_write,playlist_journal}.rs, src/ipc.rs.
- [x] 4. RED keyboard/render tests for playlist navigation/actions, new/save name dialogs, destination picker, confirmation and safe error/retry UI; integrate independent lanes. Files: new src/ui/playlists.rs, src/ui{.rs,/interaction.rs,/runtime.rs,/keyboard_tests.rs}.
- [x] 5. Full synthetic UI→IPC→metadata→EOF/write flow, offline private-D-Bus PTY, fmt/clippy/test/release, separate public artwork checks and actual process measurements. Add docs/playlists.md and validation report. Fresh review; fix material findings with regressions. No real-account POST during validation.

## Execution record

Baseline: all previous local changes retained; last full suite 145 passed, 3 ignored. Real-account research verified read access only; no account writes performed. User authorized adding the features and UI actions. Continue without a plan approval pause in the user-requested checkout.


Completion record: final workspace validation passed fmt/clippy, 172 tests (0 failures, 3 ignored) and release build. Separate public artwork/lyrics checks, playlist PTY, albums/queue PTY and process measurements passed; see docs/validation/2026-11-10-playlists.md. Whole-context integration traversed 45 controlled EOF events; fixture paging covered 225 tracks. All five fresh-review findings were fixed and confirmed closed.

Ruling: mutable playlist metadata bypasses the page cache, so refresh/preparation cannot silently reuse previously edited playlist pages; the existing request budget remains in force. A credential stamp spans the complete private context preparation. Account writes retain their own stamp checks and durable journal.

Ruling: the optional skill workspace helper was absent locally; this plan's execution/completion record is kept here. Work stayed in the user's requested checkout, retaining all pre-existing uncommitted files; no commits or publication were performed.

Ruling: final launch uses the built executable directly in a new Kitty test window, preserving the local queue and leaving playback stopped. No installed executable was replaced. Real-account validation is GET-only; create/append remain fixture-tested until the user explicitly confirms a write.
