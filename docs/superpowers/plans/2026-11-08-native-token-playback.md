# Verified goal: full native Apple Music playback using the user's token

Do not mark the goal complete based on catalog/UI/token-storage tests. Required end state: a subscribed complete song plays through Rustune on Linux using the supplied Media User Token, without a browser or Music Assistant runtime, without key extraction/DRM bypass, and with all owned processes accounted for. Credentials stay out of source/logs/argv.

## Verified evidence (2026-11-08)

- User token: private nonempty `~/.config/rustune/music-user-token`, mode0600. Format inspection only:246bytes, no whitespace/wrapper/header prefix; not a developer JWT. No token content printed. `developer-token` is empty; user explicitly has only Media User Token.
- Token-only `webPlayback` HTTPS requests to play.music.apple.com (bounded/no redirects/TLS verification, headers marked sensitive) returned a sign-in denial and no song assets. Customer message was classified without printing raw error/account data. Cookie/origin variants did not change the result.
- Native CDM candidate: MIT `Kopuz-org/cdm-host`, pinned source SHA `0e0d76b6b276b8c750bb0cb277b10071de5bc661`. Public CDM ABI, no key extraction. Research clone `/tmp/rustune-cdm-host-research`; supplied prebuilt archive removed so the C++ shim was compiled from reviewed source.
- Existing signed Google CDM4.10.3112.0 was located as a library only (no browser token/cookie access), copied privately to `~/.cache/rustune/drm/libwidevinecdm.so`. No CDM binary is added to source.
- `/tmp/rustune-native-cdm-probe` initialized the module and generated a1743-byte challenge (challenge body withheld). Public Widevine test license request returned HTTP200 and the real CDM accepted the license. No browser process, device-key extraction or user Music Token sent to the test server. This establishes native CDM interoperability, NOT Apple playback.
- `/tmp/rustune-native-auth-probe` contains a private-token HTTPS probe with zeroizing buffers/sensitive headers and status-only output. No credentials embedded.

## Authorization boundary

The user explicitly granted anonymous retrieval of Apple's public application token on November 8, with no publication to GitHub. The granted exception excludes logged-in browser sessions, cookies and private token extraction. The public token is cached only in a private file outside this repository. Music Assistant is research evidence only; its runtime and key-extraction approach are not integrated.

Apple webPlayback now accepts the supplied Media User Token together with that public application token and returns six full-stream assets for real catalog song 1440838314. The earlier synthetic fixture ID was unavailable. The selected CENC AAC manifest has 37 byte-range segments and an end marker. This verifies playback metadata authorization, not audio or a license. The public Apple Music API storefront endpoint still returns 401 and is separate from this native playback path.

## Implementation and verification record

Implementation is now present in src/backend/{apple,cenc,pcm,worker,controller}.rs.
The first genuine native-worker complete-song run reached 359.746757 seconds for
a 359.72787-second manifest, with 15,493 encrypted packets and decoded AAC frames,
31,517,516 nonzero output samples, and exit0. No browser, previews or Music
Assistant runtime were involved. This worker-only development build is separate
from the pending final release TUI/service verification. Findings from an
independent code review were fixed for duration/decoder error gates, PCM tail
conversion, post-logout controls, EOF pause/seek, stale queue Play and CDM timers.

The subsequent release service/TUI run also played the entire 359.746757-second
song and passed 30 checks. The final installed build then passed 34 native control
checks, including private D-Bus properties, near-end pause/resume and EOF, after
final MPRIS/timer-bound fixes. Reports with binary hashes, all-owned-process
RSS/PSS/CPU and installation preservation evidence are linked in docs/validation.md.
No native work was published to GitHub; no credentials or CDM binary are in source.

1. The native CDM accepted a real public test license; actual encrypted Apple AAC packets were then decrypted and decoded. Clear lead alone was never counted as decryption evidence.
2. Safe FFmpeg demux and bounded CENC metadata parsing replaced a standalone BMFF parser. One isolated native worker owns the CDM and ALSA output through private pipes. No content keys or decrypted files are exported.
3. Anonymous public application authorization and the user's private token obtained a full Apple stream and an opaque CDM license, with TLS verification and authenticated redirects disabled.
4. Real native controls/source reporting and full-song output were verified. All-owned-process RSS/PSS/CPU and exact binary distinctions are recorded. The user prohibits GitHub publication, so no push/tag/release was made.

## Public research references

- Apple MusicKit overview: https://developer.apple.com/musickit/
- Native CDM interface host: https://github.com/Kopuz-org/cdm-host
- Widevine integration: https://www.widevine.com/solutions/widevine-drm
- Public test manifest: https://storage.googleapis.com/shaka-demo-assets/angel-one-widevine/dash.mpd

The public metadata/keyboard changes are checkpointed at36481fc and installed; they are separate from this goal and do not prove playback completion.
