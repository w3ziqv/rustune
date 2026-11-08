# Contributing

Rustune is experimental. Start with `docs/architecture.md`, `docs/feasibility.md`
and `AGENTS.md`. Do not claim live playback, audio quality, or performance that
has not been measured. Never include credentials, browser profiles or personal
library data in issues, logs or fixtures.

Use stable Rust on Linux. Keep changes focused, add behavior tests before fixes,
and run:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Unit tests do not require Apple credentials. D-Bus integration tests require
`dbus-daemon` and `busctl`. Opt-in browser tests require Chromium and network:

```sh
cargo test --test browser -- --ignored --test-threads=1
```

Those tests check transport, the real public MusicKit library and catalog,
not a subscribed account's full-song playback. Describe the browser version,
resource profile, native and browser PSS, and actual subscription/DRM result
when reporting playback behavior. Do not share tokens or protected streams.

Application code is MIT. No GPL application code is incorporated. Keep third
party notices and license requirements when distributing dependencies. Browser,
MusicKit and Widevine are separate components with their own terms.
