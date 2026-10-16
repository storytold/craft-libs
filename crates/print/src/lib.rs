//! The system print spooler. On macOS and Linux this is CUPS: printers come from `lpstat` — its
//! queues, and the driverless destinations it can print to without one — and jobs are piped to
//! `lp` with the job options (copies, collation, duplex, colour). When those commands are
//! missing (a sandboxed install, storytold/photocraft#6081) or answer nothing, the server is
//! asked directly over IPP ([`ipp`]) and jobs go to it as Print-Job; when no CUPS server is
//! reachable, the job is handed to the desktop's print portal ([`portal`]), whose own dialog
//! restores the printer selection (storytold/pdfcraft#1905). On Windows the printers come
//! from the spooler and jobs are drawn by the in-box `Windows.Data.Pdf` into the XPS print API
//! with the options as a PrintTicket, all in this process (`windows.rs`): no PowerShell or other
//! program is started, so printing works where scripts are locked down. Other platforms report
//! that printing isn't available yet; the print-ready PDF can still be saved.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// IPP/1.1 straight to the local CUPS server (discovery and Print-Job): the transport the print
/// path falls back to when `lpstat`/`lp` are missing (a sandboxed install), exposed for callers
/// that need the server directly.
#[cfg(all(unix, not(target_arch = "wasm32")))]
pub mod ipp;

/// The XDG desktop portal's print dialog: the print path when no CUPS server is reachable.
#[cfg(all(unix, not(target_arch = "wasm32")))]
pub mod portal;

#[cfg(any(test, windows))]
mod progress;
#[cfg(any(test, windows))]
mod sheets;
#[cfg(test)]
mod tests;
#[cfg(any(test, windows))]
mod ticket;
#[cfg(windows)]
mod windows;

/// Native Windows driver settings, printable geometry and explicit RGB/GDI transport.
/// Callers perform ICC conversion and compose page pixels/vector strokes. Other platforms
/// return `rgb::PrintError::Unsupported`; the PDF spooler remains independent of this API.
pub mod rgb {
    pub use craft_winprint_gdi::*;
}

