# Skill usage record

Skills are development guidance, not runtime dependencies. User instructions and
measured behavior take precedence over their examples or stale crate versions.

| Skill | Application |
|---|---|
| rust-skills | Borrowed data, explicit errors, bounded allocations, no unsafe application Rust |
| ratatui; basics, layout, styling, widgets | Event-driven terminal lifecycle, responsive layouts, text/colors and visible list slices |
| rust-performance | Visible-row algorithm, bounded cache, shared encoded covers, release/compact and real `/proc` measurements |
| writing-plans / executing-plans | Initial implementation plan, inline autonomous execution and later scope update |
| test-driven-development | Failing-then-passing queue/config/IPC/capability and regression tests |
| systematic-debugging | Browser/controller failures, Firefox URL redaction, catalog argument and shutdown response fixes |
| requesting-code-review | Independent reviews; corrected lifecycle, queue, privacy/cache, metrics, Sixel and disabled-capability findings |
| verification-before-completion | Fresh formatting, Clippy, full suite, builds and explicit separation of live vs synthetic evidence |
| using-git-worktrees | Isolation guidance considered; empty repository used an isolated `codex/` branch without a second checkout |
| security-best-practices | JavaScript bridge guidance and manual Rust boundary review; no unsupported automated Rust security claim |
| skill-installer / OpenAI setup documentation | Discovery/installation of the requested development skills |

Inspected source provenance:

- [leonardomso/rust-skills](https://github.com/leonardomso/rust-skills), master
  `fd2a861ab0406a4ac536a55274d14ea6fd1ca9c9`.
- [zhanghandong/ratatui-skills](https://github.com/zhanghandong/ratatui-skills),
  `b413430707a30cf1241c0598b3e4578526a5d009`.
- [huiali/rust-skills](https://github.com/huiali/rust-skills), performance skill,
  `947bf77509d9b421035037e983da6662d08cbb8e`.
- [obra/superpowers](https://github.com/obra/superpowers), individual workflow
  skills, `22f0b463e7a5e579904dd6211a520321035526f7`.

No unrelated cloud/product/design services are required by the application.
Reference player applications and Music Assistant were research only; their
application code and runtime are not incorporated. Cargo metadata lists dual or
permissive dependency license choices compatible with MIT application source;
third-party dependencies retain their own license obligations. MusicKit,
browsers and CDM components are not redistributed here.


The token/lyrics update used the same Rust/Ratatui/TDD/debugging/review workflow.
amcli and the user's screenshot supplied visual composition; its code was not
copied. LRCLIB's primary API documentation supplied request identification,
matching and throttling requirements. Product Design web-prototype guidance was
inspected for scope and not applied to replatform this native terminal application.
The review's echo/paste/login-help findings and long-line accessibility issue
were fixed with terminal/input/render regressions; visual inspection found and
fixed a source-image upscaling issue. No optional feature is counted as native
full-song playback verification.
