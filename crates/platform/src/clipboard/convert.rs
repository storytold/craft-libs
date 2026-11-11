//! Conversions of clipboard payloads: RGBA8 images and `text/uri-list`.

use std::path::PathBuf;

use super::*;

/// `image` as an [`RGBA8`] flavor.
pub(crate) fn rgba8_flavor(image: &ClipboardImage) -> Flavor {
    let mut data = Vec::with_capacity(8 + image.rgba.len());
    data.extend_from_slice(&image.width.to_le_bytes());
    data.extend_from_slice(&image.height.to_le_bytes());
    data.extend_from_slice(&image.rgba);
    Flavor::new(RGBA8, data)
}

/// The bitmap in an [`RGBA8`] flavor's bytes; `None` without the two-word header.
pub(crate) fn rgba8_image(data: &[u8]) -> Option<ClipboardImage> {
    let (header, rgba) = data.split_first_chunk::<8>()?;
    let width = u32::from_le_bytes(header[..4].try_into().ok()?);
    let height = u32::from_le_bytes(header[4..].try_into().ok()?);
    Some(ClipboardImage { width, height, rgba: rgba.to_vec() })
}

/// Paths from a `text/uri-list`: `file:` URIs only, percent-decoded; comments and other schemes
/// are skipped.
pub(super) fn parse_uri_list(list: &str) -> Vec<PathBuf> {
    list.lines()
        .filter_map(|line| {
            let line = line.trim();
            let path = line.strip_prefix("file://")?;
            let path = path.strip_prefix("localhost").unwrap_or(path);
            let decoded = percent_decode(path);
            // `file:///C:/Users/x` is the drive path `C:/Users/x` on Windows.
            let decoded = match decoded.strip_prefix('/') {
                Some(drive) if cfg!(windows) && drive.as_bytes().get(1) == Some(&b':') => drive.to_owned(),
                _ => decoded,
            };
            path.starts_with('/').then(|| PathBuf::from(decoded))
        })
        .collect()
}

pub(super) fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| char::from(b).to_digit(16).map(|d| d as u8);
        match (bytes[i], bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(high), Some(low)) => {
                out.push(high << 4 | low);
                i += 3;
            }
            (byte, _, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `files` as a `text/uri-list` (CRLF line ends, RFC 2483). With `windows`, paths are drive paths
/// and become `file:///C:/Users/…` (the drive kept, separators turned to `/`).
#[cfg(all(feature = "clipboard", not(target_arch = "wasm32")))]
pub(crate) fn uri_list(files: &[PathBuf], windows: bool) -> String {
    let mut list = String::new();
    for file in files {
        let path = file.to_string_lossy();
        let path = if windows { std::borrow::Cow::Owned(format!("/{}", path.replace('\\', "/"))) } else { path };
        list.push_str("file://");
        for &byte in path.as_bytes() {
            if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) || (windows && byte == b':') {
                list.push(char::from(byte));
            } else {
                list.push_str(&format!("%{byte:02X}"));
            }
        }
        list.push_str("\r\n");
    }
    list
}

#[cfg(all(feature = "clipboard", not(target_arch = "wasm32")))]
pub(super) fn trim_cr(path: PathBuf) -> PathBuf {
    match path.to_str() {
        Some(text) if text.ends_with('\r') => PathBuf::from(text.trim_end_matches('\r')),
        _ => path,
    }
}