/// A system spooler failure. Document permissions belong to the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Spool(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spool(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    pub default: bool,
    /// Whether the Windows driver reports duplex support; None when unknown (CUPS).
    pub two_sided: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Duplex {
    #[default]
    Off,
    LongEdge,
    ShortEdge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Scaling {
    /// Preserve the PDF's physical sheet size, even when hardware margins clip it.
    #[default]
    Exact,
    /// Use matching driver paper or its default and shrink oversized sheets to fit.
    /// Smaller sheets retain their size; the printable area centers the result.
    /// On Windows every sheet of the job goes on that one paper.
    ShrinkToFit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    /// `None` = the system default printer.
    pub printer: Option<String>,
    pub copies: u32,
    pub collate: bool,
    /// `None` preserves the printer's duplex default; `Some(Off)` forces simplex.
    pub duplex: Option<Duplex>,
    pub scaling: Scaling,
    pub grayscale: bool,
    pub title: String,
    /// The printer driver's own job options (a PPD keyword and the chosen value), such as the
    /// paper tray or paper type; see [`printer_options`]. CUPS only.
    pub options: Vec<(String, String)>,
}

impl Default for Job {
    fn default() -> Self {
        Job {
            printer: None,
            copies: 1,
            collate: true,
            duplex: None,
            scaling: Scaling::Exact,
            grayscale: false,
            title: "Craft".into(),
            options: Vec::new(),
        }
    }
}

/// The `lpstat -d` line's destination name, when the spooler has a default.
fn default_destination(out: &str) -> Option<&str> {
    out.lines().find_map(|l| l.strip_prefix("system default destination:")).map(str::trim)
}

/// Parse `lpstat -p -d` output.
pub fn parse_lpstat(out: &str) -> Vec<Printer> {
    let default = default_destination(out);
    out.lines()
        .filter_map(|l| l.strip_prefix("printer "))
        .filter_map(|l| l.split_whitespace().next())
        .map(|n| Printer { name: n.to_string(), default: default == Some(n), two_sided: None })
        .collect()
}

/// Parse `lpstat -e` output: destination names, one per line.
pub fn parse_lpstat_e(out: &str) -> Vec<String> {
    out.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

/// The `lp` arguments for a job. There is no file argument: `lp` reads the job from stdin.
pub fn lp_args(job: &Job) -> Vec<String> {
    let mut a = Vec::new();
    if let Some(p) = &job.printer {
        a.extend(["-d".to_string(), p.clone()]);
    }
    a.extend(["-n".to_string(), job.copies.clamp(1, 999).to_string()]);
    a.extend(["-t".to_string(), job.title.clone()]);
    let mut opt = |o: &str| a.extend(["-o".to_string(), o.to_string()]);
    opt(if job.collate { "collate=true" } else { "collate=false" });
    if let Some(duplex) = job.duplex {
        opt(match duplex {
            Duplex::Off => "sides=one-sided",
            Duplex::LongEdge => "sides=two-sided-long-edge",
            Duplex::ShortEdge => "sides=two-sided-short-edge",
        });
    }
    if job.grayscale {
        opt("print-color-mode=monochrome");
    }
    opt(match job.scaling {
        Scaling::Exact => "fit-to-page=false",
        // PWG 5100.13 §6.2.5: auto-fit fits oversized pages and leaves smaller pages unscaled.
        Scaling::ShrinkToFit => "print-scaling=auto-fit",
    });
    for (key, value) in &job.options {
        // Driver options are PPD keywords; anything else is not something lp can be given.
        if is_ppd_keyword(key) && is_ppd_keyword(value) && !HANDLED_OPTIONS.contains(&key.as_str()) {
            a.extend(["-o".to_string(), format!("{key}={value}")]);
        }
    }
    a
}

/// One of the printer driver's job options (a PPD `OpenUI` entry), such as the paper tray.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrinterOption {
    /// The PPD keyword (`InputSlot`).
    pub key: String,
    /// What the driver calls it (`Paper tray`); the keyword when it gives no name.
    pub label: String,
    /// The driver's group (`Paper Handling`); empty outside a group.
    pub group: String,
    /// The choices: keyword and label (`Tray2`, `Tray 2`).
    pub choices: Vec<(String, String)>,
    /// The choice the printer uses when a job doesn't say.
    pub default: String,
}

/// Options Craft's Print dialog sets itself (paper, two-sided, collation), so the driver's
/// copies of them are neither shown nor sent.
pub const HANDLED_OPTIONS: &[&str] = &["PageSize", "PageRegion", "Duplex", "Collate", "ImageableArea", "PaperDimension"];

/// Caps on what a PPD may make the dialog hold (PPDs are text files from the printer vendor).
const MAX_OPTIONS: usize = 400;
const MAX_CHOICES: usize = 400;

/// A PPD keyword: printable ASCII without spaces, `/`, `:` or `=`.
fn is_ppd_keyword(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_graphic() && !matches!(b, b'/' | b':' | b'=' | b'"'))
}

/// `Name/Label` → (`Name`, `Label`), the label falling back to the name.
fn name_label(s: &str) -> (&str, &str) {
    match s.split_once('/') {
        Some((n, l)) if !l.trim().is_empty() => (n.trim(), l.trim()),
        Some((n, _)) => (n.trim(), n.trim()),
        None => (s.trim(), s.trim()),
    }
}

