//! Owner-only files: a token file that only its user can read, as mode 0600 makes it on Unix.
//!
//! On Windows a file created in a folder inherits the folder's access list, and folders such as
//! a new folder at a drive root, `C:\Users\Public` or `C:\ProgramData` let every signed-in user
//! read what is created in them. `restrict_to_owner` replaces the whole list with the current
//! user and SYSTEM only, makes that user the owner, and checks the result. `create_private_dir`
//! creates a folder with that owner and list from the start, which the files created in it
//! inherit, so a file can be private from the moment it exists. `read_with_sddl` reads a file and its security descriptor
//! through one handle. [`foreign_access`] reads the descriptor in SDDL form, which names accounts
//! by SID and so reads the same in every Windows language, and says why another user could have
//! written or could read the file. They run the tools Windows ships rather than calling the
//! security API directly, which would need `unsafe`. A run of a tool that takes longer than
//! `TOOL_LIMIT` (10 s) is ended, and the function fails with an error of kind
//! [`std::io::ErrorKind::TimedOut`].

/// Why the file whose security descriptor is `sddl` (`O:` owner and `D:` access list, in SDDL or
/// as `read_with_sddl` writes it) can't be trusted as user `me`'s own: it is owned by someone else, it has no
/// access list (everyone may use it), or it lets an account other than `me` or SYSTEM use it.
/// Accounts are compared by SID; `None` means it is `me`'s alone.
///
/// Stricter than OpenSSH's rule for private keys, which also lets Administrators in: an entry for
/// the Administrators group would let another administrator read the token without taking the
/// file over. `restrict_to_owner` never grants it, so the app's own file always passes; a file
/// that inherited it from a user's folder is refused.
pub fn foreign_access(sddl: &str, me: &str) -> Option<String> {
    foreign(sddl, me, false)
}

/// [`foreign_access`] for a folder's descriptor: an inherit-only entry (`IO`) counts too, since
/// every file and folder created in the folder gets it.
#[cfg(any(test, windows))]
fn foreign_folder_access(sddl: &str, me: &str) -> Option<String> {
    foreign(sddl, me, true)
}

/// [`foreign_access`], counting inherit-only entries when `inherit_only` is set.
fn foreign(sddl: &str, me: &str, inherit_only: bool) -> Option<String> {
    let me = me.trim();
    let owner = section(sddl, "O:");
    match owner {
        Some(o) if same_account(o, me) => {}
        Some(o) => return Some(format!("it is owned by {}", another_account(o, ""))),
        None => return Some("its owner can't be read".into()),
    }
    let Some(dacl) = section(sddl, "D:") else {
        return Some("it has no access list, so every user may read it".into());
    };
    if dacl.starts_with("NO_ACCESS_CONTROL") {
        return Some("it has no access control, so every user may read it".into());
    }
    for ace in dacl.split('(').skip(1) {
        let ace = ace.split(')').next().unwrap_or_default();
        let parts: Vec<&str> = ace.split(';').collect();
        // `type;flags;rights;object guid;inherited object guid;account SID[;(condition)]`.
        let (Some(kind), Some(flags)) = (parts.first(), parts.get(1)) else { continue };
        // Deny entries take access away, and inherit-only entries apply to children, not the file.
        let allows = matches!(*kind, "A" | "OA" | "XA" | "ZA");
        if !allows || (flags.contains("IO") && !inherit_only) {
            continue;
        }
        // An entry that grants access but names no account can't be shown to be the user's.
        let Some(sid) = parts.get(5).filter(|s| !s.is_empty()) else {
            return Some(format!("its access list has an entry that can't be read ({ace})"));
        };
        if !(same_account(sid, me) || matches!(*sid, "SY" | "S-1-5-18" | "OW" | "S-1-3-4")) {
            return Some(another_account(sid, "has access to it"));
        }
    }
    None
}

/// `another account (<sid>) <rest>`, with the account's name before its SID when it is one of the
/// well-known accounts a token file can carry, such as `another account, Administrators
/// (S-1-5-32-544), has access to it`. The SID stays, so the message reads the same in every
/// Windows language and can still be looked up.
fn another_account(sid: &str, rest: &str) -> String {
    match (well_known_name(sid), rest.is_empty()) {
        (Some(name), true) => format!("another account, {name} ({sid})"),
        (Some(name), false) => format!("another account, {name} ({sid}), {rest}"),
        (None, true) => format!("another account ({sid})"),
        (None, false) => format!("another account ({sid}) {rest}"),
    }
}

/// The English name Windows gives a well-known SID that can be on a file's access list or own it:
/// those a folder hands down at a drive root, in `C:\Users\Public` and in a user's profile, and
/// Everyone. The SIDs and names are Microsoft's public list of well-known SIDs
/// (<https://learn.microsoft.com/en-us/windows/win32/secauthz/well-known-sids>); the names are
/// the ones `icacls` shows, without the `BUILTIN\` or `NT AUTHORITY\` prefix. Any other SID,
/// a user's or a domain group's, has no fixed name and gets `None`.
fn well_known_name(sid: &str) -> Option<&'static str> {
    const NAMES: [(&str, &str); 8] = [
        ("S-1-1-0", "Everyone"),
        ("S-1-5-3", "BATCH"),
        ("S-1-5-4", "INTERACTIVE"),
        ("S-1-5-6", "SERVICE"),
        ("S-1-5-11", "Authenticated Users"),
        ("S-1-5-18", "SYSTEM"),
        ("S-1-5-32-544", "Administrators"),
        ("S-1-5-32-545", "Users"),
    ];
    NAMES.iter().find(|(known, _)| known.eq_ignore_ascii_case(sid)).map(|&(_, name)| name)
}

/// `section(sddl, "O:")` is the owner, `"D:"` the access list: the text after the tag up to the
/// next top-level tag.
fn section<'a>(sddl: &'a str, tag: &str) -> Option<&'a str> {
    let start = sddl.find(tag)?.checked_add(tag.len())?;
    let rest = sddl.get(start..)?;
    // The owner and group are bare SIDs; the access lists run until the next `X:` tag outside
    // parentheses.
    let mut depth = 0usize;
    let mut end = rest.len();
    for (i, c) in rest.char_indices() {
        match c {
            '(' => depth = depth.saturating_add(1),
            ')' => depth = depth.saturating_sub(1),
            ':' if depth == 0 && i > 0 => {
                // The tag is the one letter before this colon.
                end = rest.get(..i).and_then(|s| s.char_indices().last()).map_or(i, |(j, _)| j);
                break;
            }
            _ => {}
        }
    }
    rest.get(..end).map(str::trim)
}

/// Whether account `a` names user SID `me`. `OW` (owner rights) and `CO` are not users. An alias
/// is never taken for the user: [`read_with_sddl`] gives every account as a full SID, because
/// Windows' own SDDL writes some users by alias (the built-in Administrator as `LA`, whose SID
/// depends on the machine).
fn same_account(a: &str, me: &str) -> bool {
    a.eq_ignore_ascii_case(me)
}

/// The tools Windows ships that these checks run, by their path under `%SystemRoot%\System32`.
#[cfg(windows)]
const WHOAMI: &str = "whoami.exe";
#[cfg(windows)]
const POWERSHELL: &str = r"WindowsPowerShell\v1.0\powershell.exe";
#[cfg(windows)]
const ICACLS: &str = "icacls.exe";

