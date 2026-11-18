//! Bounds the Brotli-compressed metadata boxes (`brob`) of a JPEG XL container.
//!
//! jxl-oxide decompresses a `brob` box (Exif, XMP) into a buffer its `AllocTracker` does not
//! count, so a few-KB file can expand into gigabytes before any pixel check runs. Before the file
//! reaches it, every `brob` box is inflated here under [`BROB_CAP`] and rewritten as the plain box
//! it wraps; a box that would pass the cap is dropped (metadata is a courtesy, the pixels still
//! decode). The cap is one budget for the whole file, spent box by box. A file without a `brob` box is not copied at all.

use std::io::Read;

/// Most bytes all the compressed metadata boxes of one file may expand to together: real Exif and
/// XMP packets are a few KB to a few hundred KB, and a file of many small boxes that each inflate
/// to just under a per-box cap would otherwise hold N times the cap.
pub(crate) const BROB_CAP: usize = 8 << 20;

const SIGNATURE: &[u8] = b"\0\0\0\x0cJXL \r\n\x87\n";
const BROB: &[u8; 4] = b"brob";

/// One ISOBMFF box located in the file.
struct BoxSpan {
    start: usize,
    header: usize,
    end: usize,
}

impl BoxSpan {
    fn kind<'a>(&self, bytes: &'a [u8]) -> &'a [u8] {
        bytes.get(self.start + 4..self.start + 8).unwrap_or(&[])
    }

    fn payload<'a>(&self, bytes: &'a [u8]) -> &'a [u8] {
        bytes.get(self.start + self.header..self.end).unwrap_or(&[])
    }
}

/// The box starting at `at`, or `None` when its header is cut off or its size is nonsense. A box
/// that runs past the end of a truncated file ends at the file's end.
fn box_at(bytes: &[u8], at: usize) -> Option<BoxSpan> {
    let head = bytes.get(at..at.checked_add(8)?)?;
    let size = u32::from_be_bytes(head.get(..4)?.try_into().ok()?) as u64;
    let (header, total) = match size {
        0 => (8, (bytes.len() - at) as u64),
        1 => (16, u64::from_be_bytes(bytes.get(at + 8..at + 16)?.try_into().ok()?)),
        n => (8, n),
    };
    let end = usize::try_from(total).ok().filter(|t| *t >= header).and_then(|t| at.checked_add(t)).unwrap_or(bytes.len());
    Some(BoxSpan { start: at, header, end: end.min(bytes.len()) })
}

/// Every box of the container, in order, up to the first one that cannot be read.
fn boxes(bytes: &[u8]) -> Vec<BoxSpan> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(span) = box_at(bytes, at) {
        at = span.end;
        found.push(span);
        if at >= bytes.len() {
            break;
        }
    }
    found
}

/// Inflates `compressed`, or `None` when it is broken or would pass `cap` bytes.
pub(crate) fn inflate_capped(compressed: &[u8], cap: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let limit = u64::try_from(cap).ok()?.checked_add(1)?;
    brotli_decompressor::Decompressor::new(compressed, 4096).take(limit).read_to_end(&mut out).ok()?;
    (out.len() <= cap).then_some(out)
}

/// A plain box of type `kind` holding `data`.
pub(crate) fn plain_box(kind: &[u8], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16);
    match u32::try_from(data.len() + 8) {
        Ok(size) => out.extend_from_slice(&size.to_be_bytes()),
        Err(_) => out.extend_from_slice(&1u32.to_be_bytes()),
    }
    out.extend_from_slice(kind);
    if u32::try_from(data.len() + 8).is_err() {
        out.extend_from_slice(&(data.len() as u64 + 16).to_be_bytes());
    }
    out.extend_from_slice(data);
    out
}

/// The plain box a `brob` box wraps, or `None` when it must be dropped. A kept box is charged to
/// `budget`, so the boxes of one file never inflate past the cap together.
fn unwrap_brob(span: &BoxSpan, bytes: &[u8], budget: &mut usize) -> Option<Vec<u8>> {
    let payload = span.payload(bytes);
    let (inner, compressed) = (payload.get(..4)?, payload.get(4..)?);
    if inner == BROB {
        return None;
    }
    let inflated = inflate_capped(compressed, *budget)?;
    *budget -= inflated.len();
    Some(plain_box(inner, &inflated))
}

