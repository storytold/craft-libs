//! File names made from user text (N10.09): a document title becomes the suggested name of an
//! export, a page range a set of file names, an attachment's stored name a Save suggestion.
//!
//! The rules hold on every platform, so a name made on Linux still works on Windows: no path
//! separators, reserved characters (`/ \ : * ? " < > |`) or control characters (each becomes
//! `_`); no bidirectional controls (U+061C, U+200E, U+200F, U+202A–U+202E, U+2066–U+2069,
//! dropped: they make `gnp.exe` read as `exe.png`); no leading spaces and no trailing dots or
//! spaces (Windows drops them, so `report.` would write `report`); Windows' device names (`CON`,
//! `PRN`, `AUX`, `NUL`, `CONIN$`, `CONOUT$`, `COM0`–`COM9`, `LPT0`–`LPT9` and the superscript forms,
//! with any extension) get a leading `_`.
//!
//! [`natural_cmp`] is the order file names are listed in.

use std::cmp::Ordering;
use std::iter::Peekable;
use std::str::Chars;

/// The longest name [`safe_file_name`] and [`safe_file_stem`] return, in bytes: 200, so
/// `" (page 12).pdf"` still fits the usual 255-byte limit (PdfCraft `file_stem_from_user`).
pub const MAX_NAME_BYTES: usize = 200;

/// `text` made safe as a whole file name (`"Q3: plan?.pdf"` → `"Q3_ plan_.pdf"`), cut to
/// [`MAX_NAME_BYTES`] at a character boundary, keeping its extension when the cut would remove
/// it. `fallback` (`"attachment"`, `"Untitled"`) when nothing usable is left.
// pdfcraft: crates/ui-egui/src/workflows/files.rs:223 attachment_file_name
pub fn safe_file_name(text: &str, fallback: &str) -> String {
    let cleaned: String = text
        .chars()
        .filter(|c| !is_bidi_control(*c))
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    let name = trim_name(&cleaned);
    if name.is_empty() {
        return fallback.to_string();
    }
    let name = if is_reserved_device_name(name) { format!("_{name}") } else { name.to_string() };
    cut_keeping_extension(&name)
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// No leading whitespace and no trailing dots or whitespace.
fn trim_name(name: &str) -> &str {
    name.trim_start().trim_end_matches(|c: char| c == '.' || c.is_whitespace())
}

/// `name` cut to [`MAX_NAME_BYTES`] at a character boundary. A short extension (under half the
/// limit) stays and the stem before it is cut instead.
fn cut_keeping_extension(name: &str) -> String {
    if name.len() <= MAX_NAME_BYTES {
        return name.to_string();
    }
    let cut = |text: &str, max: usize| -> String {
        let mut end = text.len().min(max);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        trim_name(text.get(..end).unwrap_or_default()).to_string()
    };
    match name.rfind('.') {
        Some(dot) if dot > 0 && name.len() - dot < MAX_NAME_BYTES / 2 => {
            let (stem, ext) = name.split_at(dot);
            format!("{}{ext}", cut(stem, MAX_NAME_BYTES - ext.len()))
        }
        _ => cut(name, MAX_NAME_BYTES),
    }
}

/// `text` made safe as the stem of a file whose extension the app adds (`extension` without the
/// dot, `"pdf"`): the same rules, and a trailing `.<extension>` in any case removed first
/// (`"Report.PDF"` → `"Report"`). Empty when nothing usable is left; the caller picks the
/// default.
// pdfcraft: crates/ui-egui/src/workflows/files.rs:196 file_stem_from_user
pub fn safe_file_stem(text: &str, extension: &str) -> String {
    let text = text.trim();
    let stem = (!extension.is_empty())
        .then(|| {
            let ext_len = extension.len() + 1;
            let split = text.len().checked_sub(ext_len)?;
            let (head, tail) = (text.get(..split)?, text.get(split..)?);
            let dotted = tail.strip_prefix('.')?;
            dotted.eq_ignore_ascii_case(extension).then_some(head)
        })
        .flatten()
        .unwrap_or(text);
    safe_file_name(stem, "")
}

/// Whether `name` names a file directly inside a folder: not empty, no `/` or `\`, no drive
/// (`C:`), not ending in `.` or a space (which also rules out `.` and `..`). On Windows any `:`
/// is refused, which rules out alternate data streams.
// pdfcraft: crates/ui-egui/src/workflows/files.rs:246 is_plain_file_name
pub fn is_plain_file_name(name: &str) -> bool {
    let drive = matches!(name.as_bytes(), [letter, b':', ..] if letter.is_ascii_alphabetic());
    !(name.is_empty() || name.ends_with(['.', ' ']) || name.contains(['/', '\\']) || drive || (cfg!(windows) && name.contains(':')))
}

/// Whether `stem` (a name without its extension, compared without case) is a Windows device
/// name: `CON`, `PRN`, `AUX`, `NUL`, `CONIN$`, `CONOUT$`, `COM0`–`COM9`, `LPT0`–`LPT9`, and the
/// superscript digits `COM¹`–`COM³`, `LPT¹`–`LPT³` Windows also reserves. The crate's path
/// checks use the same rule (`is_dos_device` in `lib.rs`); the body calls it, so the two never
/// drift apart.
// pdfcraft: crates/ui-egui/src/workflows/files.rs:223 attachment_file_name (the device check at :234)
pub fn is_reserved_device_name(stem: &str) -> bool {
    super::is_dos_device(stem)
}

/// `name`, or the first of `name 2.ext`, `name 3.ext`, … ([`CopyNumbering::Space`]) or
/// `name (2).ext`, `name (3).ext`, … ([`CopyNumbering::Parenthesized`]) that `taken` says is
/// free. The number goes before the last extension. With [`Parenthesized`](CopyNumbering::Parenthesized)
/// a name that already ends in a copy number counts on from it (`photo (2).jpg` →
/// `photo (3).jpg`). With [`Space`](CopyNumbering::Space) a trailing number is part of the name,
/// as in Finder, which adds its number after the whole stem (`Chapter 1.docx` →
/// `Chapter 1 2.docx`, `Invoice 2026.pdf` → `Invoice 2026 2.pdf`). Stops after 10 000 tries and returns the last candidate.
// new: PdfCraft, DeckCraft and the export dialog (`numbered_paths`) each number names their own way
pub fn unique_file_name(name: &str, style: CopyNumbering, taken: impl Fn(&str) -> bool) -> String {
    const MAX_TRIES: u32 = 10_000;
    if !taken(name) {
        return name.to_string();
    }
    let (base, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < name.len() => name.split_at(dot),
        _ => (name, ""),
    };
    let (root, last) = match style {
        CopyNumbering::Parenthesized => split_copy_number(base).unwrap_or((base, 1)),
        CopyNumbering::Space => (base, 1),
    };
    let mut candidate = String::new();
    for n in (last + 1)..=last.saturating_add(MAX_TRIES) {
        candidate = match style {
            CopyNumbering::Space => format!("{root} {n}{ext}"),
            CopyNumbering::Parenthesized => format!("{root} ({n}){ext}"),
        };
        if !taken(&candidate) {
            break;
        }
    }
    candidate
}

/// `base` without a trailing ` (n)` copy number, and that number.
fn split_copy_number(base: &str) -> Option<(&str, u32)> {
    let (root, number) = base.rsplit_once(' ')?;
    let digits = number.strip_prefix('(')?.strip_suffix(')')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((root, digits.parse().ok()?))
}

/// How [`unique_file_name`] numbers a name that is taken. Apps pass
/// [`CopyNumbering::platform`] unless the user chose otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CopyNumbering {
    /// `photo 2.jpg` (macOS Finder).
    Space,
    /// `photo (2).jpg` (Windows Explorer, GNOME Files).
    Parenthesized,
}

