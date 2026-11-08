# Catalog search and keyboard navigation

User request: search must work after token entry; navigate the whole TUI using arrows and keyboard. Root cause: Browse is rejected by the audio-unavailable guard; Up/Down fallback changes content selection even when navigation is focused.

1. Regressions first: arrows select/open menu entries, settings are actionable, song context/actions and transport focus are keyboard accessible; public Browse is independent of audio.
2. Add catalog.rs: direct official public Apple iTunes Search/Lookup API, no account credential headers, fixed HTTPS origin/no redirects, bounded response/records/text and 8s timeout. Country is configurable (PL default), cached pages follow profile limits, 20 requests/minute and Retry-After are respected. Only songs/albums/artists are public; private library/playlists explain unavailable account authorization. No preview playback.
3. Route public Browse in bounded IPC client tasks so slow search cannot block queue/status. Page cursors are numeric/validated, never external URLs. Preserve queue/history and playback capability guard.
4. Arrow menu navigation, Enter/Right open, Left/Esc back; Tab focus, arrow-selected settings, song action menu and focused player buttons. Preserve vi shortcuts; seek moves to Alt-arrows. Search opens a typing field; Tab cycles result type. Public source/store link appears in result details.
5. Failing-then-passing network contract/state/keyboard tests, full fmt/Clippy/tests/release, isolated real PTY search→song actions→queue→lyrics/settings and existing token/no-browser regressions. Independent review, install/restart preserving user data, publish without release tag. Native playback and personal account APIs remain explicitly unavailable.

Review focus: no credentials in catalog requests; bounded/malicious response and cursor handling; slow HTTP vs status responsiveness; focus precedence with token/search/action modals; no remote-auth claim after local token save.