/// A tool Windows ships, started by its full path. Started by bare name, a `whoami.exe`,
/// `powershell.exe` or `icacls.exe` found first (next to the app, or earlier on the search path)
/// would answer instead, and `whoami`'s answer is what every check here trusts.
///
/// Each tool is a console program and the app is not, so without `CREATE_NO_WINDOW` Windows gives
/// every one of them a console window of its own, which flashes up while the app writes its
/// token file. The flag doesn't touch the pipes `output()` reads the answer through.
#[cfg(windows)]
fn system_tool(relative: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt as _;
    /// `CREATE_NO_WINDOW`, from the Windows process creation flags.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let root = std::env::var_os("SystemRoot").map_or_else(|| std::path::PathBuf::from(r"C:\Windows"), std::path::PathBuf::from);
    let mut command = std::process::Command::new(root.join("System32").join(relative));
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// How long one run of a tool may take before it is ended. Each tool finishes in a second or two.
/// On some computers a new process is left suspended (security software holding it), and without
/// a limit the caller would wait on it for as long as it stays that way.
#[cfg(windows)]
pub const TOOL_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// How often [`run_until`] checks whether the tool has exited.
#[cfg(any(test, windows))]
const POLL: std::time::Duration = std::time::Duration::from_millis(20);

/// How many more checks [`run_until`] makes for a tool it has ended to exit before it stops
/// waiting on it.
#[cfg(any(test, windows))]
const EXIT_POLLS: u32 = 50;

/// How a run of [`run_until`] ended.
#[cfg(any(test, windows))]
enum Ran {
    /// The tool exited by itself: its status and output, and how writing its standard input went.
    Exited(std::process::Output, std::io::Result<()>),
    /// The time was up first and the tool was ended. `exited` is whether it was then seen to exit.
    Ended { exited: bool },
}

/// Start `command` with its standard input, output and error piped, hand its standard input to
/// `feed` on a thread of its own, and wait for it to exit. Between checks, `expired` gets what
/// the tool has written to its standard output so far; when it answers `true` first, the tool is
/// ended ([`std::process::Child::kill`]). Standard output and error are read on threads of their
/// own, so a tool that writes more than a pipe holds is not stopped by it.
#[cfg(any(test, windows))]
fn run_until(
    mut command: std::process::Command,
    feed: impl FnOnce(std::process::ChildStdin) -> std::io::Result<()> + Send + 'static,
    mut expired: impl FnMut(&[u8]) -> bool,
) -> std::io::Result<Ran> {
    use std::io::Read as _;
    use std::process::Stdio;
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let fed = child.stdin.take().map(|stdin| std::thread::spawn(move || feed(stdin)));
    let (chunks, received) = std::sync::mpsc::channel::<Vec<u8>>();
    if let Some(mut out) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n @ 1..) = out.read(&mut buf) {
                if chunks.send(buf.get(..n).unwrap_or_default().to_vec()).is_err() {
                    break;
                }
            }
        });
    }
    let errors = child.stderr.take().map(|mut err| {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            let _ = err.read_to_end(&mut text);
            text
        })
    });
    let mut stdout = Vec::new();
    loop {
        stdout.extend(received.try_iter().flatten());
        if let Some(status) = child.try_wait()? {
            // The pipes close when the tool exits, which ends both readers.
            stdout.extend(received.iter().flatten());
            let stderr = errors.and_then(|e| e.join().ok()).unwrap_or_default();
            let sent = match fed.map(std::thread::JoinHandle::join) {
                Some(Ok(sent)) => sent,
                Some(Err(_)) => Err(std::io::Error::other("the thread writing the standard input stopped")),
                None => Err(std::io::Error::other("the standard input could not be opened")),
            };
            return Ok(Ran::Exited(std::process::Output { status, stdout, stderr }, sent));
        }
        if expired(&stdout) {
            let _ = child.kill();
            let mut exited = false;
            for _ in 0..EXIT_POLLS {
                if child.try_wait().ok().flatten().is_some() {
                    exited = true;
                    break;
                }
                std::thread::sleep(POLL);
            }
            return Ok(Ran::Ended { exited });
        }
        std::thread::sleep(POLL);
    }
}

/// Run `command`, with nothing on its standard input, for at most [`TOOL_LIMIT`]: its status and
/// output, or an error of kind [`std::io::ErrorKind::TimedOut`] when it was ended.
#[cfg(windows)]
fn run(command: std::process::Command) -> std::io::Result<std::process::Output> {
    let deadline = std::time::Instant::now() + TOOL_LIMIT;
    let program = tool_name(&command);
    let close = |stdin: std::process::ChildStdin| {
        drop(stdin);
        Ok(())
    };
    match run_until(command, close, |_| std::time::Instant::now() >= deadline)? {
        Ran::Exited(out, _) => Ok(out),
        Ran::Ended { .. } => Err(timed_out(&program)),
    }
}

/// The file name of the program `command` starts, for messages.
#[cfg(windows)]
fn tool_name(command: &std::process::Command) -> String {
    let program = std::path::Path::new(command.get_program());
    program.file_name().unwrap_or(program.as_os_str()).to_string_lossy().into_owned()
}

/// The error for a run of `program` that [`run_until`] ended.
#[cfg(windows)]
fn timed_out(program: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::TimedOut, format!("{program} did not finish within {} s and was ended", TOOL_LIMIT.as_secs()))
}

/// The current user's SID (`whoami /user`). It starts a process, so a caller that needs it for
/// several steps reads it once and passes it to each.
#[cfg(windows)]
pub fn current_user_sid() -> std::io::Result<String> {
    let mut whoami = system_tool(WHOAMI);
    whoami.args(["/user", "/fo", "csv", "/nh"]);
    let out = run(whoami)?;
    if !out.status.success() {
        return Err(std::io::Error::other("whoami /user failed"));
    }
    sid_from_whoami(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| std::io::Error::other("whoami /user printed no SID"))
}

#[cfg(any(test, windows))]
/// The SID in `whoami /user /fo csv /nh` output, `"DOMAIN\user","S-1-5-21-…"`: the last quoted
/// field, when it is one. Anything else (an error message, nothing) gives `None`.
fn sid_from_whoami(text: &str) -> Option<String> {
    let last = text.trim().rsplit(',').next()?.trim().trim_matches('"');
    let digits = last.strip_prefix("S-1-")?;
    // Every part after `S-1-` is a number; an empty remainder is one empty part, so it fails too.
    digits.split('-').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())).then(|| last.to_string())
}

/// Replace `path`'s access list with one that only user `me` (the current user's SID, from
/// [`current_user_sid`], which the caller reads once for every step) and SYSTEM are on, then make
/// that user its owner (`icacls /setowner`), then read the file's descriptor back and check it with
/// [`foreign_access`]. The whole list is replaced, not just what was inherited: on the
/// windows-latest runner a new file in the temp folder carries an Administrators entry that `icacls
/// /inheritance:r` leaves in place. The owner is set because a file an elevated administrator creates
/// is owned by the Administrators group, not by the user. The list comes first: its owner may always
/// replace it, while setting the owner needs the Full control it grants (a folder may give Modify).
#[cfg(windows)]
pub fn restrict_to_owner(path: &std::path::Path, me: &str) -> std::io::Result<()> {
    set_access_list(path, me)?;
    icacls(path, &["/setowner".into(), format!("*{me}")])?;
    still_foreign(&sddl_of(path)?, me)
}

