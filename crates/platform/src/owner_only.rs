//! Owner-only files: a token file that only its user can read, as mode 0600 makes it on Unix. On Windows a file
//! inherits its folder's access list, so these functions replace it with the current user and SYSTEM only and read a
//! file with its security descriptor through one handle.

/// Why the file whose security descriptor is `sddl` (`O:` owner and `D:` access list, in SDDL) can't be trusted as
/// user `me`'s own: it is owned by someone else, it has no access list, or it lets an account other than `me` or SYSTEM
/// use it. `None` means it is `me`'s alone.
pub fn foreign_access(sddl: &str, me: &str) -> Option<String> {
    let _ = (sddl, me);
    todo!("SH-C26")
}

/// The SID of the current user, as text.
#[cfg(windows)]
pub fn current_user_sid() -> std::io::Result<String> {
    todo!("SH-C26")
}

/// Replaces the access list of `path` with user `me` and SYSTEM only, makes `me` the owner, and checks the result.
#[cfg(windows)]
pub fn restrict_to_owner(path: &std::path::Path, me: &str) -> std::io::Result<()> {
    let _ = (path, me);
    todo!("SH-C26")
}

/// Creates the folder `dir` private to user `me`, so a file created in it is private from the moment it exists.
#[cfg(windows)]
pub fn create_private_dir(dir: &std::path::Path, me: &str) -> std::io::Result<()> {
    let _ = (dir, me);
    todo!("SH-C26")
}

/// The security descriptor of `path` in SDDL form.
#[cfg(windows)]
pub fn sddl_of(path: &std::path::Path) -> std::io::Result<String> {
    let _ = path;
    todo!("SH-C26")
}

/// Reads at most `max` bytes of `path` and its security descriptor in SDDL form through one handle.
#[cfg(windows)]
pub fn read_with_sddl(path: &std::path::Path, max: u64) -> std::io::Result<(String, Vec<u8>)> {
    let _ = (path, max);
    todo!("SH-C26")
}
