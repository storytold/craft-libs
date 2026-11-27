//! Read-only opens that no file type at the path can stall.
//!
//! `File::open` on a FIFO with no writer blocks until a writer appears. A path that was checked
//! and then replaced gives the same wait, so a type check before the open does not help. These
//! opens ask the kernel not to wait, and the caller checks the type on the open handle.

use std::fs::File;
use std::path::Path;

/// Open `path` for reading without waiting on a FIFO. On Unix `follow` false also refuses a
/// symlink in the last component (the error is not specific, so a caller that must tell a symlink
/// apart looks at `symlink_metadata` after a failed open). Other platforms open as `File::open`.
pub(crate) fn open_read(path: &Path, follow: bool) -> std::io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let flags = if follow { libc::O_NONBLOCK } else { libc::O_NONBLOCK | libc::O_NOFOLLOW };
        std::fs::OpenOptions::new().read(true).custom_flags(flags).open(path)
    }
    #[cfg(not(unix))]
    {
        let _ = follow;
        File::open(path)
    }
}