/// Create the folder `dir`, which must not exist yet and whose parent must, with user `me` (as for
/// [`restrict_to_owner`]) as its owner and an access list of that user and SYSTEM only, which every
/// file created in it inherits, given to Windows in the call that creates the folder. The folder
/// never has its parent's access list, and a file created in it is private from the moment it
/// exists. The folder is created beside `dir` under a name of its own
/// (`.<name>.<process id>.<count>.tmp`) and then moved to `dir`, which fails when `dir` exists.
/// Then check its descriptor, inherit-only entries included. An existing `dir` is an error of kind
/// [`std::io::ErrorKind::AlreadyExists`] and a missing parent one of kind
/// [`std::io::ErrorKind::NotFound`]; the folder is removed again when the check fails.
///
/// PowerShell is ended when it takes longer than [`TOOL_LIMIT`], and the call fails with an error
/// of kind [`std::io::ErrorKind::TimedOut`]; the folder under its own name is then removed once
/// PowerShell has exited. A folder PowerShell had already moved to `dir` stays there, with the
/// access list it was created with: this call can't tell it from a folder another process put at
/// `dir` since, so it doesn't remove what is there.
#[cfg(windows)]
pub fn create_private_dir(dir: &std::path::Path, me: &str) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    /// Numbers the staged folders of this process.
    static COUNT: AtomicU64 = AtomicU64::new(0);
    // No double quotes: the command line would strip them. `Directory.CreateDirectory` also creates
    // a missing parent and returns an existing folder as it is, hence the checks before it.
    // `Directory.Move` doesn't replace an existing folder (`std::fs::rename` does, when it is
    // empty).
    const SCRIPT: &str = "$ErrorActionPreference = 'Stop'; \
        if (Test-Path -LiteralPath $env:CRAFT_ACL_PATH) { exit 3 }; \
        if (-not [IO.Directory]::Exists([IO.Path]::GetDirectoryName($env:CRAFT_ACL_PATH))) { exit 4 }; \
        if (Test-Path -LiteralPath $env:CRAFT_ACL_STAGED) { exit 5 }; \
        $s = New-Object Security.AccessControl.DirectorySecurity; \
        $s.SetSecurityDescriptorSddlForm('O:' + $env:CRAFT_ACL_USER + 'D:P(A;OICI;FA;;;' + $env:CRAFT_ACL_USER + ')(A;OICI;FA;;;SY)'); \
        [void][IO.Directory]::CreateDirectory($env:CRAFT_ACL_STAGED, $s); \
        try { [IO.Directory]::Move($env:CRAFT_ACL_STAGED, $env:CRAFT_ACL_PATH) } \
        catch { [IO.Directory]::Delete($env:CRAFT_ACL_STAGED); if (Test-Path -LiteralPath $env:CRAFT_ACL_PATH) { exit 3 }; throw }";
    // A relative `dir` such as `private` has no parent in its text; the script needs one.
    let dir = &std::path::absolute(dir)?;
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let staged = dir.with_file_name(format!(".{name}.{}.{}.tmp", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
    let mut powershell = system_tool(POWERSHELL);
    powershell
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("CRAFT_ACL_PATH", dir)
        .env("CRAFT_ACL_STAGED", &staged)
        .env("CRAFT_ACL_USER", me);
    let deadline = std::time::Instant::now() + TOOL_LIMIT;
    let close = |stdin: std::process::ChildStdin| {
        drop(stdin);
        Ok(())
    };
    let out = match run_until(powershell, close, |_| std::time::Instant::now() >= deadline)? {
        Ran::Exited(out, _) => out,
        Ran::Ended { exited } => {
            if exited {
                let _ = std::fs::remove_dir(&staged);
            }
            return Err(timed_out("powershell.exe"));
        }
    };
    match out.status.code() {
        Some(3) => return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "the folder already exists")),
        Some(4) => return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "the folder's parent does not exist")),
        _ if !out.status.success() => {
            return Err(std::io::Error::other(format!(
                "PowerShell could not create the folder for its owner only: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        _ => {}
    }
    let checked = sddl_of(dir).and_then(|sddl| match foreign_folder_access(&sddl, me) {
        None => Ok(()),
        Some(problem) => Err(std::io::Error::other(format!("the folder could not be restricted to its owner: {problem}"))),
    });
    if checked.is_err() {
        let _ = std::fs::remove_dir(dir);
    }
    checked
}

/// Create the file `path`, which must not exist yet, with user `me` (as for [`restrict_to_owner`])
/// as its owner and an access list of that user and SYSTEM only, given to Windows in the call that
/// creates the file, so the file never has its folder's access list, and write `contents` through
/// that handle, opened unshared. The file is created beside `path` under a name of its own
/// (`.<name>.<process id>.<count>.tmp`) and takes the name `path` only once `contents` are
/// written, so `path` never holds a partly written file. Then check the file's descriptor as
/// [`restrict_to_owner`] does. An existing `path` is an error of kind
/// [`std::io::ErrorKind::AlreadyExists`] and is left as it is. A file that was created but could
/// not be written or checked is removed again. `contents` reach PowerShell through its standard
/// input, never as script text or an environment variable.
///
/// PowerShell is ended when it takes longer than [`TOOL_LIMIT`], and the call fails with an error
/// of kind [`std::io::ErrorKind::TimedOut`]. The file under its own name is then removed. If
/// PowerShell is not seen to exit after being ended, that file may stay, and a file PowerShell had
/// already moved to `path` stays there: this call can't tell it from a file another process put at
/// `path` since. Either still has the access list given when it was created.
#[cfg(windows)]
pub fn create_private_file(path: &std::path::Path, me: &str, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let deadline = std::time::Instant::now() + TOOL_LIMIT;
    let contents = contents.to_vec();
    create_private_file_until(path, me, move |mut stdin| stdin.write_all(&contents), |_| std::time::Instant::now() >= deadline)
}

/// [`create_private_file`], with `feed` writing the contents to PowerShell's standard input and
/// `expired` saying when to end it, as [`run_until`] takes them.
#[cfg(windows)]
fn create_private_file_until(
    path: &std::path::Path,
    me: &str,
    feed: impl FnOnce(std::process::ChildStdin) -> std::io::Result<()> + Send + 'static,
    expired: impl FnMut(&[u8]) -> bool,
) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    /// The exit code the script ends with when the file already exists.
    const EXISTS: i32 = 3;
    /// Numbers the staged files of this process.
    static COUNT: AtomicU64 = AtomicU64::new(0);
    // No double quotes: the command line would strip them. -2147024816 and -2147024713 are the
    // HRESULTs of ERROR_FILE_EXISTS and ERROR_ALREADY_EXISTS. `File.Move` fails when the
    // destination exists.
    const SCRIPT: &str = "$ErrorActionPreference = 'Stop'; \
        if (Test-Path -LiteralPath $env:CRAFT_ACL_PATH) { exit 3 }; \
        $s = New-Object Security.AccessControl.FileSecurity; \
        $s.SetSecurityDescriptorSddlForm('O:' + $env:CRAFT_ACL_USER + 'D:P(A;;FA;;;' + $env:CRAFT_ACL_USER + ')(A;;FA;;;SY)'); \
        $fs = New-Object IO.FileStream -ArgumentList @($env:CRAFT_ACL_STAGED, [IO.FileMode]::CreateNew, \
        [Security.AccessControl.FileSystemRights]::Write, [IO.FileShare]::None, 4096, [IO.FileOptions]::None, $s); \
        try { [Console]::OpenStandardInput().CopyTo($fs); $fs.Flush($true); $fs.Dispose() } \
        catch { $fs.Dispose(); [IO.File]::Delete($env:CRAFT_ACL_STAGED); throw }; \
        try { [IO.File]::Move($env:CRAFT_ACL_STAGED, $env:CRAFT_ACL_PATH) } \
        catch { [IO.File]::Delete($env:CRAFT_ACL_STAGED); $e = $_.Exception; while ($e.InnerException) { $e = $e.InnerException }; \
        if ($e.HResult -eq -2147024816 -or $e.HResult -eq -2147024713) { exit 3 }; throw }";
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let staged = path.with_file_name(format!(".{name}.{}.{}.tmp", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
    let mut powershell = system_tool(POWERSHELL);
    powershell
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("CRAFT_ACL_PATH", path)
        .env("CRAFT_ACL_STAGED", &staged)
        .env("CRAFT_ACL_USER", me);
    // A script that stops before reading (the file exists) closes the pipe; its exit code says why.
    let ran = run_until(powershell, feed, expired);
    // The staged name is this call's own, so removing it never removes another process's file.
    // PowerShell holds the staged file open unshared until it exits.
    let (out, sent) = match ran? {
        Ran::Exited(out, sent) => (out, sent),
        Ran::Ended { exited } => {
            if exited {
                let _ = std::fs::remove_file(&staged);
            }
            return Err(timed_out("powershell.exe"));
        }
    };
    if out.status.code() == Some(EXISTS) {
        return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "the file already exists"));
    }
    if !out.status.success() {
        let _ = std::fs::remove_file(&staged);
        return Err(std::io::Error::other(format!(
            "PowerShell could not create the file for its owner only: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let checked = sent.and_then(|()| sddl_of(path)).and_then(|sddl| still_foreign(&sddl, me));
    if checked.is_err() {
        let _ = std::fs::remove_file(path);
    }
    checked
}

/// `Err` naming the problem when the descriptor read back after restricting still isn't `me`'s
/// alone, so a file that only looks restricted is never used as one.
#[cfg(any(test, windows))]
fn still_foreign(sddl: &str, me: &str) -> std::io::Result<()> {
    match foreign_access(sddl, me) {
        None => Ok(()),
        Some(problem) => Err(std::io::Error::other(format!("the file could not be restricted to its owner: {problem}"))),
    }
}

/// Set file `path`'s access list to Full control for `me` and SYSTEM and nothing else, protected
/// from inheritance. Only the list is written, not the owner. `me` and the path reach PowerShell
/// through environment variables, never as script text.
#[cfg(windows)]
fn set_access_list(path: &std::path::Path, me: &str) -> std::io::Result<()> {
    // No double quotes: the command line would strip them.
    const SCRIPT: &str = "$s = New-Object Security.AccessControl.FileSecurity; \
        $s.SetSecurityDescriptorSddlForm('D:P(A;;FA;;;' + $env:CRAFT_ACL_USER + ')(A;;FA;;;SY)', 'Access'); \
        [IO.File]::SetAccessControl($env:CRAFT_ACL_PATH, $s)";
    let mut powershell = system_tool(POWERSHELL);
    powershell.args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT]).env("CRAFT_ACL_PATH", path).env("CRAFT_ACL_USER", me);
    let out = run(powershell)?;
    if out.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("PowerShell could not restrict the file to its owner: {}", String::from_utf8_lossy(&out.stderr).trim())))
    }
}