/// The job options a PPD offers, in file order, without the installable-hardware group and the
/// options Craft sets itself ([`HANDLED_OPTIONS`]). Lenient: malformed lines are skipped.
pub fn parse_ppd(ppd: &str) -> Vec<PrinterOption> {
    let mut out: Vec<PrinterOption> = Vec::new();
    let (mut group, mut group_key) = (String::new(), String::new());
    let mut open: Option<PrinterOption> = None;
    for line in ppd.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("*OpenGroup:") {
            let (k, l) = name_label(rest);
            (group_key, group) = (k.to_string(), l.to_string());
        } else if line.starts_with("*CloseGroup") {
            (group_key, group) = (String::new(), String::new());
        } else if let Some(rest) = line.strip_prefix("*OpenUI") {
            let Some((head, _kind)) = rest.trim().rsplit_once(':') else { continue };
            let (key, label) = name_label(head.trim().trim_start_matches('*'));
            open = is_ppd_keyword(key).then(|| PrinterOption {
                key: key.to_string(),
                label: label.to_string(),
                group: group.clone(),
                choices: Vec::new(),
                default: String::new(),
            });
        } else if line.starts_with("*CloseUI") {
            if let Some(o) = open.take()
                && !o.choices.is_empty()
                && group_key != "InstallableOptions"
                && !HANDLED_OPTIONS.contains(&o.key.as_str())
                && out.len() < MAX_OPTIONS
            {
                out.push(o);
            }
        } else if let Some(o) = open.as_mut() {
            let Some(rest) = line.strip_prefix('*') else { continue };
            if let Some(value) = rest.strip_prefix("Default").and_then(|r| r.strip_prefix(o.key.as_str())).and_then(|r| r.strip_prefix(':')) {
                o.default = value.trim().to_string();
            } else if let Some(rest) = rest.strip_prefix(o.key.as_str()).and_then(|r| r.strip_prefix(' ')) {
                // `*InputSlot Tray2/Tray 2: "<< ... >>"`; the code after the colon is the driver's.
                let head = rest.split_once(':').map_or(rest, |(h, _)| h);
                let (choice, label) = name_label(head);
                if is_ppd_keyword(choice) && o.choices.len() < MAX_CHOICES && o.choices.iter().all(|(c, _)| c != choice) {
                    o.choices.push((choice.to_string(), label.to_string()));
                }
            }
        }
    }
    for o in &mut out {
        if !o.choices.iter().any(|(c, _)| *c == o.default) {
            o.default = o.choices.first().map(|(c, _)| c.clone()).unwrap_or_default();
        }
    }
    out
}

/// The current choice of each option in `lpoptions -p NAME -l` output (`Key/Label: a *b c`): the
/// queue's defaults, including the user's own (`~/.cups/lpoptions`).
pub fn parse_lpoptions(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| {
            let (head, choices) = l.split_once(':')?;
            let (key, _) = name_label(head);
            let current = choices.split_whitespace().find_map(|c| c.strip_prefix('*'))?;
            Some((key.to_string(), current.to_string()))
        })
        .collect()
}

/// A PPD's text: UTF-8, or ISO Latin-1 (the PPD default) when it isn't valid UTF-8.
pub fn ppd_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

