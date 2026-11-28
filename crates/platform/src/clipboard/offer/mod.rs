//! What the system clipboard offers and how it is written, per platform.
//!
//! arboard writes the bitmap, or the HTML with its text, or the text, on every desktop platform.
//! What it can't hold goes beside it where the platform lets a second writer add formats: on
//! Windows and macOS RTF, MathML and (beside a bitmap) PNG and HTML, after arboard's write; on
//! Wayland everything in one offer. On X11 arboard owns the selection and serves only its own
//! targets, so a copy there carries text and HTML, or the bitmap, alone. Reading another
//! program's RTF works on all four.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(test)]
mod tests;
#[cfg(all(unix, not(target_os = "macos")))]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
use self::macos as platform;
#[cfg(all(unix, not(target_os = "macos")))]
use self::unix as platform;
#[cfg(windows)]
use self::windows as platform;

use std::borrow::Cow;

use super::backend::{Decode, system_error};
use super::convert::{parse_uri_list, rgba8_image};
use super::types::{ClipboardError, ClipboardImage, Flavor, HTML, MATHML, PNG, RGBA8, RTF, TEXT, URI_LIST};

/// The platform's offer state: the cached X11 `TARGETS` connection and the macOS change count recorded at a write.
pub(super) struct Offer {
    state: platform::State,
}

impl Offer {
    /// An offer with nothing cached.
    pub(super) fn new() -> Self {
        Self { state: platform::State::new() }
    }
}

/// One text-like flavour a write offers, with its index in the caller's flavours.
struct Part<'a> {
    index: usize,
    text: Cow<'a, str>,
}

/// The bitmap a write offers: its pixels for arboard and, when the caller gave one, its PNG
/// bytes for the platforms that take an encoded picture beside them.
struct Picture<'a> {
    index: usize,
    pixels: ClipboardImage,
    png: Option<&'a [u8]>,
}

/// The copied files a write offers: the local paths of the first file list that names any.
struct FileList {
    index: usize,
    paths: Vec<std::path::PathBuf>,
}

/// What a write puts on the clipboard: the first flavour of each kind among the caller's.
struct Plan<'a> {
    text: Option<Part<'a>>,
    html: Option<Part<'a>>,
    rtf: Option<Part<'a>>,
    mathml: Option<Part<'a>>,
    picture: Option<Picture<'a>>,
    files: Option<FileList>,
}

/// Which of a plan's shapes a write takes: arboard holds a bitmap, rich text or a file list, one of them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Primary {
    Picture,
    Rich,
    /// Copied files, the way a file manager's Copy puts them.
    Files,
}

impl<'a> Plan<'a> {
    fn new(flavors: &'a [Flavor], decode: Option<&Decode>) -> Self {
        let part = |mime: &str| {
            flavors
                .iter()
                .enumerate()
                .find(|(_, flavor)| flavor.mime == mime)
                .map(|(index, flavor)| Part { index, text: String::from_utf8_lossy(&flavor.data) })
        };
        let png = flavors.iter().find(|flavor| flavor.mime == PNG).map(|flavor| flavor.data.as_slice());
        let picture = flavors.iter().enumerate().find_map(|(index, flavor)| {
            let pixels = match &*flavor.mime {
                RGBA8 => rgba8_image(&flavor.data),
                PNG => decode.and_then(|decode| decode(&flavor.data)),
                _ => None,
            }?;
            pixels.is_whole().then_some(Picture { index, pixels, png })
        });
        // Only `file:` URIs become files; a list that names none leaves the write to the other flavours.
        let files = flavors.iter().enumerate().find_map(|(index, flavor)| {
            let paths = (flavor.mime == URI_LIST).then(|| parse_uri_list(&String::from_utf8_lossy(&flavor.data)))?;
            (!paths.is_empty()).then_some(FileList { index, paths })
        });
        Self { text: part(TEXT), html: part(HTML), rtf: part(RTF), mathml: part(MATHML), picture, files }
    }

    /// The shape of the write: whichever of the bitmap, the HTML or text and the file list the caller put first.
    fn primary(&self) -> Option<Primary> {
        let rich = [&self.html, &self.text].into_iter().flatten().map(|part| part.index).min();
        [
            self.picture.as_ref().map(|picture| (picture.index, Primary::Picture)),
            rich.map(|index| (index, Primary::Rich)),
            self.files.as_ref().map(|files| (files.index, Primary::Files)),
        ]
        .into_iter()
        .flatten()
        .min_by_key(|&(index, _)| index)
        .map(|(_, primary)| primary)
    }
}

