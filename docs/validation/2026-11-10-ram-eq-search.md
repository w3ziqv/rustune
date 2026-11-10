# Local RAM, EQ and search revision — 2026-11-10

Installed and verified both executables locally before publication. The user
subsequently authorized the `v0.1.0` source release. Token file identity,
configuration bytes and all39 queue records were preserved. Reopen an old TUI.

Measured real playback with cover visible: peak total PSS141.23→110.46 MiB,
−21.79% (a prior pair measured −20.04%). All three processes are included; no
browsers. Short samples vary and do not guarantee long-duration limits. Detailed
[method and measurements](../benchmarking.md), [installation](2026-11-10-eq-search-installed.json).

The native ten-band stereo EQ supports finite±12dB gains/preamp, live smoothing,
combined-response boost headroom, persistent presets and bypass. `e` opens it;
↑/↓ selects, ←/→ edits, `b` bypasses, `r` resets. It processes real decoded PCM.
A complete359.75-second subscribed song with EQ passed. The final helper is byte
identical to that test; final native controls67/67, keyboard52/52 and functional65/65
passed. Format, Clippy,105 Rust tests and standard/slim release builds passed.

Apple Music is the primary catalog, with verified account region and private
Library/My library search using the local user token. Public catalog requests
omit that token. `A&W lana del ray` finds the original; actual playback was tested.
An iTunes fallback remains explicit. Page cursors pin provider/region, and explicit
previous cursors preserve page size. Playlists and uploaded noncatalog-song playback
remain unsupported; source bit depth/lossless remain unknown/unavailable.

Fresh review found four Important issues: Library PgUp, Enhanced PgUp size,
modified EQ seek keys and cross-provider pagination. Each was reproduced by a
failing regression, fixed and followed by a passing whole suite. No findings
were deferred and the reviewer declined no behavior.

## Decisions made during execution

- Worked in the existing local checkout on `codex/ram-eq-search`, with the prior
  verified native code checkpointed locally. This preserved the established
  workspace; the tradeoff was no separate checkout for concurrent edits.
- Maintained the ledger manually because the skill's helper scripts were absent.
  Tests and reports retain verification evidence; bookkeeping remained manual.
- Replaced iTunes as the primary search after actual queries could not find A&W,
  while Apple's Music catalog handled punctuation and the artist typo. The cost
  is anonymous application-authorization retrieval; explicit iTunes fallback
  remains available, with source pinned across subsequent pages.
- Named the shared package `rustune-core`, since it owns paths/public authorization
  as well as protocol/EQ settings. It can be split again if those concerns grow.

Private credentials, personal library names, signed media URLs, license/challenge
bodies and the proprietary CDM are absent from these reports and source.