#[cfg(windows)]
fn icacls(path: &std::path::Path, args: &[String]) -> std::io::Result<()> {
    let mut icacls = system_tool(ICACLS);
    icacls.arg(path).args(args);
    let out = run(icacls)?;
    if out.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("icacls could not restrict the file to its owner: {}", String::from_utf8_lossy(&out.stderr).trim())))
    }
}

/// PowerShell that turns `$raw`, a `RawSecurityDescriptor`, into `$sddl` as [`read_with_sddl`]
/// describes it. No double quotes: the command line would strip them. An entry with no account (a
/// custom entry) is written with none, and `foreign_access` refuses it.
#[cfg(windows)]
macro_rules! describe_raw {
    () => {
        "$sddl = 'O:' + $raw.Owner.Value;         if ($raw.DiscretionaryAcl -ne $null) { $sddl += 'D:'; foreach ($a in $raw.DiscretionaryAcl) {             $t = 'A'; if ([string]$a.AceType -like 'AccessDenied*') { $t = 'D' };             $f = ''; if ($a.AceFlags.HasFlag([Security.AccessControl.AceFlags]::InheritOnly)) { $f = 'IO' };             $sid = ''; if ($a.SecurityIdentifier) { $sid = $a.SecurityIdentifier.Value };             $sddl += '(' + $t + ';' + $f + ';' + $a.AccessMask + ';;;' + $sid + ')' } }; "
    };
}

/// Open `path` shared for reading only, so that while it is open it can't be replaced, renamed or
/// written, and read through that one handle both its security descriptor (owner and access list)
/// and its first `max + 1` bytes. The descriptor checked is the one of the handle the bytes come
/// from, so there is no moment at which the path could name a different file (a time-of-check to
/// time-of-use race). The path reaches PowerShell through an environment variable, never as script
/// text.
///
/// The descriptor is written in SDDL's shape but with every account as a full SID, read from the
/// binary descriptor (`RawSecurityDescriptor`): `O:<owner>D:(<type>;<flags>;<rights>;;;<sid>)…`,
/// where the type is `D` for a deny entry and `A` for any other, and the flags are `IO` for an
/// inherit-only entry. Windows' own SDDL writes some accounts by alias, and the built-in
/// Administrator's alias `LA` stands for a SID that depends on the machine.
#[cfg(windows)]
pub fn read_with_sddl(path: &std::path::Path, max: u64) -> std::io::Result<(String, Vec<u8>)> {
    const SCRIPT: &str = concat!(
        "$fs = [IO.File]::Open($env:CRAFT_ACL_PATH, 'Open', 'Read', 'Read'); try {         $raw = New-Object Security.AccessControl.RawSecurityDescriptor -ArgumentList @($fs.GetAccessControl().GetSecurityDescriptorBinaryForm(), 0); ",
        describe_raw!(),
        "$max = [int]$env:CRAFT_ACL_MAX + 1; $buf = New-Object byte[] $max; $total = 0;         while ($total -lt $max) { $n = $fs.Read($buf, $total, $max - $total); if ($n -le 0) { break }; $total += $n };         [Console]::Out.Write($sddl + [char]10 + [BitConverter]::ToString($buf, 0, $total)) } finally { $fs.Dispose() }"
    );
    // Bounded so the script's `[int]` and buffer stay small; a token file is far below this.
    let max = max.min(1 << 20);
    let mut powershell = system_tool(POWERSHELL);
    powershell.args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT]).env("CRAFT_ACL_PATH", path).env("CRAFT_ACL_MAX", max.to_string());
    let out = run(powershell)?;
    let read = if out.status.success() { sddl_and_bytes(&String::from_utf8_lossy(&out.stdout)) } else { None };
    read.ok_or_else(|| std::io::Error::other("PowerShell could not read the file with its owner and access list"))
}

/// The security descriptor of `path` as [`read_with_sddl`] writes it, read by name without opening
/// the file's contents, so it works on a file that is open for writing (an app restricts its
/// token file before writing the token through the handle it created). A folder is read as a
/// folder, so its inherit-only entries are included (`IO`); a file's reading leaves them out.
#[cfg(windows)]
pub fn sddl_of(path: &std::path::Path) -> std::io::Result<String> {
    const SCRIPT: &str = concat!(
        // Stop: a failed read (a missing file) must end the script with an error, not print `O:`.
        "$ErrorActionPreference = 'Stop'; $p = $env:CRAFT_ACL_PATH; \
        if ([IO.Directory]::Exists($p)) { $sec = [IO.Directory]::GetAccessControl($p) } else { $sec = [IO.File]::GetAccessControl($p) }; \
        $raw = New-Object Security.AccessControl.RawSecurityDescriptor -ArgumentList @($sec.GetSecurityDescriptorBinaryForm(), 0); ",
        describe_raw!(),
        "[Console]::Out.Write($sddl + [char]10)"
    );
    let mut powershell = system_tool(POWERSHELL);
    powershell.args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT]).env("CRAFT_ACL_PATH", path);
    let out = run(powershell)?;
    let read = if out.status.success() { sddl_and_bytes(&String::from_utf8_lossy(&out.stdout)) } else { None };
    read.map(|(sddl, _)| sddl).ok_or_else(|| std::io::Error::other("PowerShell could not read the file's owner and access list"))
}