/// arboard's part of a write: the bitmap, else the HTML with its text, else the text. Answers
/// the index of the flavour it wrote and the shape it took.
fn arboard_write(handle: &mut arboard::Clipboard, plan: &Plan<'_>) -> Result<(usize, Primary), ClipboardError> {
    match plan.primary().ok_or(ClipboardError::NoFormat)? {
        Primary::Picture => {
            let picture = plan.picture.as_ref().ok_or(ClipboardError::NoFormat)?;
            let pixels = &picture.pixels;
            handle
                .set_image(arboard::ImageData { width: pixels.width as usize, height: pixels.height as usize, bytes: Cow::Borrowed(&pixels.rgba) })
                .map_err(system_error)?;
            Ok((picture.index, Primary::Picture))
        }
        Primary::Rich => match (&plan.html, &plan.text) {
            (Some(html), alt) => {
                handle.set_html(html.text.clone(), alt.as_ref().map(|text| text.text.clone())).map_err(system_error)?;
                Ok((html.index, Primary::Rich))
            }
            (None, Some(text)) => {
                handle.set_text(text.text.clone()).map_err(system_error)?;
                Ok((text.index, Primary::Rich))
            }
            (None, None) => Err(ClipboardError::NoFormat),
        },
        Primary::Files => {
            let files = plan.files.as_ref().ok_or(ClipboardError::NoFormat)?;
            handle.set().file_list(&files.paths).map_err(system_error)?;
            Ok((files.index, Primary::Files))
        }
    }
}

/// Every flavour the platform can hold in one offer, tagged with `owner`; the index of the first one written.
pub(super) fn write(
    handle: &mut arboard::Clipboard,
    offer: &mut Offer,
    flavors: &[Flavor],
    decode: Option<&Decode>,
    owner: &[u8],
) -> Result<usize, ClipboardError> {
    if flavors.is_empty() {
        offer.state.forget();
        handle.clear().map_err(system_error)?;
        return Ok(0);
    }
    platform::write(handle, &mut offer.state, &Plan::new(flavors, decode), owner)
}

/// The clipboard's RTF text, at most `limit` bytes of it.
pub(super) fn read_rtf(offer: &mut Offer, limit: u64) -> Result<Option<String>, ClipboardError> {
    let Some(bytes) = platform::rtf(&mut offer.state, limit)? else { return Ok(None) };
    // RTF is 7-bit by the spec; stray 8-bit bytes become replacement characters. Windows
    // programs NUL-terminate it.
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_end_matches('\0');
    Ok(text.trim_start().starts_with("{\\rtf").then(|| text.to_owned()))
}

/// The MIME names of the formats on offer.
pub(super) fn formats(offer: &mut Offer) -> Vec<String> {
    platform::formats(&mut offer.state)
}

/// Whether the offer holds one of `mimes`.
pub(super) fn has(offer: &mut Offer, mimes: &[&str]) -> bool {
    let offered = formats(offer);
    #[cfg(all(unix, not(target_os = "macos")))]
    let holds = unix::offered_has;
    #[cfg(any(windows, target_os = "macos"))]
    let holds = named_has;
    mimes.iter().any(|&mime| holds(&offered, mime))
}

/// Whether this process owns the clipboard, by the `owner` marker; `None` when the platform cannot tell.
pub(super) fn owns(offer: &mut Offer, owner: &[u8]) -> Option<bool> {
    platform::owns(&mut offer.state, owner)
}

/// Whether `offered`, as Windows and macOS `formats` name the standard formats (by the MIME
/// type each is read as), holds `mime`. Any bitmap is listed as `image/png`.
#[cfg(any(windows, target_os = "macos"))]
fn named_has(offered: &[String], mime: &str) -> bool {
    use super::types::BITMAP;
    let mime = match mime {
        BITMAP | RGBA8 => PNG,
        mime => mime,
    };
    offered.iter().any(|name| name == mime)
}

/// `fragment` wrapped as a CF_HTML clipboard payload.
///
/// Offsets are zero-padded byte offsets from the start of the payload; `EndHTML` excludes the
/// trailing NUL.
#[cfg(any(windows, test))]
fn wrap_cf_html(fragment: &str) -> Vec<u8> {
    let version = "Version:0.9";
    let start_html = "\r\nStartHTML:";
    let end_html = "\r\nEndHTML:";
    let start_fragment = "\r\nStartFragment:";
    let end_fragment = "\r\nEndFragment:";
    let before = "\r\n<html>\r\n<body>\r\n<!--StartFragment-->\r\n";
    let after = "\r\n<!--EndFragment-->\r\n</body>\r\n</html>\0";
    // Each offset is written as ten digits.
    let header = version.len() + start_html.len() + 10 + end_html.len() + 10 + start_fragment.len() + 10 + end_fragment.len() + 10;
    // `before` opens with a line break, so the document starts two bytes after the header.
    let html_at = header + 2;
    let fragment_at = header + before.len();
    let fragment_end = fragment_at + fragment.len();
    let html_end = fragment_end + after.len() - 1;
    format!(
        "{version}{start_html}{html_at:010}{end_html}{html_end:010}{start_fragment}{fragment_at:010}{end_fragment}{fragment_end:010}{before}{fragment}{after}"
    )
    .into_bytes()
}
