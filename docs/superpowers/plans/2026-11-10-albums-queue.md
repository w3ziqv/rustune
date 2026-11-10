# Albums and queue repair Implementation Plan

> **For agentic workers:** Execute inline with executing-plans; review the whole change with a fresh reviewer after validation. Track steps below.

**Goal:** Complete, transactional album playback and one service-owned queue model shared by TUI, IPC, CLI and MPRIS.
**Architecture:** Queue entries have monotonic persistent identities, source context, a stable playback order, FIFO priority additions and actual listening history. Album preparation runs outside the engine; a revision-checked, idempotent commit follows bounded validation and ordered asynchronous persistence. Audio generation and explicit playback phases distinguish accepted queue edits from working audio.
**Tech Stack:** Linux, Rust 2024 / 1.99.1, Tokio, serde, Ratatui, existing native worker.
**Spec:** docs/reviews/2026-11-10-albums-queue-repair-prompt.md (and project audit).

## Global Constraints

- No private data, credentials, CDM, browser playback, publication, tags or installed-binary replacement.
- Preserve existing untracked reviews and unrelated work; work in the user-requested checkout.
- Bounded records/bytes throughout IPC, preparation, persistence and recovery; restore without autoplay.
- Black/white default TUI; only backend-confirmed audio claims.

## Action semantics

| Intent | Queue | Audio | Preferences |
|---|---|---|---|
| Play album / song context | Replace atomically; whole album, optional selected song | Prepare selected entry, Playing only after confirmation | Explicit shuffle off/on; preserve repeat |
| Play entry | Existing entry identity + revision | Start exactly this occurrence | Preserve |
| Play next | Insert FIFO block ahead of remaining context, including shuffle | Unchanged; no idle autoplay | Preserve |
| Append | Stable block at end | Unchanged; no idle autoplay | Preserve |
| Clear upcoming | Retain current only; stop at its EOF | Position/pause unchanged | Preserve |
| Remove and advance | Checked occurrence; successor or no current | Preserve playing/paused; otherwise Stopped | Preserve |
| Move | Checked occurrence in visible playback order | Current identity unchanged | Preserve |
| Undo replacement/clear | One bounded snapshot; new revision, no old sound/position promise | Stop audio, restore queue without autoplay | Restore snapshot |
| Stop / shutdown | Keep session / close service | Stop at position zero / terminate worker | Preserve |

## Review Focus

Stale clients and late preparation must conflict; retry keys must survive restart. Duplicate catalog IDs retain distinct entry identities. Partial/failed/cancelled albums must leave previous queue/audio untouched. Disk failure and oversized Unicode state must preserve recoverable state. Paused transitions and old worker events must never produce unintended audio.

### Task 1: Session model and queue semantics

Files: src/model.rs, src/queue.rs, tests/core.rs, new tests/queue_session.rs.
- [x] RED tests: FIFO, stable IDs/duplicates, shuffle cycle/history, repeat, movement/removal, clear/undo and middle-of-album context.
- [x] Implement entry/context model, explicit intents, stable order and bounded history; verify queue tests.

### Task 2: Album metadata and complete bounded preparation

Files: src/catalog.rs, src/catalog/apple.rs, new src/catalog/album.rs.
- [x] RED fixtures: 45 and 225 songs, disc/track ordering, missing numbers, unavailable/unsupported records, failed page/limit/cancellation.
- [x] Preserve provider/storefront/album identity, remove relation cap, pin every page, reject incomplete fallback; verify fixtures.

### Task 3: Transactions, persistence, IPC and engine

Files: src/ipc.rs, src/service.rs, new src/service/{persistence,operations,tests}.rs, backend controller/protocol as needed.
- [x] RED tests: migration/duplicate preservation, byte boundary, disk failure/recovery, idempotent revisioned batch, slow prepare/write with Pause/Volume, controlled audio events.
- [x] Implement async preparation/progress/cancel/retry, one ordered persistence owner, checked commit, one undo snapshot, playback phases and generation-safe transitions.
- [x] Verify engine and real isolated IPC service tests.

### Task 4: TUI, CLI and MPRIS

Files: src/ui.rs, src/ui/interaction.rs, src/ui/player.rs, src/ui/keyboard_tests.rs, src/main.rs, src/mpris.rs.
- [x] RED tests: visible album menu at result/details, whole album selected-song context, identity edits, errors, paging retry, transport during slow metadata, MPRIS capabilities.
- [x] Implement discoverable actions and queue order/source; separate metadata transport; expose stop/undo/cancel/retry CLI.
- [x] Verify rendered buffers and full synthetic TUI → service → EOF flow.

### Task 5: Validation and documentation

Files: new scripts/verify-albums-queue.py, docs/queue.md, docs/validation/2026-11-10-albums-queue.md.
- [x] Run fmt, clippy -D warnings, cargo test, release build.
- [x] Run isolated PTY synthetic smoke, measure actual RSS/PSS/CPU and owned subprocesses. Keep metadata/network and real-audio claims separate.
- [x] Fresh review, fix material findings with regression tests, record results and limits. Leave local changes, no publication or install.

## Execution record

Baseline: tracked tree clean, branch codex/ram-eq-search, untracked docs/reviews preserved. cargo test passed, network/visual ignored tests kept separate. User explicitly requests continuous implementation in this checkout; no plan approval pause or worktree relocation.


Implementation record: session entries/context, FIFO priority blocks, stable shuffle, history navigation and projected upcoming order implemented. Complete album collector validated with 45- and 225-track fixtures and source/ordinal/skip/truncation cases. Persistence uses version 2, exact 16 MiB acceptance, backup and recovery lock. Explicit queue/album requests are revision checked and persist the last 64 idempotency keys. TUI/CLI/MPRIS share the model; transport lanes remain independent.

Final review: fresh read-only reviewer (albums_queue_review). Fixed all five material findings with regression tests: stopped removal autoplay, post-rename fsync split state, fallback lookup limit, ordinary Previous, and initial album source pinning. Also cleared stale Failed on Stop/Undo and added cancellation generations, protected pending Stop/EOF and retained Lyrics reload binding. No deferred reviewer findings.

Execution policy: user explicitly requested local changes in this checkout and continuous implementation. No worktree relocation, commits, publication, tags or installation replacement. State is left as a reviewable local diff. Test factories and private D-Bus sessions do not verify Apple authorization/audio.

Verification evidence and measured limits: docs/validation/2026-11-10-albums-queue.md. Validation commands and synthetic PTY run are rerun after the final source change.

Final evidence: fmt/clippy/release and git diff --check passed; cargo test 145 passed, 0 failed, 3 ignored; two online ignored checks passed separately. Final isolated PTY: 6 scenarios, four sizes, RSS 17720 KiB / PSS 8470 KiB, 0.0% sampled CPU over 2.000081s, two owned processes and no audio/browser. No live Apple audio claim.
