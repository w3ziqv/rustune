# Native-only interface and visible secret input

User steering: completely remove Firefox/add-on playback and show masked feedback while entering tokens. Earlier browser-free playback requirement remains active.

1. Add failing regressions for removed browser CLI/setup/configuration, clean UI screens and masked token editing.
2. Remove browser playback controllers, extension/setup assets and authorization routes. Preserve saved queue/preferences and private token files; migrate recognized old browser config to the native-only configuration. Native audio stays explicitly unavailable until a real engine exists.
3. Show bounded dots/count in CLI and TUI; support typing, bracketed paste, Backspace, Ctrl-U, save/cancel and explicit local-save feedback. Never echo the secret.
4. Remove only Rustune-owned installed integration files, without touching Firefox profiles or unrelated extensions. Check any existing service before replacing it so an older process cannot serve removed behavior.
5. Run format, Clippy, tests, release build, isolated PTY login and full online lyrics/artwork smoke. Independent code review, install verified binary, publish source without a release tag and report actual limitations.
