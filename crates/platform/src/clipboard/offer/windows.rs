//! The offer on Windows.
//!
//! arboard writes first and empties the clipboard; the formats it doesn't hold are added after
//! it without emptying, each with `clipboard-win`, and the owner marker last.

use clipboard_win::{Clipboard, EnumFormats, formats as cf, raw};

use super::{Plan, Primary, arboard_write, wrap_cf_html};
use crate::clipboard::types::{ClipboardError, HTML, MATHML, PNG, RTF, TEXT, URI_LIST};

/// The registered format the owner marker goes under.
const OWNER_FORMAT: &str = "Craft Clipboard Owner";
/// RTF's registered format.
const RTF_FORMAT: &str = "Rich Text Format";
/// HTML's registered format (CF_HTML).
const HTML_FORMAT: &str = "HTML Format";
/// The encoded picture's registered format.
const PNG_FORMAT: &str = "PNG";
/// The clipboard format names for MathML that Windows programs read (W3C MathML 3 §6.2.4; Office
/// reads and writes both).
const MATHML_FORMATS: [&str; 2] = ["MathML", "MathML Presentation"];
/// Most formats looked at.
const MAX_FORMATS: usize = 64;

/// The owner marker is on the clipboard itself; only a write that couldn't add it is recorded.
pub(super) struct State {
    unmarked: bool,
}

impl State {
    pub(super) fn new() -> Self {
        Self { unmarked: false }
    }

    /// Emptying the clipboard removes the marker with everything else.
    pub(super) fn forget(&mut self) {
        self.unmarked = false;
    }
}

fn open() -> Result<Clipboard, ClipboardError> {
    Clipboard::new_attempts(10).map_err(|_| ClipboardError::Busy)
}

fn register(name: &str) -> Option<u32> {
    raw::register_format(name).map(|format| format.get())
}

/// Adds `bytes` under the registered format `name` beside what is there. A format that can't be
/// added is logged: what arboard wrote still carries the copy. Whether it was added.
fn add(name: &str, bytes: &[u8]) -> bool {
    let Some(format) = register(name) else {
        log::warn!("clipboard: couldn't register {name}");
        return false;
    };
    if let Err(error) = raw::set_without_clear(format, bytes) {
        log::warn!("clipboard: {name}: {error}");
        return false;
    }
    true
}

pub(super) fn write(handle: &mut arboard::Clipboard, state: &mut State, plan: &Plan<'_>, owner: &[u8]) -> Result<usize, ClipboardError> {
    let (index, primary) = arboard_write(handle, plan)?;
    // Until the marker is on the clipboard, `owns` can't tell and the caller compares content.
    state.unmarked = true;
    let _open = match open() {
        Ok(open) => open,
        Err(error) => {
            log::warn!("clipboard: couldn't open it to add formats beside arboard's: {error}");
            return Ok(index);
        }
    };
    match primary {
        Primary::Rich => {
            if let Some(rtf) = &plan.rtf {
                // NUL-terminated, as is usual.
                let bytes: Vec<u8> = rtf.text.as_bytes().iter().copied().chain(std::iter::once(0)).collect();
                add(RTF_FORMAT, &bytes);
            }
            if let Some(mathml) = &plan.mathml {
                // UTF-16LE text with its terminating NUL, as Windows text formats have.
                let bytes: Vec<u8> = mathml.text.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect();
                for name in MATHML_FORMATS {
                    add(name, &bytes);
                }
            }
        }
        Primary::Picture => {
            if let Some(png) = plan.picture.as_ref().and_then(|picture| picture.png) {
                add(PNG_FORMAT, png);
            }
            if let Some(html) = &plan.html {
                add(HTML_FORMAT, &wrap_cf_html(&html.text));
            }
        }
        // arboard's file list is the whole copy.
        Primary::Files => {}
    }
    state.unmarked = !add(OWNER_FORMAT, owner);
    Ok(index)
}

pub(super) fn rtf(_state: &mut State, limit: u64) -> Result<Option<Vec<u8>>, ClipboardError> {
    let Some(format) = register(RTF_FORMAT) else { return Ok(None) };
    let _open = open()?;
    if !raw::is_format_avail(format) {
        return Ok(None);
    }
    // Checked before the copy, so a hostile owner's size never becomes an allocation.
    let Some(size) = raw::size(format) else { return Ok(None) };
    if size.get() as u64 > limit {
        return Err(ClipboardError::TooLarge { limit });
    }
    let mut out = Vec::new();
    raw::get_vec(format, &mut out).map_err(|error| ClipboardError::Other(error.to_string()))?;
    Ok(Some(out))
}

/// The MIME type a format is read as, for [`formats`]; `None` for formats nothing reads.
fn mime(format: u32, name: Option<&str>) -> Option<&'static str> {
    match format {
        cf::CF_UNICODETEXT => Some(TEXT),
        cf::CF_HDROP => Some(URI_LIST),
        cf::CF_DIB | cf::CF_DIBV5 => Some(PNG),
        _ => match name? {
            HTML_FORMAT => Some(HTML),
            PNG_FORMAT => Some(PNG),
            RTF_FORMAT => Some(RTF),
            name if MATHML_FORMATS.contains(&name) => Some(MATHML),
            _ => None,
        },
    }
}

/// The registered and standard names of the formats on offer, and the MIME type of each format
/// a read takes. No content is read.
pub(super) fn formats(_state: &mut State) -> Vec<String> {
    let Ok(_open) = open() else { return Vec::new() };
    let mut names = Vec::new();
    for format in EnumFormats::new().take(MAX_FORMATS) {
        let name = raw::format_name_big(format);
        let mime = mime(format, name.as_deref());
        names.extend(name);
        if let Some(mime) = mime
            && !names.iter().any(|offered| offered == mime)
        {
            names.push(mime.to_owned());
        }
    }
    names
}

pub(super) fn owns(state: &mut State, owner: &[u8]) -> Option<bool> {
    if state.unmarked {
        return None;
    }
    let format = register(OWNER_FORMAT)?;
    let _open = open().ok()?;
    if !raw::is_format_avail(format) {
        return Some(false);
    }
    // A marker longer than ours is another process's; it is never copied.
    if raw::size(format).is_some_and(|size| size.get() > owner.len()) {
        return Some(false);
    }
    let mut out = Vec::new();
    raw::get_vec(format, &mut out).ok()?;
    Some(out == owner)
}
