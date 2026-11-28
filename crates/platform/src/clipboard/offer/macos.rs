//! The offer on macOS.
//!
//! arboard writes first (it clears the pasteboard); the types it doesn't hold are added to the
//! item it wrote. The pasteboard's change count after the write tells a later call whether the
//! copy is still ours, so no marker is written.

use objc2_app_kit::NSPasteboard;
use objc2_foundation::{NSData, NSString};

use super::{Plan, Primary, arboard_write};
use crate::clipboard::types::{ClipboardError, HTML, MATHML, PNG, RTF, TEXT, URI_LIST};

/// RTF's pasteboard type (`NSPasteboardTypeRTF`, whose extern static needs `unsafe` to read,
/// which this crate forbids). The other standard types are named the same way.
const RTF_TYPE: &str = "public.rtf";
/// The MathML types (W3C MathML 3 §6.2.4): presentation markup, and the generic type for
/// readers that only know that one.
const MATHML_TYPES: [&str; 2] = ["public.mathml.presentation", "public.mathml"];
/// Most types looked at.
const MAX_TYPES: usize = 128;

/// The pasteboard's change count recorded after this process's last write.
pub(super) struct State {
    change: Option<isize>,
}

impl State {
    pub(super) fn new() -> Self {
        Self { change: None }
    }

    /// Nothing this process wrote is on offer any more.
    pub(super) fn forget(&mut self) {
        self.change = None;
    }
}

pub(super) fn write(handle: &mut arboard::Clipboard, state: &mut State, plan: &Plan<'_>, _owner: &[u8]) -> Result<usize, ClipboardError> {
    let (index, primary) = arboard_write(handle, plan)?;
    let board = NSPasteboard::generalPasteboard();
    // A type that can't be added is logged: what arboard wrote still carries the copy.
    let data = |name: &str, bytes: &[u8]| {
        if !board.setData_forType(Some(&NSData::with_bytes(bytes)), &NSString::from_str(name)) {
            log::warn!("clipboard: couldn't add {name}");
        }
    };
    let string = |name: &str, text: &str| {
        if !board.setString_forType(&NSString::from_str(text), &NSString::from_str(name)) {
            log::warn!("clipboard: couldn't add {name}");
        }
    };
    match primary {
        Primary::Rich => {
            if let Some(rtf) = &plan.rtf {
                data(RTF_TYPE, rtf.text.as_bytes());
            }
            if let Some(mathml) = &plan.mathml {
                for name in MATHML_TYPES {
                    string(name, &mathml.text);
                }
            }
        }
        Primary::Picture => {
            if let Some(png) = plan.picture.as_ref().and_then(|picture| picture.png) {
                data("public.png", png);
            }
            if let Some(html) = &plan.html {
                string("public.html", &html.text);
            }
        }
        // arboard's file list is the whole copy.
        Primary::Files => {}
    }
    state.change = Some(board.changeCount());
    Ok(index)
}

pub(super) fn rtf(_state: &mut State, limit: u64) -> Result<Option<Vec<u8>>, ClipboardError> {
    let Some(data) = NSPasteboard::generalPasteboard().dataForType(&NSString::from_str(RTF_TYPE)) else { return Ok(None) };
    if data.len() as u64 > limit {
        return Err(ClipboardError::TooLarge { limit });
    }
    Ok(Some(data.to_vec()))
}

/// The MIME type a pasteboard type is read as, for [`formats`]; `None` for types nothing reads.
fn mime(name: &str) -> Option<&'static str> {
    match name {
        "public.utf8-plain-text" => Some(TEXT),
        "public.html" => Some(HTML),
        "public.png" | "public.tiff" => Some(PNG),
        "public.file-url" => Some(URI_LIST),
        RTF_TYPE => Some(RTF),
        name if MATHML_TYPES.contains(&name) => Some(MATHML),
        _ => None,
    }
}

/// The pasteboard's types, and the MIME type of each a read takes. `types` lists the formats on
/// offer; no item's data is fetched.
pub(super) fn formats(_state: &mut State) -> Vec<String> {
    let Some(types) = NSPasteboard::generalPasteboard().types() else { return Vec::new() };
    let mut names = Vec::new();
    for offered in types.iter().take(MAX_TYPES) {
        let name = offered.to_string();
        let mime = mime(&name);
        names.push(name);
        if let Some(mime) = mime
            && !names.iter().any(|offered| offered == mime)
        {
            names.push(mime.to_owned());
        }
    }
    names
}

pub(super) fn owns(state: &mut State, _owner: &[u8]) -> Option<bool> {
    let Some(recorded) = state.change else { return Some(false) };
    Some(NSPasteboard::generalPasteboard().changeCount() == recorded)
}
