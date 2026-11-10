# v0.2.0 — Complete albums, repaired queues and Apple Music playlists

Rustune remains experimental and Linux-only. This is a source release; build both
executables with `scripts/build-slim-audio.sh` using Rust 1.99.1 or later. Keep
`rustune-audio` beside `rustune`. Credentials, private caches and the proprietary
Widevine CDM are not distributed.

## Changes

- Play or shuffle an entire album or library playlist across all track pages;
  insert complete FIFO next blocks or append to the queue. Playlist order and
  deliberate duplicate song occurrences are preserved.
- Durable queue identities, revision-checked edits, stable shuffle/history,
  clear-upcoming and one-step undo. Legacy queues migrate with backup/recovery;
  restoring a queue does not start playback.
- Browse your Apple Music playlists and edit permissions. Explicitly create a
  private empty playlist, save the whole queue, or add a selected song/the queue
  to an editable playlist. TUI writes require a review and confirmation; CLI
  write commands are explicit.
- Playlist writes recheck edit permission, preserve duplicates, export at most
  1000 tracks in one POST, and retain a private bounded durable receipt journal.
  Ambiguous outcomes block automatic retries to avoid accidental duplicate writes.
- Independent metadata, queue and transport handling; shared service/IPC/MPRIS
  state and controlled-worker integration tests.
- Make the service integration test wait for asynchronous configuration persistence.
- Fix real artwork colors when NO_COLOR is inherited by Kitty/other terminals.

## Validation and limits

172 Rust tests passed, with 3 opt-in network/visual tests excluded from the default
suite. Format, Clippy with warnings denied and release build passed. Separate
public artwork/lyrics checks and isolated PTY tests passed. Read-only real-account
playlist access was verified, and the user accepted this version locally.

New create/append account writes were tested with synthetic loopback HTTP, not by
automatically changing the user's account. Earlier native-backend validation
played a complete subscribed song; this release does not claim a new complete-song
run. A Music User Token supplies account authorization, not a DRM playback engine.
Native playback still requires the locally installed compatible CDM, subscribed
account and native audio helper. No browser or Music Assistant process is used.

See [playlist controls](playlists.md), [queue behavior](queue.md),
[installation](installation.md) and [validation](validation.md). Close old clients
and stop the old service before replacing both executables. Back up your local
queue/configuration first; never publish tokens or private state.
