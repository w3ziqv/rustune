# Contributing

Linux, Rust 2024, stable 1.99.1. Read AGENTS.md. Keep memory bounded, private data
outside the repository, and application code free of unsafe Rust.

Run `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`, and `cargo build --release`. Queue/config/IPC/capability changes need
failing-then-passing regressions. Tests require Python 3, D-Bus daemon and busctl.

Optional online smoke uses `scripts/verify-functional.py` with pyte in a test-only
virtualenv, isolated XDG paths and private D-Bus. It accesses public LRCLIB/Apple
artwork, uses an artificial token and never verifies Apple authentication/audio.
`dbus-run-session -- python3 scripts/verify-login.py target/release/rustune` checks
real masked CLI/TUI input, private storage, cancellation and service cleanup.

The native playback engine is unavailable. Do not claim complete-song playback,
measured source formats or account acceptance from a local token file or mocked
protocol. Do not add browser controllers, credential extraction or DRM bypasses.