/// The driver's job options for a printer (CUPS: its PPD, with the queue's current defaults);
/// empty when it has none or on other platforms. Reads files and runs `lpoptions`, so call it
/// when the user asks, not every frame.
pub fn printer_options(printer: &str) -> Vec<PrinterOption> {
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    {
        // Queue names never contain `/`; anything else is not a queue and not a path to follow.
        if !is_ppd_keyword(printer) {
            return Vec::new();
        }
        const PPD_CAP: u64 = 16 << 20;
        let ppd = std::fs::File::open(format!("/etc/cups/ppd/{printer}.ppd")).ok().and_then(|f| {
            use std::io::Read as _;
            let mut bytes = Vec::new();
            f.take(PPD_CAP).read_to_end(&mut bytes).ok().map(|_| bytes)
        });
        let mut options = ppd.map(|b| parse_ppd(&ppd_text(&b))).unwrap_or_default();
        let current = std::process::Command::new("lpoptions")
            .args(["-p", printer, "-l"])
            .output()
            .map(|o| parse_lpoptions(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default();
        for (key, value) in current {
            if let Some(o) = options.iter_mut().find(|o| o.key == key)
                && o.choices.iter().any(|(c, _)| *c == value)
            {
                o.default = value;
            }
        }
        options
    }
    #[cfg(not(all(unix, not(target_arch = "wasm32"))))]
    {
        let _ = printer;
        Vec::new()
    }
}

/// Whether [`open_printer_preferences`] can show the driver's own settings window (Windows).
pub const HAS_PRINTER_PREFERENCES: bool = cfg!(windows);

/// Open the printer driver's Printing Preferences window (Windows: `printui.dll`). What the user
/// sets there is saved as that printer's defaults for this Windows user, which every job starts
/// from; the Print dialog's copies, two-sided, grayscale and paper still apply on top.
pub fn open_printer_preferences(printer: &str) -> Result<(), Error> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // printui takes the name in quotes; Windows printer names can't contain one anyway.
        if printer.is_empty() || printer.contains('"') {
            return Err(Error::Spool("not a printer name".into()));
        }
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let rundll = std::path::Path::new(&root).join("System32").join("rundll32.exe");
        std::process::Command::new(rundll)
            .raw_arg("printui.dll,PrintUIEntry")
            .raw_arg("/e")
            .raw_arg("/n")
            .raw_arg(format!("\"{printer}\""))
            .spawn()
            .map(|_| ())
            .map_err(|e| Error::Spool(format!("the printer's preferences could not be opened: {e}")))
    }
    #[cfg(not(windows))]
    {
        let _ = printer;
        Err(Error::Spool("printer preferences open only on Windows; use the printer's options instead".into()))
    }
}

/// `lpstat`, forced to print untranslated messages so [`parse_lpstat`] and [`parse_lpstat_e`] can
/// read them.
///
/// `LC_ALL`/`LANG=C` is enough on Linux. macOS CUPS ignores them and follows the user's
/// interface language (`AppleLanguages`) unless `SOFTWARE` is set, in which case it uses `LANG`.
fn lpstat_with(args: &[&str]) -> std::process::Command {
    let mut c = std::process::Command::new("lpstat");
    c.args(args).env("LC_ALL", "C").env("LANG", "C").env("SOFTWARE", "Craft");
    c
}

/// `lpstat -p -d`: the spooler's queues and the default destination.
pub fn lpstat_command() -> std::process::Command {
    lpstat_with(&["-p", "-d"])
}

/// `lpstat -e`: every destination CUPS can print to, including driverless network printers no
/// permanent queue exists for (a queue is built only when a job needs one). GTK's and macOS's
/// print dialogs list them the same way. CUPS ≥ 1.7 (2013).
///
/// Gated like its only caller, `printers`' unix block: off unix this is dead code, and the
/// Windows clippy gate builds with `-D warnings`.
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn lpstat_e_command() -> std::process::Command {
    lpstat_with(&["-e"])
}

/// Whether `name` can be started: an executable `name` on `PATH`, the way a `Command` resolves
/// one. Gated like its unix callers.
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn spawnable(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| spawnable_in(&path, name))
}

/// [`spawnable`] under a given search path, so tests can stand in for the environment.
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn spawnable_in(path: &std::ffi::OsStr, name: &str) -> bool {
    std::env::split_paths(path).any(|dir| {
        std::fs::metadata(dir.join(name)).is_ok_and(|m| {
            use std::os::unix::fs::PermissionsExt;
            m.is_file() && m.permissions().mode() & 0o111 != 0
        })
    })
}

