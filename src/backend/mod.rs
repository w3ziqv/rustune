//! Native CDM playback, with no browser process or credentials in local IPC.
pub mod controller;
pub mod worker;
pub const NATIVE_UNAVAILABLE: &str = "Native Linux playback needs a saved Music User Token (rustune login), a compatible CDM at ~/.cache/rustune/drm/libwidevinecdm.so, and the executable rustune-audio helper beside rustune";

pub mod driver;

#[cfg(test)]
pub(crate) mod testing;
