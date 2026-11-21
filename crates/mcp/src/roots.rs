//! The folders an MCP server's tools may read from and write to.
//!
//! [`Roots`] holds read roots and write roots. A path inside any read root may be read, and a path
//! inside any write root may be written. A kind of access with no root of its own is not granted.
//! [`Roots::required`] refuses to build roots unless both kinds are given, for entry points that
//! must not start without them.
//!
//! [`Roots::check`] checks a path like this:
//!
//! - It is made absolute against the working directory, then its links are followed. For a path
//!   that does not exist yet, the links of its nearest existing folder are followed and the rest
//!   must be plain names: `..` below a folder that does not exist is refused, and so is a link
//!   that leads nowhere.
//! - The result must be a root or lie inside one, compared folder name by folder name, so `/work2`
//!   is not inside `/work`. Following the links gives existing names as the file system stores
//!   them, so a path typed in another case still matches on a file system that ignores case.
//! - On Windows these are refused, in the path as given and again in the path its links lead to:
//!   device names (`CON`, `NUL`, `COM1`, `LPT¹` and the like, also with an extension), a `:`
//!   after the drive, a file or folder name that ends in a dot or a space, a path that starts
//!   with `\\.\`, and one that starts with `\\?\` unless it is a drive or UNC path with no `/`
//!   and no empty, `.` or `..` name, such as a path [`Roots::check`] returned. The names follow
//!   <https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file>.
//!
//! A path is checked and then opened. Another program that swaps a folder for a link between the
//! two steps can still reach outside the roots: the roots confine the client's tool arguments, not
//! other programs on the computer.

use std::ffi::OsString;
use std::fmt;
use std::path::{Component, Path, PathBuf, Prefix};

/// Reading a file or writing one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

impl fmt::Display for Access {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Access::Read => "read",
            Access::Write => "write",
        })
    }
}

impl Access {
    fn verb(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "written",
        }
    }
}

/// Why a path may not be used, or a root could not be set up. `BadRoot` is a root that is not an
/// existing folder, `NoRoot` a kind [`Roots::required`] was given no root for, and `NotGranted` a
/// check for a kind that has no root.
#[derive(Debug, PartialEq, Eq)]
pub enum AccessError {
    BadRoot { access: Access, path: PathBuf, reason: String },
    NoRoot { access: Access },
    NotGranted { access: Access, path: PathBuf },
    Outside { access: Access, path: PathBuf, roots: String },
    Empty,
    Device { path: PathBuf },
    Stream { path: PathBuf },
    TrailingDotOrSpace { path: PathBuf },
    Verbatim { path: PathBuf },
    ParentOfMissing { path: PathBuf },
    BrokenLink { path: PathBuf },
    Unresolved { path: PathBuf, reason: String },
}

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccessError::BadRoot { access, path, reason } => write!(f, "the {access} root `{}` can't be used: {reason}", path.display()),
            AccessError::NoRoot { access } => write!(f, "no {access} root was given"),
            AccessError::NotGranted { access, path } => {
                write!(f, "files can't be {}: no {access} root was given: {}", access.verb(), path.display())
            }
            AccessError::Outside { access, path, roots } => write!(f, "{} is outside the {access} roots ({roots})", path.display()),
            AccessError::Empty => f.write_str("an empty path names no file"),
            AccessError::Device { path } => write!(f, "{} names a device, not a file", path.display()),
            AccessError::Stream { path } => write!(f, "{} has a `:` in a file or folder name", path.display()),
            AccessError::TrailingDotOrSpace { path } => write!(f, "{} has a file or folder name that ends in a dot or a space", path.display()),
            AccessError::Verbatim { path } => write!(f, r"{} starts with `\\?\` or `\\.\`; give the path without that prefix", path.display()),
            AccessError::ParentOfMissing { path } => write!(f, "{} goes up with `..` below a folder that does not exist", path.display()),
            AccessError::BrokenLink { path } => write!(f, "{} is a link to something that does not exist", path.display()),
            AccessError::Unresolved { path, reason } => write!(f, "{} could not be resolved: {reason}", path.display()),
        }
    }
}

impl std::error::Error for AccessError {}

/// The read and write roots, each made absolute with its links followed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roots {
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
}