/// The printers the system knows (empty when there are none or no spooler): its queues, then the
/// driverless destinations CUPS can print to without one — from `lpstat` when it answers, else
/// over IPP ([`ipp::printers`]).
pub fn printers() -> Vec<Printer> {
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    {
        if let Some(queues) = lpstat_command().output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()) {
            // A spooler without `-e`, or one whose discovery times out, only loses the driverless names.
            let available = lpstat_e_command().output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
            let parsed = printers_parsed(&queues, available.as_deref());
            if !parsed.is_empty() {
                return parsed;
            }
        }
        // `lpstat` missing (a sandboxed install) or answering nothing (a stopped scheduler) is
        // not "no printers" when the server itself is there: ask it directly. A server that
        // cannot be reached, or whose answer cannot be parsed, leaves the list empty as before.
        if ipp::cups_reachable() {
            return ipp::printers().unwrap_or_default();
        }
        Vec::new()
    }
    #[cfg(windows)]
    {
        windows::printers()
    }
    #[cfg(not(any(windows, all(unix, not(target_arch = "wasm32")))))]
    {
        Vec::new()
    }
}

/// The Print dialog's printers from lpstat's two answers: the spooler's queues (`-p -d`), then
/// the driverless destinations of `available` (`-e`) that have no queue — CUPS builds a temporary
/// queue when `lp` sends them a job. Each destination once, queues first; the default still comes
/// from `-d`, so a driverless default keeps its marker. `None` when the spooler doesn't know `-e`
/// (CUPS < 1.7, 2013), which loses only the driverless names.
pub fn printers_parsed(queues: &str, available: Option<&str>) -> Vec<Printer> {
    let mut printers = parse_lpstat(queues);
    let default = default_destination(queues);
    if let Some(names) = available {
        for name in parse_lpstat_e(names) {
            if !printers.iter().any(|p| p.name == name) {
                printers.push(Printer { default: default == Some(name.as_str()), name, two_sided: None });
            }
        }
    }
    printers
}

/// The Print dialog's printers from the Windows spooler's queues (name, and whether the driver
/// takes two-sided settings when that is known): sorted by name, each once, the default marked
/// when it is one of them.
#[cfg(any(test, windows))]
pub(crate) fn printer_list(queues: impl IntoIterator<Item = (String, Option<bool>)>, default: Option<&str>) -> Vec<Printer> {
    let mut queues: Vec<(String, Option<bool>)> = queues.into_iter().filter(|(n, _)| !n.trim().is_empty()).collect();
    queues.sort_by(|a, b| a.0.cmp(&b.0));
    queues.dedup_by(|a, b| a.0 == b.0);
    queues.into_iter().map(|(name, two_sided)| Printer { default: default == Some(name.as_str()), name, two_sided }).collect()
}

/// A printer list in the text form craft-print's former PowerShell backend printed: one per line,
/// `*` before the system default, followed by a tab and 2/1 for duplex support. The first `*`
/// wins when a broken spooler marks more than one. [`printers`] no longer uses it; it stays for
/// callers that parse lists in this form.
pub fn parse_windows_printers(out: &str) -> Vec<Printer> {
    let mut printers: Vec<Printer> = out
        .lines()
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (l, two_sided) = match l.rsplit_once('\t') {
                Some((name, "2")) => (name, Some(true)),
                Some((name, "1")) => (name, Some(false)),
                _ => (l, None),
            };
            match l.strip_prefix('*') {
                Some(name) => Printer { name: name.to_string(), default: true, two_sided },
                None => Printer { name: l.to_string(), default: false, two_sided },
            }
        })
        .collect();
    let mut seen_default = false;
    for p in &mut printers {
        if p.default {
            p.default = !seen_default;
            seen_default = true;
        }
    }
    printers
}

/// Send a print-ready PDF to the spooler. Returns the spooler's message: CUPS includes a job id;
/// Windows confirms submission only ("spooled 3 sheet(s)").
pub fn submit(pdf: &[u8], job: &Job) -> Result<String, Error> {
    submit_with_progress(pdf, job, &mut |_, _| {})
}

