# Feasibility and research — 2026-11-08

This is a historical research record from before native playback was implemented.
Current native CDM playback and EQ are documented in [audio support](audio-support.md)
and backed by [live validation](validation.md). Browser research below describes
removed code and is not an installation guide or a supported runtime.

## Initial target and blocker

The user requires native playback with their own Music User Token, no browser
process and no Music Assistant dependency. Music Assistant is a research
reference only; none of its application code or runtime is integrated.

Its [Apple Music provider manifest](https://github.com/music-assistant/server/blob/main/music_assistant/providers/apple_music/manifest.json)
depends on `pywidevine==1.9.0`. Its [documentation](https://www.music-assistant.io/music-providers/apple-music/)
says playback is not officially supported by Apple and reports AAC up to 256 kb/s,
without lossless/Atmos. This demonstrates a non-browser implementation exists,
but not that an account token alone supplies a playback engine. We do not copy
key-extraction or DRM-bypass code. The project retains its no-DRM-bypass constraint.

Apple's current MusicKit overview describes Apple-platform, Android and web
playback SDKs; no native Linux playback SDK is documented there. Widevine's
[device integration process](https://www.widevine.com/solutions/widevine-drm)
requires agreements and SDK/integration work. No compatible licensed native
playback integration has been established for Rustune. Native full-song playback
is therefore **not implemented or verified**. The native engine is unavailable and reports that limitation. Browser controllers,
extensions and their setup/login paths have now been removed. A manually supplied Music User Token
is kept outside the repository and is not treated as a developer JWT or used to
pretend that native playback works.

In the earlier browser research, the public MusicKit `bitrate` getter is an adaptive/configured 64/256 kb/s target,
not evidence of the current stream encoding. That getter never established a measured source format. Current native source
fields remain unknown.

Apple provides catalog and personal-library APIs, but their JSON responses are
not a native Linux full-song audio API. MusicKit JS is the supported web playback
engine. Apple describes developer JWT signing with ES256, a Media Services key
and an Apple Developer Program account. Music User Tokens identify a subscriber;
they do not replace developer tokens. MusicKit manages user authorization for
web apps. Rustune does not ask for or store Apple ID passwords.

Primary references inspected:

- [MusicKit overview](https://developer.apple.com/musickit/)
- [Developer token requirements](https://developer.apple.com/documentation/applemusicapi/generating-developer-tokens)
- [User authorization](https://developer.apple.com/documentation/applemusicapi/user-authentication-for-musickit)
- [Official MusicKit JS v3 documentation](https://js-cdn.music.apple.com/musickit/v3/docs/index.html)
- [Official MusicKit JS v3 library](https://js-cdn.music.apple.com/musickit/v3/musickit.js)
- [Library songs endpoint](https://developer.apple.com/documentation/applemusicapi/get-all-library-songs)
- [Widevine](https://www.widevine.com/solutions/widevine-drm)
- [Apple lossless platform requirements](https://support.apple.com/118295)
- [Ratatui installation and version compatibility](https://ratatui.rs/installation/)
- [Chromium CDP](https://chromedevtools.github.io/devtools-protocol/)

The documentation's Markdown forms were fetched directly when Apple's normal
pages required JavaScript. The official v3 library was inspected for public
controller shape, preview status and playback-state enums, not DRM internals.
Ratatui 0.30.2 uses Crossterm 0.29 and requires Rust 1.88 or newer.

Two legitimate controller sources are implemented: a minimal local MusicKit
host with the user's own developer JWT, and the official Apple web player with
Apple's own configured MusicKit instance. The latter avoids asking a listener
to become an Apple developer. It does not harvest or copy page tokens. Its
controller exposure was verified in real Chromium; future Apple page changes
can break it. `auto` selects based on whether a valid-shaped private JWT exists.

Research also inspected source and licenses for
[applemusic-tui](https://github.com/k1y0miiii/applemusic-tui) (MIT) and
[Slipmat](https://github.com/SoftARV/Slipmat) (GPL-3.0). No application code was
copied from either project. They demonstrate browser-hosted Linux integrations;
they do not establish that this machine's DRM, subscription or headless playback
works. Rustune remains MIT and does not redistribute MusicKit, Chromium or CDM.

A Linux Chromium installation needs compatible protected-content support and
codecs. EME availability, browser branding, CDM installation and headless mode
can affect playback. Rustune does not bypass DRM, forge entitlements, disable
security protections or download protected streams. Preview playback is blocked
by authorization checks and MusicKit's preview flag; it is never counted as
full-song verification.

No verified MusicKit capability here establishes ALAC Lossless, Hi-Res Lossless,
a configurable sample rate or a measurable source bitrate. Only Auto is enabled;
actual source technical fields stay Unknown. PipeWire output settings cannot
prove the source format. The protected media pipeline does not provide Rustune
a verified PCM path for DSP, EQ or visualization; these remain disabled.

Full-song playback, subscription entitlement, authorized personal data and
long-session stability require interactive Apple authorization on an active
subscription and an actual successful DRM playback run. See `validation.md`
for the exact evidence collected; successful transport/search tests alone do
not meet the release gate.