impl CopyNumbering {
    /// The platform's own file manager's style: [`Space`](Self::Space) on macOS,
    /// [`Parenthesized`](Self::Parenthesized) elsewhere.
    pub const fn platform() -> CopyNumbering {
        if cfg!(target_os = "macos") { CopyNumbering::Space } else { CopyNumbering::Parenthesized }
    }
}

/// File-name order as people sort: runs of ASCII digits compare by value ("2" before "10";
/// leading zeros dropped, then shorter first, then by digits), other characters compare
/// ignoring case, and an exact string comparison breaks ties.
// pc: crates/ui-egui/src/file_open.rs:374 natural_cmp, :403 digit_run
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let (m, n) = (digit_run(&mut x), digit_run(&mut y));
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                match m.len().cmp(&n.len()).then_with(|| m.cmp(n)) {
                    Ordering::Equal => {}
                    o => return o,
                }
            }
            (Some(c), Some(d)) => {
                x.next();
                y.next();
                match c.to_lowercase().cmp(d.to_lowercase()) {
                    Ordering::Equal => {}
                    o => return o,
                }
            }
        }
    }
}

/// Takes the run of ASCII digits at the front of `it`.
fn digit_run(it: &mut Peekable<Chars>) -> String {
    let mut run = String::new();
    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
        run.push(c);
        it.next();
    }
    run
}