impl Roots {
    /// Roots from the folders given, which must exist. A folder given twice counts once. Either
    /// list may be empty; that kind of access is then not granted. On Windows a folder whose path,
    /// with its links followed, breaks the name rules in the module documentation is refused.
    pub fn new(read: &[PathBuf], write: &[PathBuf]) -> Result<Roots, AccessError> {
        Ok(Roots { read: open_all(read, Access::Read)?, write: open_all(write, Access::Write)? })
    }

    /// [`Roots::new`], refusing roots that leave out either kind.
    pub fn required(read: &[PathBuf], write: &[PathBuf]) -> Result<Roots, AccessError> {
        if read.is_empty() {
            return Err(AccessError::NoRoot { access: Access::Read });
        }
        if write.is_empty() {
            return Err(AccessError::NoRoot { access: Access::Write });
        }
        Roots::new(read, write)
    }

    /// The read roots, absolute with their links followed.
    pub fn read(&self) -> &[PathBuf] {
        &self.read
    }

    /// The write roots, absolute with their links followed.
    pub fn write(&self) -> &[PathBuf] {
        &self.write
    }

    /// Whether neither kind of root was given.
    pub fn is_empty(&self) -> bool {
        self.read.is_empty() && self.write.is_empty()
    }

    /// `path` resolved, when it may be used for `access`; otherwise why not.
    pub fn check(&self, path: &Path, access: Access) -> Result<PathBuf, AccessError> {
        let roots = match access {
            Access::Read => &self.read,
            Access::Write => &self.write,
        };
        if roots.is_empty() {
            return Err(AccessError::NotGranted { access, path: path.to_path_buf() });
        }
        let resolved = resolve(path)?;
        if roots.iter().any(|r| inside(&resolved, r)) {
            return Ok(resolved);
        }
        Err(AccessError::Outside { access, path: path.to_path_buf(), roots: list(roots) })
    }

    /// One line naming the roots, for server instructions and logs.
    pub fn describe(&self) -> String {
        let list = |roots: &[PathBuf]| if roots.is_empty() { "none".to_owned() } else { list(roots) };
        format!("read roots {}; write roots {}", list(&self.read), list(&self.write))
    }
}

fn list(roots: &[PathBuf]) -> String {
    roots.iter().map(|r| display_path(r).display().to_string()).collect::<Vec<_>>().join(", ")
}

fn open_all(paths: &[PathBuf], access: Access) -> Result<Vec<PathBuf>, AccessError> {
    let mut out: Vec<PathBuf> = Vec::with_capacity(paths.len());
    for p in paths {
        let bad = |reason: String| AccessError::BadRoot { access, path: p.clone(), reason };
        if p.as_os_str().is_empty() {
            return Err(bad("an empty path names no folder".into()));
        }
        let resolved = std::fs::canonicalize(p).map_err(|e| bad(e.to_string()))?;
        if !resolved.is_dir() {
            return Err(bad("it is not a folder".into()));
        }
        if cfg!(windows) {
            check_resolved_names(&resolved).map_err(|e| bad(e.to_string()))?;
        }
        if !out.iter().any(|r| same_path(r, &resolved)) {
            out.push(resolved);
        }
    }
    Ok(out)
}

