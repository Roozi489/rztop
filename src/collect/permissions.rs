// Modified for rztop on 2026-11-03: report observed permission failures without elevation prompts.
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};

static ACCESS_DENIED: AtomicBool = AtomicBool::new(false);
static ELEVATED: OnceLock<Option<bool>> = OnceLock::new();

/// Remember actual access failures, including one-time hardware backend discovery.
pub fn note_error(error: &windows::core::Error) {
    if error.code() == windows::Win32::Foundation::E_ACCESSDENIED {
        ACCESS_DENIED.store(true, Ordering::Relaxed);
    }
}

pub fn limited_without_admin() -> bool {
    needs_admin_notice(is_elevated(), ACCESS_DENIED.load(Ordering::Relaxed))
}

/// Whether this process runs elevated; `None` if the token cannot be read.
/// Cached, since elevation cannot change for a running process.
pub fn is_elevated() -> Option<bool> {
    *ELEVATED.get_or_init(elevated)
}

fn needs_admin_notice(elevated: Option<bool>, denied: bool) -> bool {
    elevated == Some(false) && denied
}

fn elevated() -> Option<bool> {
    use windows::Win32::{
        Foundation::HANDLE,
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    let mut raw = HANDLE::default();
    // SAFETY: current process pseudo-handle is valid; raw is writable. The real
    // token handle is owned and closed on every path after a successful open.
    unsafe {
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw).ok()?;
    }
    let token = super::win::OwnedHandle::new(raw)?;
    let mut info = TOKEN_ELEVATION::default();
    let mut size = 0;
    // SAFETY: output points to a correctly sized/aligned TOKEN_ELEVATION;
    // token remains open throughout the call and the result is checked.
    unsafe {
        GetTokenInformation(
            token.get(),
            TokenElevation,
            Some((&mut info as *mut TOKEN_ELEVATION).cast()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        )
        .ok()?;
    }
    Some(info.TokenIsElevated != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notice_requires_known_non_elevation_and_an_access_failure() {
        assert!(needs_admin_notice(Some(false), true));
        for state in [Some(true), None] {
            assert!(!needs_admin_notice(state, true));
        }
        assert!(!needs_admin_notice(Some(false), false));
    }
}
