//! The Win32 calls craft-font-store makes to put its fonts within reach of every app of the user
//! on Windows 10 1809 and later: the per-user font registry (`HKCU\Software\Microsoft\Windows
//! NT\CurrentVersion\Fonts`), `AddFontResourceW` and `RemoveFontResourceW` for the running
//! session, and the `WM_FONTCHANGE` broadcast.
//!
//! This is the one crate in `crates/` that may use `unsafe` (AGENTS.md): the session calls exist
//! only as unsafe functions. Each one is a small function of its own with `allow(unsafe_code)`.
//! On other systems the calls report that they are unsupported, or do nothing.

use std::io;
use std::path::{Path, PathBuf};

/// The per-user font key under `HKEY_CURRENT_USER`. Its values name a font (`<full name>
/// (TrueType)`) and hold the font file's full path.
pub const FONTS_KEY: &str = r"Software\Microsoft\Windows NT\CurrentVersion\Fonts";

/// Whether this process can add fonts for every app of the user: Windows 10 1809 (build 17763,
/// the first with per-user fonts) or later, and not a packaged (MSIX) app, whose registry and
/// `AppData` writes other apps don't see.
pub fn check_supported() -> Result<(), String> {
    imp::check_supported()
}

/// The path the per-user font value `name` holds, when there is one.
pub fn registered(name: &str) -> Option<PathBuf> {
    imp::registered(name)
}

/// Lists the font at `path` as the per-user font value `name`, so Windows loads it at each
/// logon, and loads it into the running session.
pub fn register(name: &str, path: &Path) -> io::Result<()> {
    imp::register(name, path)
}

/// Unloads the font at `path` from the running session, so the file can be replaced or deleted.
/// It calls `RemoveFontResourceW` until it fails, at most 1000 times: every `AddFontResourceW`
/// counts. A program that holds the file open itself still keeps it.
pub fn unload(path: &Path) {
    imp::unload(path);
}

/// Unloads the font at `path` and deletes the per-user font value `name` while it still points
/// to `path` (in any letter case).
pub fn unregister(name: &str, path: &Path) -> io::Result<()> {
    imp::unregister(name, path)
}

/// Tells every top-level window that the fonts changed (`WM_FONTCHANGE`). It waits up to a
/// second for each window that doesn't answer, so call it off the UI thread.
pub fn broadcast_font_change() {
    imp::broadcast_font_change();
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    use windows_registry::{CURRENT_USER, LOCAL_MACHINE};
    use windows_sys::Win32::Foundation::APPMODEL_ERROR_NO_PACKAGE;
    use windows_sys::Win32::Graphics::Gdi::{AddFontResourceW, RemoveFontResourceW};
    use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
    use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_FONTCHANGE};

    use super::FONTS_KEY;

    const FIRST_BUILD: u32 = 17763;

    pub fn check_supported() -> Result<(), String> {
        let build = LOCAL_MACHINE
            .open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
            .and_then(|k| k.get_string("CurrentBuildNumber"))
            .ok()
            .and_then(|b| b.trim().parse::<u32>().ok());
        if build.is_none_or(|b| b < FIRST_BUILD) {
            return Err("adding fonts for every app needs Windows 10 version 1809 or later".into());
        }
        if packaged() {
            return Err("a packaged (MSIX) app can't add fonts for other apps".into());
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    fn packaged() -> bool {
        let mut len = 0u32;
        // SAFETY: with a length of 0 and a null buffer the call writes only the length it would
        // need to `len`, which outlives the call.
        let status = unsafe { GetCurrentPackageFullName(&mut len, std::ptr::null_mut()) };
        status != APPMODEL_ERROR_NO_PACKAGE
    }

    pub fn registered(name: &str) -> Option<PathBuf> {
        CURRENT_USER.open(FONTS_KEY).ok()?.get_string(name).ok().map(PathBuf::from)
    }

    pub fn register(name: &str, path: &Path) -> io::Result<()> {
        let data = path.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the font's path isn't valid Unicode"))?;
        CURRENT_USER.create(FONTS_KEY).and_then(|k| k.set_string(name, data)).map_err(io::Error::other)?;
        if add_font_resource(path) == 0 {
            return Err(io::Error::other(format!("Windows didn't load {}", path.display())));
        }
        Ok(())
    }

    pub fn unregister(name: &str, path: &Path) -> io::Result<()> {
        unload(path);
        if registered(name).is_some_and(|p| same_path(&p, path)) {
            CURRENT_USER.create(FONTS_KEY).and_then(|k| k.remove_value(name)).map_err(io::Error::other)?;
        }
        Ok(())
    }

    /// Whether two Windows paths name the same file, ignoring letter case.
    fn same_path(a: &Path, b: &Path) -> bool {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    }

    /// `path` as a NUL-terminated UTF-16 string.
    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    #[allow(unsafe_code)]
    fn add_font_resource(path: &Path) -> i32 {
        let wide = wide(path);
        // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the call, which only
        // reads it.
        unsafe { AddFontResourceW(wide.as_ptr()) }
    }

    #[allow(unsafe_code)]
    pub fn unload(path: &Path) {
        let wide = wide(path);
        for _ in 0..1000 {
            // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the call, which only
            // reads it.
            if unsafe { RemoveFontResourceW(wide.as_ptr()) } == 0 {
                break;
            }
        }
    }

    #[allow(unsafe_code)]
    pub fn broadcast_font_change() {
        let mut result = 0usize;
        // SAFETY: `WM_FONTCHANGE` carries no pointers (both parameters are 0), and `result`
        // outlives the call, which only writes it.
        unsafe { SendMessageTimeoutW(HWND_BROADCAST, WM_FONTCHANGE, 0, 0, SMTO_ABORTIFHUNG, 1000, &mut result) };
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;
    use std::path::{Path, PathBuf};

    pub fn check_supported() -> Result<(), String> {
        Err("the Windows font registry exists only on Windows".into())
    }

    pub fn registered(_name: &str) -> Option<PathBuf> {
        None
    }

    pub fn register(_name: &str, _path: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    pub fn unload(_path: &Path) {}

    pub fn unregister(_name: &str, _path: &Path) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    pub fn broadcast_font_change() {}
}