/// `path` without the `\\?\` prefix of Windows' canonical paths, for messages and logs. On Windows
/// a name that has no text form is shown with U+FFFD in its place. Other paths are returned
/// unchanged.
pub fn display_path(path: &Path) -> PathBuf {
    if !cfg!(windows) {
        return path.to_path_buf();
    }
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    text.strip_prefix(r"\\?\").map_or_else(|| path.to_path_buf(), PathBuf::from)
}

/// `path` made absolute with its links followed; for a path that does not exist, its nearest
/// existing folder's links followed and the remaining names added.
fn resolve(path: &Path) -> Result<PathBuf, AccessError> {
    if path.as_os_str().is_empty() {
        return Err(AccessError::Empty);
    }
    let plain;
    let path = if cfg!(windows) {
        plain = without_verbatim_prefix(path)?;
        check_windows_names(&plain)?;
        plain.as_path()
    } else {
        path
    };
    // On Windows, `std::path::absolute` removes each `..` and the name before it from the text, so
    // a `..` that follows a missing folder is refused here, before that happens.
    let mut before = PathBuf::new();
    for c in path.components() {
        if c == Component::ParentDir
            && !before.as_os_str().is_empty()
            && matches!(std::fs::symlink_metadata(&before), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
        {
            return Err(AccessError::ParentOfMissing { path: path.to_path_buf() });
        }
        before.push(c);
    }
    let unresolved = |reason: String| AccessError::Unresolved { path: path.to_path_buf(), reason };
    let absolute = std::path::absolute(path).map_err(|e| unresolved(e.to_string()))?;
    // Names below the nearest existing folder, the last one first.
    let mut missing: Vec<OsString> = Vec::new();
    let mut at = absolute.as_path();
    loop {
        match std::fs::canonicalize(at) {
            Ok(mut found) => {
                found.extend(missing.iter().rev());
                if cfg!(windows) {
                    check_resolved_names(&found)?;
                }
                return Ok(found);
            }
            Err(e) => match std::fs::symlink_metadata(at) {
                Ok(m) if m.file_type().is_symlink() => return Err(AccessError::BrokenLink { path: path.to_path_buf() }),
                Ok(_) => return Err(unresolved(e.to_string())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(unresolved(e.to_string())),
            },
        }
        match (at.components().next_back(), at.parent()) {
            (Some(Component::Normal(name)), Some(parent)) => {
                missing.push(name.to_os_string());
                at = parent;
            }
            (Some(Component::ParentDir), _) => return Err(AccessError::ParentOfMissing { path: path.to_path_buf() }),
            _ => return Err(unresolved("no part of it exists".into())),
        }
    }
}

/// A `\\?\C:\` or `\\?\UNC\` path, such as one [`Roots::check`] returned, without its prefix, when
/// every name after the prefix is one Windows would parse the same way without it: no `/`, and no
/// empty, `.` or `..` name. Other paths are returned unchanged. The names are copied as they are,
/// without a conversion to text.
fn without_verbatim_prefix(path: &Path) -> Result<PathBuf, AccessError> {
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return Ok(path.to_path_buf());
    };
    let mut plain = match prefix.kind() {
        Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:\\", char::from(drive))),
        Prefix::VerbatimUNC(server, share) => {
            let mut unc = OsString::from(r"\\");
            unc.push(server);
            unc.push(r"\");
            unc.push(share);
            PathBuf::from(unc)
        }
        _ => return Ok(path.to_path_buf()),
    };
    let refused = || AccessError::Verbatim { path: path.to_path_buf() };
    let bytes = path.as_os_str().as_encoded_bytes();
    let rest = bytes.get(prefix.as_os_str().len()..).ok_or_else(refused)?;
    // `\\?\C:foo` names `foo` in the drive's current folder, not `C:\foo`.
    let disk = matches!(prefix.kind(), Prefix::VerbatimDisk(_));
    let rest = match rest.strip_prefix(b"\\") {
        Some(rest) => rest,
        None if rest.is_empty() && !disk => rest,
        None => return Err(refused()),
    };
    if rest.contains(&b'/') || (!rest.is_empty() && rest.split(|&b| b == b'\\').any(|name| matches!(name, b"" | b"." | b".."))) {
        return Err(refused());
    }
    for c in path.components().skip(1) {
        match c {
            Component::RootDir => {}
            Component::Normal(name) => plain.push(name),
            _ => return Err(refused()),
        }
    }
    Ok(plain)
}

/// The Windows name rules applied to a path with its links followed, so a link or junction to a
/// folder whose name the rules refuse is refused too.
fn check_resolved_names(found: &Path) -> Result<(), AccessError> {
    check_windows_names(&without_verbatim_prefix(found)?)
}

/// The Windows name rules from the module documentation.
fn check_windows_names(path: &Path) -> Result<(), AccessError> {
    for c in path.components() {
        match c {
            // These prefixes turn off Windows' own name parsing for the rest of the path.
            Component::Prefix(p)
                if matches!(p.kind(), Prefix::Verbatim(_) | Prefix::VerbatimUNC(..) | Prefix::VerbatimDisk(_) | Prefix::DeviceNS(_)) =>
            {
                return Err(AccessError::Verbatim { path: path.to_path_buf() });
            }
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                if is_device_name(&name) {
                    return Err(AccessError::Device { path: path.to_path_buf() });
                }
                if name.contains(':') {
                    return Err(AccessError::Stream { path: path.to_path_buf() });
                }
                if name.ends_with(['.', ' ']) {
                    return Err(AccessError::TrailingDotOrSpace { path: path.to_path_buf() });
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Whether the resolved `path` is `root` or lies inside it, folder name by folder name. Both come
/// from `canonicalize`, which gives each existing name as the file system stores it, so names are
/// compared exactly. Folding case would make two folders equal in a Windows folder set to be
/// case-sensitive, or two names that differ beyond ASCII case.
fn inside(path: &Path, root: &Path) -> bool {
    let mut parts = path.components();
    root.components().all(|r| parts.next() == Some(r))
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.components().count() == b.components().count() && inside(a, b)
}

/// A Windows device name (`CON`, `NUL`, `COM1`, `LPT¹` and the like, also with an extension).
fn is_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ').to_ascii_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix).is_some_and(|n| {
            let mut chars = n.chars();
            chars.next().is_some_and(|c| matches!(c, '1'..='9' | '¹' | '²' | '³')) && chars.next().is_none()
        })
    };
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") || numbered("COM") || numbered("LPT")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder holding `in` (the read root), `out` (the write root), `outside` and `in2`.
    fn folders(tag: &str) -> (PathBuf, Roots) {
        let base = std::env::temp_dir().join(format!("craft-mcp-roots-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for d in ["in/sub", "out", "outside", "in2"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
        }
        std::fs::write(base.join("in/show.json"), "{}").unwrap();
        std::fs::write(base.join("outside/secret.json"), "{}").unwrap();
        let roots = Roots::new(&[base.join("in")], &[base.join("out")]).unwrap();
        (base, roots)
    }

    #[test]
    fn paths_inside_a_root_pass_and_others_are_refused() {
        let (base, roots) = folders("inside");
        assert!(roots.check(&base.join("in/show.json"), Access::Read).is_ok());
        assert!(roots.check(&base.join("in"), Access::Read).is_ok(), "the root itself");
        assert!(roots.check(&base.join("in/sub/new.json"), Access::Read).is_ok(), "a missing file is checked by its folder");
        assert!(roots.check(&base.join("out/new/deeper/x.mkv"), Access::Write).is_ok(), "and by its nearest existing folder");
        let e = roots.check(&base.join("outside/secret.json"), Access::Read).unwrap_err();
        assert!(matches!(e, AccessError::Outside { access: Access::Read, .. }), "{e}");
        assert!(matches!(roots.check(&base.join("in/show.json"), Access::Write), Err(AccessError::Outside { .. })));
        assert!(matches!(roots.check(&base.join("out/x.json"), Access::Read), Err(AccessError::Outside { .. })));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_sibling_with_the_same_prefix_is_outside() {
        let (base, roots) = folders("prefix");
        assert!(roots.check(&base.join("in2/x.json"), Access::Read).is_err());
        let text = format!("{}2{}x.json", base.join("in").display(), std::path::MAIN_SEPARATOR);
        assert!(roots.check(Path::new(&text), Access::Read).is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn parent_steps_are_resolved_or_refused() {
        let (base, roots) = folders("dotdot");
        let up = base.join("in").join("..").join("outside").join("secret.json");
        assert!(matches!(roots.check(&up, Access::Read), Err(AccessError::Outside { .. })), "an existing path is resolved first");
        let back_in = base.join("in").join("sub").join("..").join("show.json");
        assert!(roots.check(&back_in, Access::Read).is_ok(), "`..` that stays inside is allowed");
        let below_missing = base.join("in").join("nothing").join("..").join("..").join("outside").join("secret.json");
        assert!(matches!(roots.check(&below_missing, Access::Read), Err(AccessError::ParentOfMissing { .. })));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_kind_of_access_without_a_root_is_not_granted() {
        let (base, _) = folders("one-kind");
        let roots = Roots::new(&[base.join("in")], &[]).unwrap();
        let e = roots.check(&base.join("in/show.json"), Access::Write).unwrap_err();
        assert!(matches!(e, AccessError::NotGranted { access: Access::Write, .. }), "{e}");
        assert!(roots.describe().contains("write roots none"), "{}", roots.describe());
        assert_eq!(Roots::required(&[base.join("in")], &[]), Err(AccessError::NoRoot { access: Access::Write }));
        assert_eq!(Roots::required(&[], &[base.join("out")]), Err(AccessError::NoRoot { access: Access::Read }));
        assert!(Roots::required(&[base.join("in")], &[base.join("out")]).is_ok());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn several_roots_of_one_kind_each_grant_access_and_repeats_count_once() {
        let (base, _) = folders("several");
        let roots = Roots::new(&[base.join("in"), base.join("in2"), base.join("in")], &[]).unwrap();
        assert_eq!(roots.read().len(), 2);
        assert!(roots.check(&base.join("in2/x.json"), Access::Read).is_ok());
        assert!(roots.check(&base.join("in/show.json"), Access::Read).is_ok());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_root_must_be_an_existing_folder() {
        let (base, _) = folders("bad-root");
        assert!(matches!(Roots::new(&[base.join("missing")], &[]), Err(AccessError::BadRoot { .. })));
        assert!(matches!(Roots::new(&[], &[base.join("in/show.json")]), Err(AccessError::BadRoot { .. })));
        assert!(matches!(Roots::new(&[PathBuf::new()], &[]), Err(AccessError::BadRoot { .. })));
        assert!(matches!(Roots::new(&[base.join("in")], &[]).map(|r| r.check(Path::new(""), Access::Read)), Ok(Err(AccessError::Empty))));
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[test]
    fn links_that_lead_out_of_a_root_are_outside() {
        let (base, roots) = folders("links");
        std::os::unix::fs::symlink(base.join("outside"), base.join("in/escape")).unwrap();
        std::os::unix::fs::symlink(base.join("outside/none"), base.join("in/dangling")).unwrap();
        assert!(matches!(roots.check(&base.join("in/escape/secret.json"), Access::Read), Err(AccessError::Outside { .. })));
        assert!(matches!(roots.check(&base.join("in/dangling"), Access::Read), Err(AccessError::BrokenLink { .. })));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn device_names_are_recognized() {
        for name in ["CON", "nul", "COM1", "lpt9.txt", "AUX ", "COM¹", "lpt³.log", "NUL.tar.gz"] {
            assert!(is_device_name(name), "{name}");
        }
        for name in ["COM0", "COM10", "COM¹1", "console", "show.json"] {
            assert!(!is_device_name(name), "{name}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_paths_and_windows_name_rules_are_refused() {
        let (base, roots) = folders("verbatim");
        let inside = base.join("in/show.json");
        let checked = roots.check(&inside, Access::Read).unwrap();
        assert_eq!(roots.check(&checked, Access::Read).as_ref(), Ok(&checked), "a checked path checks again");
        assert!(roots.check(&roots.read()[0].join("new.json"), Access::Read).is_ok(), "a name joined to a root");
        // A folder name with an unpaired UTF-16 surrogate, which has no text form.
        let odd: OsString = std::os::windows::ffi::OsStringExt::from_wide(&[u16::from(b'a'), 0xD800]);
        std::fs::create_dir_all(base.join("in").join(&odd)).unwrap();
        let checked = roots.check(&base.join("in").join(&odd).join("x.json"), Access::Read).unwrap();
        assert_eq!(roots.check(&checked, Access::Read).as_ref(), Ok(&checked), "names are kept as they are");
        let verbatim = |p: &Path| PathBuf::from(format!(r"\\?\{}", std::path::absolute(p).unwrap().display()));
        assert!(roots.check(&verbatim(&inside), Access::Read).is_ok());
        for escape in [r"in\..\outside\secret.json", r"in\.\show.json", r"in\\show.json", r"in/show.json"] {
            let p = PathBuf::from(format!(r"{}\{escape}", verbatim(&base).display()));
            assert!(matches!(roots.check(&p, Access::Read), Err(AccessError::Verbatim { .. })), "{escape}");
        }
        let trailing = PathBuf::from(format!(r"{}\in\show.json.", verbatim(&base).display()));
        assert!(matches!(roots.check(&trailing, Access::Read), Err(AccessError::TrailingDotOrSpace { .. })));
        let drive = std::path::absolute(&inside).unwrap().to_string_lossy().chars().next().unwrap();
        for relative in [format!(r"\\?\{drive}:show.json"), format!(r"\\?\{drive}:")] {
            assert!(matches!(roots.check(Path::new(&relative), Access::Read), Err(AccessError::Verbatim { .. })), "{relative}");
        }
        let root = PathBuf::from(r"\\?\GLOBALROOT\Device\HarddiskVolume1\x.json");
        assert!(matches!(roots.check(&root, Access::Read), Err(AccessError::Verbatim { .. })));
        let device = PathBuf::from(format!(r"\\.\{}", std::path::absolute(&inside).unwrap().display()));
        assert!(matches!(roots.check(&device, Access::Read), Err(AccessError::Verbatim { .. })));
        for name in ["show.json.", "show.json ", "sub./x.json", "sub /x.json"] {
            let e = roots.check(&base.join("in").join(name), Access::Read).unwrap_err();
            assert!(matches!(e, AccessError::TrailingDotOrSpace { .. }), "{name}: {e}");
        }
        assert!(matches!(roots.check(&base.join("in/show.json:secret"), Access::Read), Err(AccessError::Stream { .. })));
        assert!(matches!(roots.check(&base.join("in/COM¹"), Access::Write), Err(AccessError::Device { .. })));
        assert!(roots.check(&base.join("IN").join("SHOW.JSON"), Access::Read).is_ok(), "a path typed in another case");
        assert!(roots.check(&base.join("IN").join("new.json"), Access::Read).is_ok(), "and a missing file below it");
        // `İ` lowercases to `i` and a combining dot, which names a different folder.
        std::fs::create_dir_all(base.join("\u{130}")).unwrap();
        std::fs::create_dir_all(base.join("i\u{307}")).unwrap();
        let dotted = Roots::new(&[base.join("\u{130}")], &[]).unwrap();
        assert!(matches!(dotted.check(&base.join("i\u{307}").join("x.json"), Access::Read), Err(AccessError::Outside { .. })));
        assert!(!display_path(&roots.read()[0]).to_string_lossy().starts_with(r"\\?\"));
        assert!(roots.check(&inside, Access::Read).is_ok());
        let _ = std::fs::remove_dir_all(base);
    }

    /// A link or junction can lead to a folder whose name the Windows rules refuse; `canonicalize`
    /// then returns that name. Making such a link needs rights the test may not have, so the check
    /// runs on paths in the form `canonicalize` returns.
    #[cfg(windows)]
    #[test]
    fn resolved_names_follow_the_windows_name_rules() {
        for ok in [r"\\?\C:\work\actual\x.txt", r"\\?\UNC\server\share\work\x.txt", r"\\?\UNC\server\share"] {
            assert_eq!(check_resolved_names(Path::new(ok)), Ok(()), "{ok}");
        }
        let refused = |p: &str| check_resolved_names(Path::new(p)).unwrap_err();
        assert!(matches!(refused(r"\\?\C:\work\actual.\x.txt"), AccessError::TrailingDotOrSpace { .. }));
        assert!(matches!(refused(r"\\?\C:\work\actual \x.txt"), AccessError::TrailingDotOrSpace { .. }));
        assert!(matches!(refused(r"\\?\C:\work\CON\x.txt"), AccessError::Device { .. }));
        assert!(matches!(refused(r"\\?\UNC\server\share\nul.txt"), AccessError::Device { .. }));
        assert!(matches!(refused(r"\\?\Volume{00000000-0000-0000-0000-000000000000}\x.txt"), AccessError::Verbatim { .. }));
    }

    #[cfg(windows)]
    #[test]
    fn a_root_whose_resolved_name_the_windows_rules_refuse_is_bad() {
        let (base, _) = folders("resolved-root");
        let trailing = PathBuf::from(format!(r"\\?\{}\trailing.", std::path::absolute(&base).unwrap().display()));
        std::fs::create_dir_all(&trailing).unwrap();
        let e = Roots::new(&[trailing], &[]).unwrap_err();
        assert!(matches!(e, AccessError::BadRoot { access: Access::Read, .. }), "{e}");
        let _ = std::fs::remove_dir_all(base);
    }
}
