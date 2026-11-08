//! Native CDM playback, with no browser process or credentials in local IPC.
pub mod apple;
pub mod cenc;
pub mod controller;
pub mod pcm;
pub mod worker;
pub const NATIVE_UNAVAILABLE: &str = "Native Linux playback needs a saved Music User Token (rustune login) and a compatible CDM at ~/.cache/rustune/drm/libwidevinecdm.so";