/// [`read_with_sddl`]'s output: the SDDL, a newline, then the bytes as `[BitConverter]::ToString`
/// prints them (`7B-22-…`, empty for none). Anything else gives `None`.
#[cfg(any(test, windows))]
fn sddl_and_bytes(text: &str) -> Option<(String, Vec<u8>)> {
    let (sddl, hex) = text.split_once('\n')?;
    let sddl = sddl.trim();
    if !sddl.contains("O:") {
        return None;
    }
    let hex = hex.trim();
    if hex.is_empty() {
        return Some((sddl.to_string(), Vec::new()));
    }
    // Two hex digits each; `from_str_radix` alone would also take a sign, as in `+7`.
    let byte = |pair: &str| (pair.len() == 2 && pair.bytes().all(|b| b.is_ascii_hexdigit())).then(|| u8::from_str_radix(pair, 16).ok()).flatten();
    let bytes = hex.split('-').map(byte).collect::<Option<Vec<u8>>>()?;
    Some((sddl.to_string(), bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "S-1-5-21-1-2-3-1001";

    /// The SID of the user running the tests, as a caller reads it once and passes on.
    #[cfg(windows)]
    fn user() -> String {
        current_user_sid().unwrap()
    }

    #[test]
    fn the_users_own_file_with_system_is_trusted_and_administrators_are_not() {
        assert_eq!(foreign_access(&format!("O:{ME}G:{ME}D:P(A;;FA;;;{ME})(A;;FA;;;SY)"), ME), None);
        assert_eq!(foreign_access(&format!("O:{ME}D:(A;;2032127;;;S-1-5-18)(A;;2032127;;;{ME})"), ME), None);
        // Another administrator could read it through the Administrators group, by alias or by SID.
        for admins in ["BA", "S-1-5-32-544"] {
            let sddl = format!("O:{ME}D:PAI(A;;FA;;;SY)(A;;FR;;;{admins})(A;;FA;;;{ME})");
            assert!(foreign_access(&sddl, ME).is_some_and(|p| p.contains(admins)), "{sddl}");
        }
        // Inherit-only entries and deny entries don't grant access to the file itself.
        assert_eq!(foreign_access(&format!("O:{ME}D:(A;OICIIO;GA;;;CO)(D;;FA;;;BU)(A;;FA;;;{ME})"), ME), None);
    }

    /// Each tool is started by its full path in System32, never looked up by name.
    #[cfg(windows)]
    #[test]
    fn the_tools_are_started_from_system32() {
        for tool in [WHOAMI, POWERSHELL, ICACLS] {
            let command = system_tool(tool);
            let path = std::path::Path::new(command.get_program());
            assert!(path.is_absolute() && path.is_file(), "{path:?}");
            assert!(path.to_string_lossy().to_ascii_lowercase().contains(r"\system32\"), "{path:?}");
        }
    }

    /// The tools start without a console window: the app has no console, so a tool that isn't
    /// told otherwise gets a window of its own, which flashes up. The started PowerShell reports
    /// its console's window and the processes attached to that console. Both are checked, because
    /// a tool started without the flag by a process that has a console shares it instead, and that
    /// console may itself have no window (a test run started by a service or an IDE).
    #[cfg(windows)]
    #[test]
    fn the_tools_start_without_a_console_window() {
        const PROBE: &str = r#"Add-Type -Namespace CraftProbe -Name Console -MemberDefinition '[DllImport("kernel32.dll")] public static extern System.IntPtr GetConsoleWindow(); [DllImport("kernel32.dll")] public static extern uint GetConsoleProcessList(uint[] ids, uint count);'
$ids = New-Object uint32[] 256
$n = [CraftProbe.Console]::GetConsoleProcessList($ids, 256)
"$([CraftProbe.Console]::GetConsoleWindow()) $($ids[0..($n-1)] -join ',')""#;
        let out = system_tool(POWERSHELL).args(["-NoProfile", "-NonInteractive", "-Command", PROBE]).output().unwrap();
        // The answer still comes back through the pipe.
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{text} {}", String::from_utf8_lossy(&out.stderr));
        let (window, attached) = text.trim().split_once(' ').unwrap();
        assert_eq!(window, "0", "the tool has a console window: {text}");
        let attached: Vec<u32> = attached.split(',').map(|id| id.parse().unwrap()).collect();
        assert!(!attached.is_empty() && !attached.contains(&std::process::id()), "the tool shares this process's console: {text}");
    }

    #[test]
    fn other_owners_readers_and_missing_lists_are_refused() {
        let other = "S-1-5-21-1-2-3-1002";
        assert!(foreign_access(&format!("O:{other}D:(A;;FA;;;{ME})"), ME).unwrap().contains("owned by another account"));
        // Users (BU) and Authenticated Users (AU), as a folder at a drive root passes on.
        assert!(foreign_access(&format!("O:{ME}D:AI(A;ID;FA;;;{ME})(A;ID;0x1301bf;;;AU)"), ME).unwrap().contains("AU"));
        assert!(foreign_access(&format!("O:{ME}D:(A;;FR;;;BU)(A;;FA;;;{ME})"), ME).unwrap().contains("BU"));
        assert!(foreign_access(&format!("O:{ME}D:(A;;FA;;;{other})"), ME).is_some());
        assert!(foreign_access(&format!("O:{ME}G:SY"), ME).unwrap().contains("no access list"));
        assert!(foreign_access(&format!("O:{ME}D:NO_ACCESS_CONTROL"), ME).is_some());
        assert!(foreign_access("D:(A;;FA;;;SY)", ME).unwrap().contains("owner"));
    }

    /// An inherit-only entry doesn't apply to a file, but does to what's created in a folder.
    #[test]
    fn an_inherit_only_entry_counts_for_a_folder_only() {
        let sddl = format!("O:{ME}D:(A;;FA;;;{ME})(A;IO;FR;;;BU)");
        assert_eq!(foreign_access(&sddl, ME), None, "{sddl}");
        assert!(foreign_folder_access(&sddl, ME).is_some_and(|p| p.contains("BU")), "{sddl}");
        assert_eq!(foreign_folder_access(&format!("O:{ME}D:(A;;FA;;;{ME})(A;IO;FA;;;SY)"), ME), None);
    }

    /// A conditional entry can hold a `:` in a string; the access list doesn't end there, so the
    /// entry after it is still checked.
    #[test]
    fn a_colon_inside_an_entry_does_not_end_the_access_list() {
        let sddl = format!(r#"O:{ME}D:(XA;;FA;;;{ME};(@User.dept == "a:b"))(A;;FR;;;BU)"#);
        assert!(foreign_access(&sddl, ME).is_some_and(|p| p.contains("BU")), "{sddl}");
    }

    /// A conditional entry for the user is the user's; one that grants access but whose account
    /// can't be read is refused rather than skipped.
    #[test]
    fn conditional_and_malformed_entries() {
        assert_eq!(foreign_access(&format!(r#"O:{ME}D:(XA;;FA;;;{ME};(@User.dept == "x"))"#), ME), None);
        assert!(foreign_access(&format!("O:{ME}D:(A;;FA;;;{ME})(A;;FR)"), ME).is_some_and(|p| p.contains("can't be read")));
        assert!(foreign_access(&format!("O:{ME}D:(A;;FA;;;)"), ME).is_some_and(|p| p.contains("can't be read")));
        // A conditional entry grants access like a plain one: another account on it is refused.
        let other = format!(r#"O:{ME}D:(A;;FA;;;{ME})(XA;;FR;;;BU;(@User.dept == "x"))"#);
        assert!(foreign_access(&other, ME).is_some_and(|p| p.contains("BU")), "{other}");
    }

    #[test]
    fn the_sid_is_read_from_whoamis_output_and_nothing_else() {
        assert_eq!(sid_from_whoami("\"desk\\ada\",\"S-1-5-21-1-2-3-1001\"\r\n").as_deref(), Some("S-1-5-21-1-2-3-1001"));
        for bad in [
            "",
            "ERROR: Access is denied.",
            "\"desk\\ada\",\"\"",
            "\"desk\\ada\",\"S-1-\"",
            "\"S-1-5-x\"",
            "\"desk\\ada\"",
            "\"desk\\ada\",\"1-5-21-7\"",
        ] {
            assert_eq!(sid_from_whoami(bad), None, "{bad:?}");
        }
    }

    /// The real tools: a file created in a folder that lets Users read what's in it inherits that
    /// access; after `restrict_to_owner` only the current user and SYSTEM are left.
    #[cfg(windows)]
    #[test]
    fn a_file_in_a_shared_folder_is_restricted_to_its_owner() {
        let dir = std::env::temp_dir().join(format!("craft-owner-only-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = std::process::Command::new("icacls").arg(&dir).arg("/grant").arg("*S-1-5-32-545:(OI)(CI)R").output().unwrap();
        assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
        let file = dir.join("ctl.json");
        std::fs::write(&file, b"{}").unwrap();
        let me = current_user_sid().unwrap();
        // Users inherit read access, so the file is refused. (Run elevated, the file is also owned
        // by Administrators, which is the reason given first.)
        let before = sddl_of(&file).unwrap();
        assert!(before.contains(";BU)") || before.contains(";S-1-5-32-545)"), "Users inherit read access: {before}");
        assert!(foreign_access(&before, &me).is_some(), "{before}");
        restrict_to_owner(&file, &user()).unwrap();
        let sddl = sddl_of(&file).unwrap();
        assert_eq!(section(&sddl, "O:"), Some(me.as_str()), "{sddl}");
        assert_eq!(foreign_access(&sddl, &me), None, "{sddl}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file created in a private folder inside a folder that lets Users read what's in it is the
    /// user's alone from the moment it exists, before anything restricts it.
    #[cfg(windows)]
    #[test]
    fn a_file_created_in_a_private_folder_is_private_from_the_start() {
        let dir = std::env::temp_dir().join(format!("craft-owner-only-private-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ok = std::process::Command::new("icacls").arg(&dir).arg("/grant").arg("*S-1-5-32-545:(OI)(CI)R").output().unwrap();
        assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
        let private = dir.join("staging");
        create_private_dir(&private, &user()).unwrap();
        let me = current_user_sid().unwrap();
        let folder = sddl_of(&private).unwrap();
        assert_eq!((section(&folder, "O:"), foreign_access(&folder, &me)), (Some(me.as_str()), None), "{folder}");
        let file = private.join("ctl.json");
        std::fs::OpenOptions::new().write(true).create_new(true).open(&file).unwrap();
        let sddl = sddl_of(&file).unwrap();
        assert!(!sddl.contains(";BU)") && !sddl.contains(";S-1-5-32-545)"), "Users must not inherit read access: {sddl}");
        assert_eq!(foreign_access(&sddl, &me), None, "{sddl}");
        // The name is taken: the folder isn't reused.
        assert!(create_private_dir(&private, &user()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A folder's inherit-only entries are read, and the folder check counts them: every file
    /// created in the folder would get them.
    #[cfg(windows)]
    #[test]
    fn a_folders_inherit_only_entries_are_read_and_refused() {
        let dir = std::env::temp_dir().join(format!("craft-owner-only-inherit-only-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ok = std::process::Command::new("icacls").arg(&dir).arg("/grant").arg("*S-1-5-32-545:(OI)(CI)(IO)R").output().unwrap();
        assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
        let sddl = sddl_of(&dir).unwrap();
        let users = sddl
            .split('(')
            .filter_map(|ace| ace.split(')').next())
            .find(|ace| ace.split(';').nth(1).is_some_and(|flags| flags.contains("IO")) && ace.ends_with(";S-1-5-32-545"));
        let users = users.unwrap_or_else(|| panic!("the inherit-only entry for Users is read: {sddl}"));
        // The folder also inherits the temp folder's entries (Administrators on a CI runner), which the check reports
        // first, so it is asked about the Users entry alone.
        let me = current_user_sid().unwrap();
        let only_users = format!("O:{}D:({users})", section(&sddl, "O:").unwrap());
        assert!(foreign_folder_access(&only_users, &me).is_some_and(|p| p.contains("S-1-5-32-545")), "{sddl}");
        assert_eq!(foreign_access(&only_users, &me), None, "a file's check skips the inherit-only entry: {sddl}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `create_private_dir` gives the folder its owner and list in the call that creates it, so in
    /// a folder that lets Users read what's created in it, no step leaves the new folder with that
    /// entry: the folder is moved to its name only after it was created with its own list. An
    /// existing name and a missing parent are refused.
    #[cfg(windows)]
    #[test]
    fn a_private_folder_has_its_list_from_the_start() {
        let dir = std::env::temp_dir().join(format!("craft-owner-only-private-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ok = std::process::Command::new("icacls").arg(&dir).arg("/grant").arg("*S-1-5-32-545:(OI)(CI)R").output().unwrap();
        assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
        let me = current_user_sid().unwrap();
        let private = dir.join("staging");
        let made = create_private_dir(&private, &user());
        assert!(made.is_ok(), "{made:?}");
        let folder = sddl_of(&private).unwrap();
        assert_eq!((section(&folder, "O:"), foreign_folder_access(&folder, &me)), (Some(me.as_str()), None), "{folder}");
        assert!(!folder.contains(";S-1-5-32-545)") && !folder.contains(";BU)"), "Users must not inherit read access: {folder}");
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["staging"], "no folder is left under its own name");
        let again = create_private_dir(&private, &user()).unwrap_err();
        assert_eq!(again.kind(), std::io::ErrorKind::AlreadyExists, "{again}");
        let orphan = create_private_dir(&dir.join("missing").join("staging"), &user()).unwrap_err();
        assert_eq!(orphan.kind(), std::io::ErrorKind::NotFound, "{orphan}");
        assert!(!dir.join("missing").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file created by `create_private_file` in a folder that lets Users read what's in it is the
    /// user's alone as soon as it exists, with nothing written to it yet; one created with contents
    /// holds them, and an existing file is refused and left as it is.
    #[cfg(windows)]
    #[test]
    fn a_private_file_is_private_from_the_start() {
        let dir = std::env::temp_dir().join(format!("craft-owner-only-private-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ok = std::process::Command::new("icacls").arg(&dir).arg("/grant").arg("*S-1-5-32-545:(OI)(CI)R").output().unwrap();
        assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
        let me = current_user_sid().unwrap();
        let empty = dir.join("empty.token");
        create_private_file(&empty, &user(), b"").unwrap();
        let sddl = sddl_of(&empty).unwrap();
        assert!(!sddl.contains(";BU)") && !sddl.contains(";S-1-5-32-545)"), "Users must not inherit read access: {sddl}");
        assert_eq!((section(&sddl, "O:"), foreign_access(&sddl, &me)), (Some(me.as_str()), None), "{sddl}");
        assert_eq!(std::fs::read(&empty).unwrap(), b"");
        let file = dir.join("ctl.token");
        create_private_file(&file, &user(), b"secret\n").unwrap();
        let sddl = sddl_of(&file).unwrap();
        assert_eq!((section(&sddl, "O:"), foreign_access(&sddl, &me)), (Some(me.as_str()), None), "{sddl}");
        assert_eq!(std::fs::read(&file).unwrap(), b"secret\n");
        let again = create_private_file(&file, &user(), b"other\n").unwrap_err();
        assert_eq!(again.kind(), std::io::ErrorKind::AlreadyExists, "{again}");
        assert_eq!(std::fs::read(&file).unwrap(), b"secret\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A command that writes `ready` to its standard output and then reads its standard input until
    /// it closes.
    fn waiting_reader() -> std::process::Command {
        #[cfg(windows)]
        let command = {
            let mut powershell = system_tool(POWERSHELL);
            powershell.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::Out.Write('ready'); [Console]::Out.Flush(); $null = [Console]::In.ReadToEnd()",
            ]);
            powershell
        };
        #[cfg(not(windows))]
        let command = {
            let mut sh = std::process::Command::new("sh");
            sh.args(["-c", "printf ready; cat >/dev/null"]);
            sh
        };
        command
    }

    /// A tool still running when the time is up is ended and exits. The time is up once the tool
    /// has said it is waiting (a handshake, not a clock).
    /// The same tool, given its input, exits by itself and nothing ends it.
    #[test]
    fn a_tool_that_does_not_finish_in_time_is_ended() {
        let (keep, kept) = std::sync::mpsc::channel();
        // The standard input stays open, so the tool waits until it is ended.
        let hold = move |stdin: std::process::ChildStdin| keep.send(stdin).map_err(std::io::Error::other);
        match run_until(waiting_reader(), hold, |out| out == b"ready").unwrap() {
            Ran::Ended { exited } => assert!(exited, "the tool was ended but is still running"),
            Ran::Exited(out, _) => panic!("the tool exited by itself: {out:?}"),
        }
        drop(kept);
        let close = |stdin: std::process::ChildStdin| {
            drop(stdin);
            Ok(())
        };
        match run_until(waiting_reader(), close, |_| false).unwrap() {
            Ran::Exited(out, sent) => {
                assert!(out.status.success(), "{out:?}");
                assert_eq!(out.stdout, b"ready");
                sent.unwrap();
            }
            Ran::Ended { .. } => panic!("the tool was ended"),
        }
    }

    /// PowerShell that has created the file but not written it when the time is up is ended, the
    /// call fails as timed out, and the file it created is removed; nothing took the name. A file
    /// that was at the name before is another's, and it is left as it is whenever the time runs out.
    #[cfg(windows)]
    #[test]
    fn a_private_file_whose_create_does_not_finish_in_time_is_removed() {
        let dir = std::env::temp_dir().join(format!("craft-owner-only-limit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ctl.token");
        let entries = || std::fs::read_dir(&dir).unwrap().count();
        let (keep, kept) = std::sync::mpsc::channel();
        let hold = move |stdin: std::process::ChildStdin| keep.send(stdin).map_err(std::io::Error::other);
        // The time is up once the script has created its file and waits for the contents.
        let late = create_private_file_until(&file, &user(), hold, |_| entries() > 0).unwrap_err();
        drop(kept);
        assert_eq!(late.kind(), std::io::ErrorKind::TimedOut, "{late}");
        assert_eq!(entries(), 0, "the file PowerShell created is removed");
        std::fs::write(&file, b"other\n").unwrap();
        let (keep, kept) = std::sync::mpsc::channel();
        let hold = move |stdin: std::process::ChildStdin| keep.send(stdin).map_err(std::io::Error::other);
        let refused = create_private_file_until(&file, &user(), hold, |_| true).unwrap_err();
        drop(kept);
        assert!(matches!(refused.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::AlreadyExists), "{refused}");
        assert_eq!(std::fs::read(&file).unwrap(), b"other\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A folder may give its user Modify but not Full control, so a file created there doesn't let
    /// its owner change the owner (no `WRITE_OWNER`). An owner can always replace the access list,
    /// though, and the user's Full control on it then allows setting the owner.
    #[cfg(windows)]
    #[test]
    fn a_file_in_a_folder_granting_only_modify_is_still_restricted() {
        let dir = std::env::temp_dir().join(format!("craft-owner-only-modify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let me = current_user_sid().unwrap();
        let modify = std::process::Command::new("icacls")
            .arg(&dir)
            .args(["/inheritance:r", "/grant:r"])
            .arg(format!("*{me}:(OI)(CI)M"))
            .arg("*S-1-5-18:(OI)(CI)F")
            .output()
            .unwrap();
        assert!(modify.status.success(), "{}", String::from_utf8_lossy(&modify.stderr));
        let file = dir.join("ctl.json");
        std::fs::write(&file, b"{}").unwrap();
        restrict_to_owner(&file, &user()).unwrap();
        let sddl = sddl_of(&file).unwrap();
        assert_eq!((section(&sddl, "O:"), foreign_access(&sddl, &me)), (Some(me.as_str()), None), "{sddl}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A null access list (`NO_ACCESS_CONTROL`) lets every account use the file, so it is refused.
    #[cfg(windows)]
    #[test]
    fn a_file_with_no_access_list_is_refused() {
        let dir = std::env::temp_dir().join(format!("craft-owner-only-null-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ctl.json");
        std::fs::write(&file, b"{}").unwrap();
        restrict_to_owner(&file, &user()).unwrap();
        // A null access list lets everyone in; the user, as owner, may set one.
        let null = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$s = New-Object Security.AccessControl.FileSecurity; $s.SetSecurityDescriptorSddlForm('D:NO_ACCESS_CONTROL', 'Access'); [IO.File]::SetAccessControl($env:CRAFT_NULL_PATH, $s)",
            ])
            .env("CRAFT_NULL_PATH", &file)
            .output()
            .unwrap();
        assert!(null.status.success(), "{}", String::from_utf8_lossy(&null.stderr));
        let (sddl, _) = read_with_sddl(&file, 64).unwrap();
        let me = current_user_sid().unwrap();
        assert!(foreign_access(&sddl, &me).is_some_and(|p| p.contains("no access list")), "{sddl}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An `OWNER RIGHTS` entry replaces what an owner may do implicitly. Granting only *write
    /// owner* that way, the access list can't be replaced although the owner can still be set: the
    /// file isn't taken as restricted. Run elevated, it can be replaced, and then must be restricted.
    #[cfg(windows)]
    #[test]
    fn a_file_whose_access_list_cant_be_replaced_is_not_restricted() {
        let dir = std::env::temp_dir().join(format!("craft-owner-only-ow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let me = current_user_sid().unwrap();
        let file = dir.join("ctl.json");
        std::fs::write(&file, b"{}").unwrap();
        // The whole list is written, as `restrict_to_owner` does: `icacls /inheritance:r` leaves
        // the Administrators entry the windows-latest runner gives a new file, and with it an
        // elevated administrator could replace the list after all.
        let rights = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$s = New-Object Security.AccessControl.FileSecurity; $s.SetSecurityDescriptorSddlForm('D:P(A;;FR;;;' + $env:CRAFT_OW_USER + ')(A;;WO;;;OW)', 'Access'); [IO.File]::SetAccessControl($env:CRAFT_OW_PATH, $s)",
            ])
            .env("CRAFT_OW_PATH", &file)
            .env("CRAFT_OW_USER", &me)
            .output()
            .unwrap();
        assert!(rights.status.success(), "{}", String::from_utf8_lossy(&rights.stderr));
        // Nothing but the two entries: no Administrators entry for the elevated runner to use.
        let sddl = sddl_of(&file).unwrap();
        assert_eq!(section(&sddl, "D:"), Some(format!("(A;;524288;;;S-1-3-4)(A;;1179785;;;{me})").as_str()), "{sddl}");
        // An elevated administrator (the windows-latest runner) may replace the list anyway; then the
        // file must really be the user's alone, since the result is checked before it is trusted.
        if restrict_to_owner(&file, &user()).is_ok() {
            let after = sddl_of(&file).unwrap();
            assert!(elevated(), "the access list could not be replaced: {after}");
            assert_eq!(foreign_access(&after, &me), None, "{after}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Whether the tests run elevated: `whoami /groups` lists the high (or system) mandatory label.
    #[cfg(windows)]
    fn elevated() -> bool {
        let out = system_tool(WHOAMI).args(["/groups", "/fo", "csv", "/nh"]).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        text.contains("\"S-1-16-12288\"") || text.contains("\"S-1-16-16384\"")
    }

    /// When the tools fail, the functions say so: nothing is taken as restricted or as checked.
    #[cfg(windows)]
    #[test]
    fn tool_failures_are_errors() {
        let missing = std::env::temp_dir().join(format!("craft-owner-only-missing-{}", std::process::id())).join("none.json");
        assert!(restrict_to_owner(&missing, &user()).is_err(), "icacls on a missing file");
        assert!(sddl_of(&missing).is_err(), "Get-Acl on a missing file");
        let me = current_user_sid().unwrap();
        assert!(me.starts_with("S-1-5-") && me.split('-').count() >= 4, "{me}");
        assert!(!me.contains('"') && !me.contains('\\'), "the SID alone, not the account name: {me}");
    }

    #[test]
    fn the_descriptor_and_bytes_are_read_from_the_scripts_output_and_nothing_else() {
        let sddl = format!("O:{ME}D:P(A;;FA;;;{ME})");
        assert_eq!(sddl_and_bytes(&format!("{sddl}\n7B-0A-ff\r\n")), Some((sddl.clone(), vec![0x7B, 0x0A, 0xFF])));
        assert_eq!(sddl_and_bytes(&format!("{sddl}\n")), Some((sddl.clone(), Vec::new())));
        for bad in [
            String::new(),
            sddl.clone(),
            "D:P(A;;FA;;;SY)\n7B".into(),
            format!("{sddl}\n7B-2"),
            format!("{sddl}\n7B--22"),
            format!("{sddl}\n7G"),
            format!("{sddl}\n+7-22"),
        ] {
            assert_eq!(sddl_and_bytes(&bad), None, "{bad:?}");
        }
    }

    /// The bytes come with the descriptor of the same handle, capped at `max + 1` so an oversized
    /// file shows, and the restricted file is owned by the user.
    #[cfg(windows)]
    #[test]
    fn the_file_is_read_with_its_descriptor_through_one_handle() {
        let dir = std::env::temp_dir().join(format!("craft-owner-only-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ctl.json");
        std::fs::write(&file, br#"{"port":1}"#).unwrap();
        restrict_to_owner(&file, &user()).unwrap();
        let me = current_user_sid().unwrap();
        let (sddl, bytes) = read_with_sddl(&file, 64).unwrap();
        assert_eq!(section(&sddl, "O:"), Some(me.as_str()), "{sddl}");
        assert_eq!((foreign_access(&sddl, &me), bytes.as_slice()), (None, &br#"{"port":1}"#[..]));
        assert_eq!(read_with_sddl(&file, 3).unwrap().1, br#"{"po"#);
        // A limit past what the script's `[int]` holds is capped, not handed to it.
        assert_eq!(read_with_sddl(&file, u64::MAX).unwrap().1, br#"{"port":1}"#);
        assert!(read_with_sddl(&dir.join("none.json"), 64).is_err());
        // A deny entry is read as one: denying Guests takes nothing from the user.
        let guests = std::process::Command::new("icacls").arg(&file).arg("/deny").arg("*S-1-5-32-546:(R)").output().unwrap();
        assert!(guests.status.success(), "{}", String::from_utf8_lossy(&guests.stderr));
        let (sddl, _) = read_with_sddl(&file, 64).unwrap();
        assert!(sddl.contains(";;;S-1-5-32-546)") && sddl.contains("(D;"), "{sddl}");
        assert_eq!(foreign_access(&sddl, &me), None, "{sddl}");
        // The handle shares the file for reading only: a file someone has open for writing (or
        // could rename or delete) isn't read, and so can't change between the check and the read.
        let writer = std::fs::OpenOptions::new().write(true).open(&file).unwrap();
        assert!(read_with_sddl(&file, 64).is_err(), "a file open for writing is not read");
        // Restricting reads the descriptor alone, so it works on the file an app holds open to
        // write the token through.
        restrict_to_owner(&file, &user()).unwrap();
        assert_eq!(foreign_access(&sddl_of(&file).unwrap(), &me), None);
        drop(writer);
        assert!(read_with_sddl(&file, 64).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Entries of the file's own, not inherited, are replaced too: an Administrators or Users
    /// entry (as the windows-latest runner's temp folder leaves on a new file) and a deny that
    /// would keep the user from setting the owner.
    #[cfg(windows)]
    #[test]
    fn entries_of_the_files_own_are_replaced() {
        let dir = std::env::temp_dir().join(format!("craft-owner-only-own-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let me = current_user_sid().unwrap();
        let mine = format!("*{me}:(R)");
        for (name, entries) in [
            ("others.json", ["/grant", "*S-1-5-32-544:(F)", "/grant", "*S-1-5-32-545:(R)"]),
            ("deny.json", ["/deny", "*S-1-1-0:(WO)", "/grant", "*S-1-5-32-544:(F)"]),
        ] {
            let file = dir.join(name);
            std::fs::write(&file, b"{}").unwrap();
            // The user keeps read access, so the descriptor can be read before restricting.
            let own = std::process::Command::new("icacls").arg(&file).arg("/inheritance:r").args(entries).args(["/grant", &mine]).output().unwrap();
            assert!(own.status.success(), "{}", String::from_utf8_lossy(&own.stderr));
            assert!(foreign_access(&sddl_of(&file).unwrap(), &me).is_some(), "{name}");
            restrict_to_owner(&file, &user()).unwrap();
            let sddl = sddl_of(&file).unwrap();
            assert_eq!((section(&sddl, "O:"), foreign_access(&sddl, &me)), (Some(me.as_str()), None), "{name}: {sddl}");
            assert!(!sddl.contains("(D;"), "{name}: the deny is gone: {sddl}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_descriptor_still_open_to_others_after_restricting_is_an_error() {
        assert!(still_foreign(&format!("O:{ME}D:P(A;;FA;;;SY)(A;;FA;;;{ME})"), ME).is_ok());
        for bad in [format!("O:{ME}D:P(A;;FA;;;S-1-5-32-544)(A;;FA;;;{ME})"), format!("O:S-1-5-32-544D:P(A;;FA;;;{ME})"), format!("O:{ME}")] {
            assert!(still_foreign(&bad, ME).is_err_and(|e| e.to_string().contains("could not be restricted")), "{bad}");
        }
    }

    /// [`read_with_sddl`] gives accounts as full SIDs (rights as a number, no list flags), the way a
    /// CI runner's built-in Administrator reads its own restricted file. An alias such as `LA`
    /// is never taken for the user, whatever the user's RID: on a domain-joined machine a domain
    /// account can share the local Administrator's RID 500.
    #[test]
    fn descriptors_with_full_sids_and_aliases_that_are_never_the_user() {
        let admin = "S-1-5-21-1-2-3-500";
        let full = format!("O:{admin}D:(A;;2032127;;;S-1-5-18)(A;;2032127;;;{admin})");
        assert_eq!(foreign_access(&full, admin), None);
        // Owner rights by SID, a deny by SID, an inherit-only entry for Creator Owner.
        let more = format!("O:{ME}D:(D;;1179785;;;S-1-5-32-546)(A;IO;268435456;;;S-1-3-0)(A;;2032127;;;S-1-3-4)(A;;2032127;;;{ME})");
        assert_eq!(foreign_access(&more, ME), None);
        assert!(foreign_access(&format!("O:{ME}D:(A;;1179785;;;S-1-5-32-545)(A;;2032127;;;{ME})"), ME).is_some_and(|p| p.contains("S-1-5-32-545")));
        for me in [admin, "S-1-5-21-1-2-3-501"] {
            assert!(foreign_access("O:LAD:PAI(A;;FA;;;SY)(A;;FA;;;LA)", me).is_some_and(|p| p.contains("owned by another account (LA)")), "{me}");
            assert!(foreign_access("O:LGD:P(A;;FA;;;LG)", me).is_some(), "{me}");
        }
        assert!(foreign_access(&format!("O:{admin}D:P(A;;FA;;;{admin})(A;;FR;;;LA)"), admin).is_some_and(|p| p.contains("LA")));
    }

    /// A refusal names a well-known account as well as giving its SID, so `S-1-5-32-544` reads as
    /// Administrators without a lookup; any other SID is given alone.
    #[test]
    fn refusals_name_well_known_accounts_and_keep_their_sid() {
        for (sid, name) in [
            ("S-1-5-32-544", "Administrators"),
            ("S-1-5-32-545", "Users"),
            ("S-1-5-11", "Authenticated Users"),
            ("S-1-5-4", "INTERACTIVE"),
            ("S-1-1-0", "Everyone"),
            ("S-1-5-6", "SERVICE"),
            ("S-1-5-3", "BATCH"),
        ] {
            let sddl = format!("O:{ME}D:PAI(A;;FA;;;{ME})(A;ID;1179817;;;{sid})");
            assert_eq!(foreign_access(&sddl, ME), Some(format!("another account, {name} ({sid}), has access to it")), "{sddl}");
        }
        // SYSTEM may have access, but a file it owns isn't the user's.
        assert_eq!(foreign_access(&format!("O:S-1-5-18D:P(A;;FA;;;{ME})"), ME).as_deref(), Some("it is owned by another account, SYSTEM (S-1-5-18)"));
        assert_eq!(
            foreign_access(&format!("O:S-1-5-32-544D:P(A;;FA;;;{ME})"), ME).as_deref(),
            Some("it is owned by another account, Administrators (S-1-5-32-544)")
        );
        // Another user, a lookalike and an alias have no fixed name: the account is given as it was.
        for sid in ["S-1-5-21-1-2-3-1002", "S-1-5-32-5440", "S-1-5-1", "BA"] {
            let sddl = format!("O:{ME}D:(A;;FA;;;{ME})(A;;FR;;;{sid})");
            assert_eq!(foreign_access(&sddl, ME), Some(format!("another account ({sid}) has access to it")), "{sddl}");
        }
        assert_eq!(
            foreign_access("O:S-1-5-21-1-2-3-1002D:(A;;FA;;;SY)", ME).as_deref(),
            Some("it is owned by another account (S-1-5-21-1-2-3-1002)")
        );
    }

    #[test]
    fn sections_stop_at_the_next_tag() {
        assert_eq!(section("O:S-1-5-18G:S-1-5-32-544D:(A;;FA;;;SY)", "O:"), Some("S-1-5-18"));
        assert_eq!(section("O:BAG:SYD:P(A;;FA;;;SY)S:(AU;SA;FA;;;WD)", "D:"), Some("P(A;;FA;;;SY)"));
        assert_eq!(section("O:BA", "D:"), None);
    }
}
