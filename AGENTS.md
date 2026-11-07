# Rustune engineering

Linux only. Rust 2024; minimum Rust 1.88. Prefer small feature modules, borrowed
data and bounded memory. No unsafe application code. Do not extract tokens from
Apple's web player or bypass DRM. Never log tokens, cookies, CDP expressions or
raw backend errors containing personal data. No credentials in this repository.

Use official MusicKit JS with a user-supplied developer token. Browser IPC must
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
