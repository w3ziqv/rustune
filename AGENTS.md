# Rustune engineering

Linux only. Rust 2024; stable Rust 1.99.1 (minimum 1.99). Prefer small feature modules, borrowed
data and bounded memory. No unsafe application code. Do not extract tokens from
Apple's web player or bypass DRM. Never log tokens, cookies, CDP expressions or
raw backend errors containing personal data. No credentials in this repository.

The target is native playback using user-supplied credentials, without a browser
process or a Music Assistant dependency. Music Assistant is research only.
Do not claim that a Music User Token supplies a native DRM playback engine.
The existing Firefox/MusicKit backends are legacy, explicitly selected options;
never extract their tokens. The optional local MusicKit host uses a user-supplied
developer token. Browser IPC must
use private inherited pipes, never a public debugging port. Own browser processes
and clean them up. Never disable the sandbox, web security or certificate checks.

Distinguish implemented behavior from verified live playback. No release tag
until a subscribed account has played a complete song through the real backend.
Unknown audio formats stay unknown; unsupported controls must explain why.

Validation: `cargo fmt --all -- --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`,
`cargo build --release`. Use TDD for queue, config, IPC and capability logic.
Run browser smoke tests separately from unit tests; they must not pretend to
verify Apple authentication or playback. Record actual RSS/PSS and CPU
measurements including every owned browser subprocess.

Default appearance: pure black background and white text/borders. Tokyo Night is
an optional theme. Keep real album artwork bounded and lazy-loaded; report quality
only from reliable backend fields and label reported values clearly.