/// The message when no transport can take the job at all: no spooler command, no CUPS server,
/// no print portal.
const PRINTING_UNAVAILABLE: &str = "printing to a printer isn't available on this platform yet; save the print-ready PDF instead";

/// [`submit`] with Windows sheet-processing progress: `(processed, sheets)` before the first
/// sheet and after each sheet is successfully added to the native print job. Counts refer to
/// source PDF sheets, regardless of copies or duplex. CUPS reports no progress.
///
/// The callback runs synchronously on the thread calling this function, need not be `Send`,
/// and should return promptly. It is never retained. All sheets processed is not terminal
/// success: only the returned result confirms submission, and a later failure may follow the
/// last progress event. This does not report physical printing or provide cancellation.
pub fn submit_with_progress(pdf: &[u8], job: &Job, progress: &mut dyn FnMut(usize, usize)) -> Result<String, Error> {
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    {
        let _ = progress;
        if spawnable("lp") {
            return submit_via(std::process::Command::new("lp"), pdf, job);
        }
        // No `lp`: the server itself takes the job as Print-Job (the driver's PPD options are
        // not sent on this path), else the portal's dialog asks the user for a printer.
        if ipp::cups_reachable() {
            return ipp::print_job(pdf, job);
        }
        if portal::print_dialog_available() {
            return portal::print(pdf, job);
        }
        Err(Error::Spool(PRINTING_UNAVAILABLE.into()))
    }
    #[cfg(windows)]
    {
        windows::submit_with_progress(pdf, job, progress)
    }
    #[cfg(not(any(windows, all(unix, not(target_arch = "wasm32")))))]
    {
        let _ = (pdf, job, progress);
        Err(Error::Spool(PRINTING_UNAVAILABLE.into()))
    }
}

/// [`submit`] with the spooler command given, so tests can stand in for `lp`.
///
/// The job goes to `lp` on stdin, never through a file: a predictable job folder in a shared
/// temp directory lets another local user read printed documents or swap the file before `lp`
/// reads it.
#[cfg(any(test, all(unix, not(target_arch = "wasm32"))))]
pub(crate) fn submit_via(mut lp: std::process::Command, pdf: &[u8], job: &Job) -> Result<String, Error> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = lp
        .args(lp_args(job))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Spool(format!("the print spooler is not available: {e}")))?;
    let stdin = child.stdin.take();
    // Feed the job on its own thread while collecting the output, so neither side can fill a
    // pipe and wait on the other. Dropping `stdin` at the end closes it: the end of the job.
    let fed_and_out = std::thread::scope(|s| {
        let feeder = std::thread::Builder::new().name("print job".into()).spawn_scoped(s, move || match stdin {
            Some(mut stdin) => stdin.write_all(pdf),
            None => Err(std::io::Error::other("no pipe to the spooler")),
        });
        match feeder {
            Ok(feeder) => {
                let out = child.wait_with_output();
                Ok((feeder.join(), out))
            }
            Err(e) => {
                // No thread to feed it: stop `lp` rather than leave it with an empty job.
                let _ = child.kill();
                let _ = child.wait();
                Err(Error::Spool(format!("the print job could not be sent to the spooler: {e}")))
            }
        }
    });
    let (fed, out) = fed_and_out?;
    let out = out.map_err(|e| Error::Spool(format!("the print spooler is not available: {e}")))?;
    if !out.status.success() {
        // `lp` may refuse before reading the job (an unknown printer); its message says why,
        // and the broken pipe that leaves behind does not.
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(Error::Spool(if err.is_empty() { "the print job was refused".into() } else { err }));
    }
    match fed {
        Ok(Ok(())) => Ok(job_message(&out.stdout)),
        Ok(Err(e)) => Err(Error::Spool(format!("the print job could not be sent to the spooler: {e}"))),
        Err(_) => Err(Error::Spool("the print job could not be sent to the spooler".into())),
    }
}

