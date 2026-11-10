# RAM, equalizer and search Implementation Plan

Historical implementation plan: the user subsequently authorized GitHub publication
on2026-11-10. The local-only constraints below governed the implementation phase.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Reduce measured RAM, add a real ten-band EQ and find combined song/artist queries reliably.

**Architecture:** Separate a dependency-light shared protocol from the UI/service and the owned native audio executable. Apply safe stereo biquads between decoded PCM and ALSA. Improve search through measured upstream behavior, bounded candidate retrieval and deterministic ranking/fallbacks.

**Tech Stack:** Rust2024/stable1.99.1, Ratatui0.30, native CDM, FFmpeg9.0.2, ALSA, verified HTTPS, private pipes.

**Spec:** docs/superpowers/specs/2026-11-08-ram-eq-search.md

## Global Constraints

- Linux only; no unsafe application Rust; native playback without a browser or Music Assistant.
- Credentials outside repository/logs/argv; no DRM bypass or disabled TLS verification.
- No pushes, PRs, tags or releases; changes and installation remain local.
- Preserve user token/queue/preferences; keep black/white default and real bounded covers.
- Use TDD for config, IPC, queue and capability behavior; distinguish synthetic tests from real playback.

## Review Focus

- Missing/incompatible helper must fail clearly rather than launch another application.
- EQ bypass/flat must preserve samples; each channel has independent filter state.
- EQ edits during pause/seek/track changes must persist and reach the correct worker.
- Short titles, punctuation, artist typos and album versions must not be hidden by truncation or unstable pages.
- RAM numbers include every owned process; slim TLS must still validate certificates.

### Task 1: Lightweight audio boundary and RAM

**Files:** Cargo.toml; crates/core/{Cargo.toml,src/lib.rs,src/paths.rs}; audio/{Cargo.toml,src/main.rs,src/worker.rs,src/apple.rs,src/cenc.rs,src/pcm.rs}; src/backend/{controller,worker,mod}.rs; src/config.rs; scripts/build-slim-audio.sh.
**Interfaces:** Shared Control/Event/Paths/private_write; Controller resolves a sibling rustune-audio executable and owns its private pipes/lifecycle. Standard workspace builds use system FFmpeg; optional slim-ffmpeg feature links a minimal private build.
- [x] RED: helper resolution/missing helper and bounded protocol validation tests.
- [x] Move existing tested native code behind shared lightweight interfaces; remove FFmpeg/CDM/ALSA from UI dependency graph.
- [x] GREEN: workspace tests and actual encrypted stream controls. Build pinned minimal FFmpeg outside source tree, keep TLS verified.
- [x] Compare same-song/interval/process RSS/PSS/CPU against preserved baseline binary; retain covers.
- [x] Commit locally after checks.

### Task 2: Real equalizer

**Files:** crates/core/src/equalizer.rs; audio/src/{equalizer,worker}.rs; src/{config,ipc,service,ui}.rs; src/ui/equalizer.rs; tests.
**Interfaces:** EqSettings (enabled, preamp_db, bands_db[10]); Request/Control::Equalizer; worker acknowledgment; live source-rate aware stereo DSP.
- [x] RED: invalid gain/config/IPC, bit-exact bypass, measured 1kHz gain/cut, independent stereo, stable silence, runtime edits and reset on seek.
- [x] Implement bounded RBJ peaking biquads, output clipping protection/headroom, reused PCM buffer and honest state reporting.
- [x] Implement arrow-controlled sliders, bypass, Flat/Bass/Vocal/Treble presets and persistent settings.
- [x] GREEN: full workspace suite; real PTY edits plus native playback with EQ enabled, including pause/seek and track replacement.
- [x] Commit locally after checks.

### Task 3: Search quality

**Files:** src/catalog.rs; src/catalog/search.rs; tests/online scripts.
**Interfaces:** Catalog::browse remains stable; query candidate retrieval/ranking is bounded and pages are cached as a stable ordered result set.
- [x] Reproduce exact A&W/Lana Del Rey/Del Ray queries against real upstream endpoints without user credentials.
- [x] RED: punctuation and mixed title/artist, artist typo, ranking original vs unrelated covers, deduplication and stable pagination; no external cursors/auth leakage.
- [x] Implement general query normalization, bounded fallbacks and scoring without hardcoded artist/song aliases; keep country/explicit/version behavior visible.
- [x] GREEN: whole workspace suite and real online searches find the requested original in the first results.
- [x] Commit locally after checks.

### Final verification and installation

- [x] Fresh independent whole-change code review; fix substantive findings with regressions.
- [x] cargo fmt --all -- --check; cargo clippy --all-targets -- -D warnings; cargo test; cargo build --release.
- [x] Installed keyboard/search/EQ/native audio tests and full-song playback; document measured RAM rather than forecast savings.
- [x] Install both executables atomically, preserve private user files and avoid interrupting active playback. No GitHub publication.