/// `bytes` with every `brob` box inflated, `cap` bytes in all (or removed), or `None` when the file is
/// not a container or has no `brob` box and can be used as it is.
pub(crate) fn sanitize(bytes: &[u8], cap: usize) -> Option<Vec<u8>> {
    if !bytes.starts_with(SIGNATURE) {
        return None;
    }
    let spans = boxes(bytes);
    if !spans.iter().any(|s| s.kind(bytes) == BROB) {
        return None;
    }
    let mut budget = cap;
    let mut out = Vec::with_capacity(bytes.len());
    for span in &spans {
        match (span.kind(bytes) == BROB, bytes.get(span.start..span.end)) {
            (true, _) => out.extend(unwrap_brob(span, bytes, &mut budget).unwrap_or_default()),
            (false, Some(whole)) => out.extend_from_slice(whole),
            (false, None) => {}
        }
    }
    out.extend_from_slice(bytes.get(spans.last().map_or(0, |s| s.end)..).unwrap_or(&[]));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "II*\0\x08\0\0\0hello exif" as Brotli.
    const EXIF_BR: [u8; 22] =
        [0x8b, 0x08, 0x80, 0x49, 0x49, 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x20, 0x65, 0x78, 0x69, 0x66, 0x03];
    /// 64 MiB of `a` as Brotli: 54 bytes.
    const BOMB_BR: [u8; 54] = [
        0xcf, 0xff, 0xff, 0x7f, 0xf8, 0x25, 0xc2, 0xe2, 0xb1, 0x40, 0x20, 0xf7, 0xfe, 0x9f, 0xff, 0xff, 0xff, 0xf0, 0x4b, 0x00, 0xc4, 0x61, 0x01,
        0x80, 0xee, 0xfd, 0x3f, 0xff, 0xff, 0xff, 0xe1, 0x97, 0x00, 0x88, 0xc3, 0x22, 0x00, 0xdd, 0xfb, 0x7f, 0xfe, 0xff, 0xff, 0xc3, 0x2f, 0x01,
        0x10, 0x87, 0x05, 0x00, 0xba, 0xf7, 0xff, 0x03,
    ];

    fn container(boxes: &[Vec<u8>]) -> Vec<u8> {
        let mut file = SIGNATURE.to_vec();
        boxes.iter().for_each(|b| file.extend_from_slice(b));
        file
    }

    fn brob_of(inner: &[u8; 4], compressed: &[u8]) -> Vec<u8> {
        plain_box(BROB, &[inner.as_slice(), compressed].concat())
    }

    #[test]
    fn a_bomb_passes_the_cap_and_a_real_packet_does_not() {
        assert_eq!(inflate_capped(&EXIF_BR, BROB_CAP).as_deref(), Some(&b"II*\0\x08\0\0\0hello exif"[..]));
        assert!(inflate_capped(&BOMB_BR, BROB_CAP).is_none());
        assert!(inflate_capped(&BOMB_BR[..20], BROB_CAP).is_none(), "a cut stream is not a result");
    }

    #[test]
    fn a_container_without_brob_is_used_as_it_is() {
        assert!(sanitize(&container(&[plain_box(b"Exif", b"abc")]), BROB_CAP).is_none());
        assert!(sanitize(&[0xff, 0x0a, 0x00], BROB_CAP).is_none(), "a bare codestream has no boxes");
    }

    #[test]
    fn a_small_brob_becomes_the_plain_box_it_wraps() {
        let tail = plain_box(b"jxlc", b"codestream");
        let out = sanitize(&container(&[brob_of(b"Exif", &EXIF_BR), tail.clone()]), BROB_CAP).unwrap();
        assert_eq!(out, container(&[plain_box(b"Exif", b"II*\0\x08\0\0\0hello exif"), tail]));
    }

    #[test]
    fn a_brob_bomb_is_dropped_and_the_rest_kept() {
        let tail = plain_box(b"jxlc", b"codestream");
        let out = sanitize(&container(&[brob_of(b"xml ", &BOMB_BR), tail.clone()]), BROB_CAP).unwrap();
        assert_eq!(out, container(&[tail]));
    }

    #[test]
    fn a_nested_or_empty_brob_is_dropped() {
        let nested = brob_of(BROB, &EXIF_BR);
        let empty = plain_box(BROB, b"");
        assert_eq!(sanitize(&container(&[nested, empty]), BROB_CAP).unwrap(), container(&[]));
    }

    #[test]
    fn many_small_boxes_share_one_budget() {
        let exif = b"II*\0\x08\0\0\0hello exif".len();
        let boxes: Vec<Vec<u8>> = (0..4).map(|_| brob_of(b"Exif", &EXIF_BR)).collect();
        let out = sanitize(&container(&boxes), exif * 2 + 1).unwrap();
        let kept = out.windows(4).filter(|w| *w == b"Exif").count();
        assert_eq!(kept, 2, "the boxes past the file's budget are dropped");
        assert!(out.len() < SIGNATURE.len() + 2 * (exif + 8) + 1);
    }

    #[test]
    fn a_cut_off_file_keeps_its_tail_verbatim() {
        let mut file = container(&[brob_of(b"Exif", &EXIF_BR)]);
        file.extend_from_slice(b"\0\0\0\x20jxlc half");
        let out = sanitize(&file, BROB_CAP).unwrap();
        assert!(out.ends_with(b"\0\0\0\x20jxlc half"));
    }
}
