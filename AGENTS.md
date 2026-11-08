# Rustune engineering

Linux only. Rust 2024; stable Rust 1.99.1 (minimum 1.99). Prefer small feature modules, borrowed
data and bounded memory. No unsafe application code. Do not extract tokens from
Apple's signed-in web player or bypass DRM. Never log tokens, cookies, CDP expressions or
raw backend errors containing personal data. No credentials in this repository.

The target is native playback using user-supplied credentials, without a browser
process or a Music Assistant dependency. Music Assistant is research only.
Do not claim that a Music User Token supplies a native DRM playback engine.
Do not add browser playback, extensions, native messaging or browser login.
Credentials are user supplied and stay local. Token entry must visibly show bounded
masked feedback; saving a token never implies remote authentication.

User exception (2026-11-08): anonymous HTTP retrieval of Apple's public application
token is authorized, without browser sessions/cookies or private user-token
extraction. Keep it only in private local configuration/cache, never in source,
logs, argv or messages. The supplied Media User Token remains private.
Latest user instruction: do not publish this native playback work or any obtained
credentials to GitHub. Continue locally; no pushes, PRs, tags or releases.

Distinguish implemented behavior from verified live playback. No release tag
until a subscribed account has played a complete song through the real backend.
Unknown audio formats stay unknown; unsupported controls must explain why.

Validation: `cargo fmt --all -- --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`,
`cargo build --release`. Use TDD for queue, config, IPC and capability logic.
Run online lyrics/artwork and PTY smoke separately from unit tests; they must
not pretend to verify Apple authentication or playback. Record actual RSS/PSS and CPU
measurements including every owned browser subprocess.

Default appearance: pure black background and white text/borders. Tokyo Night is
an optional theme. Keep real album artwork bounded and lazy-loaded; report quality
only from reliable backend fields and label reported values clearly.