/// `lp`'s reply ("request id is Office-12"), without the "(0 file(s))" CUPS adds when the job came
/// on stdin: it reads as if nothing was sent.
#[cfg(any(test, all(unix, not(target_arch = "wasm32"))))]
pub(crate) fn job_message(stdout: &[u8]) -> String {
    let s = String::from_utf8_lossy(stdout);
    let s = s.trim();
    s.strip_suffix("(0 file(s))").map_or(s, str::trim_end).to_string()
}

/// The job's dialog options as `key=value`: the lines both in-process previews print after the
/// transport name.
fn job_options(job: &Job) -> Vec<String> {
    vec![
        format!("printer={}", job.printer.as_deref().unwrap_or("(default)")),
        format!("copies={}", job.copies.clamp(1, 999)),
        format!("collate={}", job.collate),
        format!(
            "duplex={}",
            match job.duplex {
                None => "(queue default)",
                Some(Duplex::Off) => "one-sided",
                Some(Duplex::LongEdge) => "two-sided-long-edge",
                Some(Duplex::ShortEdge) => "two-sided-short-edge",
            }
        ),
        format!(
            "scaling={}",
            match job.scaling {
                Scaling::Exact => "exact",
                Scaling::ShrinkToFit => "shrink-to-fit",
            }
        ),
        format!("color={}", if job.grayscale { "monochrome" } else { "color" }),
    ]
}

/// [`command_preview`] for the Windows backend.
fn windows_preview(job: &Job) -> Vec<String> {
    std::iter::once("xps-print".to_string()).chain(job_options(job)).collect()
}

/// [`command_preview`] for the IPP transport: the Print-Job's attributes. The driver's PPD
/// options have no equivalent on this path, so they are not listed.
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn ipp_preview(job: &Job) -> Vec<String> {
    std::iter::once("ipp".to_string()).chain(job_options(job)).collect()
}

/// [`command_preview`] for the print portal: the printer and the options are the user's to
/// choose in the portal's own dialog.
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn portal_preview() -> Vec<String> {
    vec!["portal".into(), "printer=(dialog)".into()]
}

/// [`command_preview`] for the CUPS backend: the transport that would run — `lp` when the
/// spooler's command is there, else IPP to the reachable server, else the portal's dialog.
fn cups_preview(job: &Job) -> Result<Vec<String>, Error> {
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    {
        if spawnable("lp") {
            return Ok(std::iter::once("lp".to_string()).chain(lp_args(job)).collect());
        }
        if ipp::cups_reachable() {
            return Ok(ipp_preview(job));
        }
        if portal::print_dialog_available() {
            return Ok(portal_preview());
        }
    }
    #[cfg(not(all(unix, not(target_arch = "wasm32"))))]
    {
        let _ = job;
    }
    Err(Error::Spool(PRINTING_UNAVAILABLE.into()))
}

/// The platform backend. No backend is available in the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Cups,
    WindowsPdf,
}

pub const fn backend() -> Option<Backend> {
    if cfg!(windows) {
        Some(Backend::WindowsPdf)
    } else if cfg!(all(unix, not(target_arch = "wasm32"))) {
        Some(Backend::Cups)
    } else {
        None
    }
}

/// A diagnostic command preview; creates no job file and starts no process. It names the
/// transport that would run: on CUPS `lp`, else IPP to the reachable server, else the portal's
/// dialog; on Windows, where no program is started, the in-process job instead (`xps-print`,
/// then the printer and the PrintTicket's options as `key=value`). Submit with [`submit`],
/// rather than executing this preview. PDF bytes are supplied separately.
pub fn command_preview(job: &Job) -> Result<Vec<String>, Error> {
    match backend() {
        Some(Backend::Cups) => cups_preview(job),
        Some(Backend::WindowsPdf) => Ok(windows_preview(job)),
        None => Err(Error::Spool(PRINTING_UNAVAILABLE.into())),
    }
}
