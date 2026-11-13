//! `run.elevated`, collected only with `--collect elevation`: whether this
//! run had administrator (Windows) or root (Unix) rights. A property of the
//! invocation, not the machine — the same machine is elevated one run and
//! not the next — which is why it sits in `run`, beside the run ID.
//!
//! - **Windows** — `GetTokenInformation(TokenElevation)` on this process's
//!   own token, opened for `TOKEN_QUERY` only. "Elevated" is UAC's meaning:
//!   the token carries the administrator rights, which for an administrator
//!   under UAC means "Run as administrator" was used.
//! - **Unix** — `geteuid() == 0`.
//!
//! A query that fails leaves the field absent, never `false`.

/// Only [`crate::optin::gather`] calls this, and only when `elevation` was
/// asked for.
#[cfg(windows)]
pub(crate) fn read() -> Option<bool> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle that needs no
    // closing; `token` is a valid out-pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        log::debug!(target: "loadbearer::optin", "OpenProcessToken failed");
        return None;
    }
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0u32;
    // SAFETY: `token` was opened with TOKEN_QUERY above; the buffer is a
    // TOKEN_ELEVATION and its exact size is passed.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut TOKEN_ELEVATION as *mut core::ffi::c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    // SAFETY: `token` is a real handle this function opened and owns.
    unsafe { CloseHandle(token) };
    if ok == 0 {
        log::debug!(target: "loadbearer::optin", "GetTokenInformation(TokenElevation) failed");
        return None;
    }
    Some(elevation.TokenIsElevated != 0)
}

#[cfg(unix)]
pub(crate) fn read() -> Option<bool> {
    // SAFETY: geteuid has no preconditions and cannot fail.
    Some(unsafe { libc::geteuid() } == 0)
}

#[cfg(not(any(windows, unix)))]
pub(crate) fn read() -> Option<bool> {
    None
}
