# Existing Firefox session

Firefox is an explicitly selected legacy backend. It uses your already signed-in official
`https://music.apple.com/` tab through a small local WebExtension and Native
Messaging. No cookie, Music User Token or developer-token extraction occurs.
Firefox remains user-owned: Rustune never starts a separate Firefox profile,
closes your browser or kills its processes.

1. Build/install Rustune, then run `rustune firefox-setup`.
2. In Firefox, open `about:debugging#/runtime/this-firefox`.
3. Choose **Load Temporary Add-on…** and select the `manifest.json` printed by
   setup, normally `~/.local/share/rustune/firefox/manifest.json`. In the file
   picker, Ctrl+L accepts an absolute path even though `.local` is hidden.
4. Allow the extension's requested access to `music.apple.com` if prompted.
5. Keep your signed-in Apple Music tab open and run `rustune login --browser`, then
   `rustune`. A previously running Rustune daemon needs restarting after changing
   backend settings (`rustune ctl stop`).

This development add-on is temporary: Firefox removes it on browser restart.
A permanent release requires Mozilla signing; Rustune does not disable add-on
signing. After updating the local extension files, use **Reload** on its
`about:debugging` card. Clicking the extension's toolbar action reconnects and
allows choosing a new music tab if the pinned tab was closed.

Permissions are limited to `nativeMessaging`, `scripting` and the Apple Music
host. There are no cookie, history, password, request-interception or all-sites
permissions. Only fixed MusicKit operations cross the bridge. Arbitrary script
source and native shell commands are not accepted. The native host is registered
at `~/.mozilla/native-messaging-hosts/org.rustune.bridge.json` with exactly one
allowed extension ID, `rustune@rustune.local`. Its private Unix socket is
`$XDG_RUNTIME_DIR/rustune/firefox.sock`.

Native Messaging uses inherited stdin/stdout with native-endian length-prefixed
JSON. The service socket uses big-endian length-prefixed JSON. Bounds are checked
before allocations. The socket is mode 0600 and peers must have the same UID.

Firefox must already be running with its Apple Music tab open for playback.
All player operations then happen in the TUI; initial account login happens on
Apple's site. Developer tokens and manually supplied Music User Tokens are not
needed for this backend. The original MusicKit-host backend remains available
with `backend = "music_kit"` and your own developer JWT.

Resource reports include the **whole shared Firefox process tree**, including
unrelated tabs, separately from Rustune. This is not a measurement of incremental
Apple Music tab memory. The Rust native-host helper is counted under native
memory and excluded from browser totals to avoid double-counting.

Primary sources: [Mozilla Native Messaging](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Native_messaging),
[MAIN execution-world support](https://blog.mozilla.org/addons/2024/07/10/manifest-v3-updates-landed-in-firefox-128/),
[temporary installation](https://extensionworkshop.com/documentation/develop/temporary-installation-in-firefox/).
