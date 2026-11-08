//! Safe Rust wrapper over the C++ Widevine-CDM host shim.
//!
//! Drives the system `libwidevinecdm.so` through its official interface to
//! generate license challenges and decrypt CENC — without extracting any keys
//! (the CDM keeps its device key sealed; we only use the public ABI).
//!
//! Two ways to get a [`CdmHost`]:
//! * [`CdmHost::open`] — you supply the path to a `libwidevinecdm.so`.
//! Rustune intentionally omits automatic installation and downloading.

use std::ffi::CString;
use std::os::raw::{c_char, c_int};
use std::path::Path;
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};
static OPENED: AtomicBool = AtomicBool::new(false);

extern "C" {
    fn ch_open(so_path: *const c_char) -> c_int;
    fn ch_challenge(init_data: *const u8, len: u32, out: *mut *mut u8, out_len: *mut u32) -> c_int;
    fn ch_update(license: *const u8, len: u32) -> c_int;
    fn ch_decrypt(
        data: *const u8,
        data_size: u32,
        key_id: *const u8,
        key_id_size: u32,
        iv: *const u8,
        iv_size: u32,
        subs: *const u32,
        num_subs: u32,
        out: *mut *mut u8,
        out_len: *mut u32,
    ) -> c_int;
    fn ch_free(p: *mut u8);
    fn ch_poll() -> c_int;
}

unsafe fn take(out: *mut u8, len: u32) -> Vec<u8> {
    let v = std::slice::from_raw_parts(out, len as usize).to_vec();
    ch_free(out);
    v
}

/// A handle to the process Widevine CDM.
///
/// The underlying native CDM is a single global instance, so every `CdmHost`
/// drives the same CDM; opening again re-initializes it. Treat it as one
/// process-wide resource rather than many independent objects.
pub struct CdmHost {
    _priv: PhantomData<Rc<()>>,
}

impl CdmHost {
    /// Load + initialize the CDM at `so_path` (e.g. Brave's `libwidevinecdm.so`).
    /// The path is supplied by the caller — nothing is downloaded or copied.
    pub fn open(so_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !OPENED.swap(true, Ordering::SeqCst),
            "only one CDM may be opened per process"
        );
        let path = so_path.as_ref();
        let s = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF-8 CDM path: {}", path.display()))?;
        let c = CString::new(s)?;
        match unsafe { ch_open(c.as_ptr()) } {
            0 => Ok(CdmHost { _priv: PhantomData }),
            n => anyhow::bail!("ch_open({}) failed (code {n})", path.display()),
        }
    }
    /// Pump due host timers on the same thread that owns this CDM.
    pub fn poll(&self) -> anyhow::Result<()> {
        match unsafe { ch_poll() } {
            0 => Ok(()),
            _ => anyhow::bail!("CDM scheduled work requires renewed authorization"),
        }
    }

    /// Generate a license challenge from a CENC `pssh` box. This is a real
    /// ChromeCDM challenge — the device Widevine services accept.
    pub fn challenge(&self, pssh_box: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut out = std::ptr::null_mut();
        let mut len = 0u32;
        match unsafe { ch_challenge(pssh_box.as_ptr(), pssh_box.len() as u32, &mut out, &mut len) }
        {
            0 => Ok(unsafe { take(out, len) }),
            n => anyhow::bail!("ch_challenge failed (code {n})"),
        }
    }

    /// Feed the license response back into the CDM (so it learns the content keys).
    pub fn update(&self, license: &[u8]) -> anyhow::Result<()> {
        match unsafe { ch_update(license.as_ptr(), license.len() as u32) } {
            0 => Ok(()),
            n => anyhow::bail!("ch_update failed (code {n})"),
        }
    }

    /// Decrypt one CENC buffer with the loaded keys. `subsamples` is (clear,
    /// cipher) byte counts per subsample; empty = whole buffer encrypted.
    pub fn decrypt(
        &self,
        data: &[u8],
        key_id: &[u8],
        iv: &[u8],
        subsamples: &[(u32, u32)],
    ) -> anyhow::Result<Vec<u8>> {
        let mut subs: Vec<u32> = Vec::with_capacity(subsamples.len() * 2);
        for &(c, e) in subsamples {
            subs.push(c);
            subs.push(e);
        }
        let mut out = std::ptr::null_mut();
        let mut len = 0u32;
        match unsafe {
            ch_decrypt(
                data.as_ptr(),
                data.len() as u32,
                key_id.as_ptr(),
                key_id.len() as u32,
                iv.as_ptr(),
                iv.len() as u32,
                subs.as_ptr(),
                subsamples.len() as u32,
                &mut out,
                &mut len,
            )
        } {
            0 => Ok(unsafe { take(out, len) }),
            n => anyhow::bail!("ch_decrypt failed (code {n})"),
        }
    }
}
