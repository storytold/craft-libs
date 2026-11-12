//! Locating metadata blocks inside image containers.
//!
//! - JPEG (ITU-T T.81 marker structure): APP1 `Exif\0\0`, APP1 `http://ns.adobe.com/xap/1.0/\0` (XMP),
//!   APP1 `http://ns.adobe.com/xmp/extension/\0` (extended XMP, reassembled), APP2 `ICC_PROFILE\0`
//!   (multi-chunk, reassembled in sequence order), APP13 `Photoshop 3.0\0` 8BIM resource 0x0404 (IPTC-IIM).
//! - PNG (PNG spec 3rd ed.): `eXIf`, `iTXt` with keyword `XML:com.adobe.xmp` (uncompressed).
//! - WebP (RIFF container spec): `EXIF`, `XMP `, `ICCP` chunks.
//! - TIFF-based files (TIFF, DNG and most raws): the whole file is the Exif block; XMP (700), ICC (34675) and
//!   IPTC (33723) from IFD0.
//! - Canon CR3 (ISO base media file, see [`crate::cr3`]): `CMT1`/`CMT2`/`CMT4` merged into one Exif block; the
//!   XMP `uuid` box.
//! - JPEG XL (ISO base media file, see [`crate::jxl`]): the `Exif` and `xml ` boxes, plain or Brotli-wrapped
//!   in `brob`; a bare codestream has no metadata.
//!
//! PSD, MRW and HEIF files are read by `craft-psd`, `craft-raw` and `craft-heif`.

use craft_tiff::{Tiff, tags};

/// Metadata blocks found in a file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Embedded {
    /// TIFF-structured Exif block (without the `Exif\0\0` prefix).
    pub exif: Option<Vec<u8>>,
    /// XMP packet (the main one; extended XMP is appended by [`Embedded::xmp_extended`]).
    pub xmp: Option<String>,
    /// Extended XMP (JPEG only), reassembled.
    pub xmp_extended: Option<String>,
    /// The GUID every Extended XMP segment carries, set only when all segments share it and
    /// their chunks tile exactly the full length they declare (no gap, overlap or excess).
    pub xmp_extended_guid: Option<String>,
    pub icc: Option<Vec<u8>>,
    /// IPTC-IIM stream.
    pub iptc: Option<Vec<u8>>,
    /// True when the PNG/WebP chunk stream could not be fully inspected.
    pub chunk_walk_incomplete: bool,
    /// Why the Extended XMP the standard packet links to (`xmpNote:HasExtendedXMP`) could not be
    /// read: its chunks are missing, truncated, too large, or do not cover their declared length
    /// exactly once. [`Embedded::xmp_extended`] is `None` then.
    pub xmp_extended_error: Option<String>,
}

/// Sniff the container and collect its metadata blocks. Unknown containers yield an empty result.
pub fn embedded(bytes: &[u8]) -> Embedded {
    if bytes.starts_with(b"FUJIFILMCCD-RAW") {
        // Fujifilm RAF: metadata lives in the embedded JPEG whose (offset, length) are big-endian u32s at byte 84
        let be = |i: usize| bytes.get(i..i + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize);
        return match (be(84), be(88)) {
            (Some(o), Some(l)) if o > 0 => bytes.get(o..o.saturating_add(l).min(bytes.len())).map(jpeg_segments).unwrap_or_default(),
            _ => Embedded::default(),
        };
    }
    if let Some(c) = crate::cr3::parse_cr3(bytes) {
        return Embedded { exif: crate::cr3::merged_exif(&c), xmp: c.xmp.map(|b| String::from_utf8_lossy(b).into_owned()), ..Default::default() };
    }
    if crate::jxl::is_jxl_container(bytes) {
        return crate::jxl::metadata(bytes);
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        jpeg_segments(bytes)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png_chunks(bytes)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        webp_chunks(bytes)
    } else if let Ok(t) = Tiff::parse(bytes) {
        let ifd0 = &t.ifds[0];
        Embedded {
            exif: Some(bytes.to_vec()),
            xmp: ifd0.bytes(tags::XMP).map(|b| String::from_utf8_lossy(b).into_owned()),
            xmp_extended: None,
            xmp_extended_guid: None,
            icc: ifd0.bytes(tags::ICC_PROFILE).map(|b| b.to_vec()),
            iptc: ifd0.value(tags::IPTC_NAA).map(|v| match v {
                craft_tiff::Value::Byte(b) | craft_tiff::Value::Undefined(b) => b.clone(),
                other => craft_tiff::writer::encode(t.order, other),
            }),
            chunk_walk_incomplete: false,
            xmp_extended_error: None,
        }
    } else {
        Embedded::default()
    }
}

const XMP_SIG: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const XMP_EXT_SIG: &[u8] = b"http://ns.adobe.com/xmp/extension/\0";

/// Walk JPEG markers up to SOS and collect APP1 Exif / XMP, APP2 ICC and APP13 IPTC.
pub fn jpeg_segments(b: &[u8]) -> Embedded {
    jpeg_segments_inner(b, false).0
}

/// Walk JPEG metadata and return every ordinary XMP APP1 packet in segment order, along with the
/// usual [`Embedded`] projection (which retains only the first ordinary packet).
pub fn jpeg_segments_with_all_xmp(b: &[u8]) -> (Embedded, Vec<String>) {
    jpeg_segments_inner(b, true)
}

fn jpeg_segments_inner(b: &[u8], collect_all_xmp: bool) -> (Embedded, Vec<String>) {
    let mut out = Embedded::default();
    let mut xmp_packets = Vec::new();
    if !b.starts_with(&[0xff, 0xd8]) {
        return (out, xmp_packets);
    }
    let mut icc_parts: [Option<&[u8]>; 255] = [None; 255];
    let mut icc_total = None;
    let mut icc_invalid = false;
    let mut ext: Vec<(u32, Vec<u8>)> = Vec::new();
    // GUID and declared full length of every Extended XMP segment, to verify they belong together
    let mut ext_headers: Vec<(&[u8], u32)> = Vec::new();
    let mut i = 2usize;
    while i + 4 <= b.len() {
        if b[i] != 0xff {
            break;
        }
        let marker = b[i + 1];
        if marker == 0xff {
            i += 1; // fill byte
            continue;
        }
        if marker == 0xd8 || (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if len < 2 {
            break;
        }
        let Some(start) = i.checked_add(4) else { break };
        let Some(declared_end) = i.checked_add(2).and_then(|at| at.checked_add(len)) else { break };
        let complete = declared_end <= b.len();
        let end = declared_end.min(b.len());
        let Some(payload) = b.get(start..end) else { break };
        match marker {
            0xe1 if payload.starts_with(b"Exif\0") && out.exif.is_none() => out.exif = Some(payload.get(6..).unwrap_or(&[]).to_vec()),
            0xe1 if payload.starts_with(XMP_SIG) && out.xmp.is_none() => {
                let packet = String::from_utf8_lossy(&payload[XMP_SIG.len()..]).into_owned();
                if collect_all_xmp {
                    xmp_packets.push(packet.clone());
                }
                out.xmp = Some(packet);
            }
            0xe1 if payload.starts_with(XMP_SIG) && collect_all_xmp => {
                xmp_packets.push(String::from_utf8_lossy(&payload[XMP_SIG.len()..]).into_owned());
            }
            0xe1 if payload.starts_with(XMP_EXT_SIG) => {
                // GUID (32) + full length (4) + offset (4) + data
                let p = &payload[XMP_EXT_SIG.len()..];
                if p.len() >= 40 {
                    let full = u32::from_be_bytes([p[32], p[33], p[34], p[35]]);
                    let off = u32::from_be_bytes([p[36], p[37], p[38], p[39]]);
                    ext_headers.push((&p[..32], full));
                    ext.push((off, p[40..].to_vec()));
                }
            }
            0xe2 if payload.starts_with(b"ICC_PROFILE\0") => {
                if !complete || payload.len() < 14 {
                    icc_invalid = true;
                } else if let Some(header) = payload.get(12..14) {
                    let (Some(&sequence), Some(&total)) = (header.first(), header.get(1)) else {
                        icc_invalid = true;
                        i = end;
                        continue;
                    };
                    if sequence == 0 || total == 0 || sequence > total || icc_total.is_some_and(|expected| expected != total) {
                        icc_invalid = true;
                    } else {
                        icc_total = Some(total);
                        if let Some(slot) = icc_parts.get_mut(usize::from(sequence - 1)) {
                            if slot.is_some() {
                                icc_invalid = true;
                            } else if let Some(data) = payload.get(14..) {
                                *slot = Some(data);
                            } else {
                                icc_invalid = true;
                            }
                        } else {
                            icc_invalid = true;
                        }
                    }
                } else {
                    icc_invalid = true;
                }
            }
            0xed if payload.starts_with(b"Photoshop 3.0\0") && out.iptc.is_none() => out.iptc = photoshop_iptc(&payload[14..]),
            _ => {}
        }
        i = end;
    }
    if let Some(total) = icc_total {
        let mut profile_len = 0usize;
        for chunk in icc_parts.iter().take(usize::from(total)) {
            let Some(bytes) = chunk else {
                icc_invalid = true;
                break;
            };
            let Some(next_len) = profile_len.checked_add(bytes.len()) else {
                icc_invalid = true;
                break;
            };
            profile_len = next_len;
        }
        let maximum_len = usize::from(total).checked_mul(65_519);
        if maximum_len.is_none_or(|maximum| profile_len > maximum) {
            icc_invalid = true;
        }
        if !icc_invalid {
            let mut profile = Vec::with_capacity(profile_len);
            for bytes in icc_parts.iter().take(usize::from(total)).flatten() {
                profile.extend_from_slice(bytes);
            }
            out.icc = Some(profile);
        }
    }
    attach_extension(&mut out, b, ext, &ext_headers);
    (out, xmp_packets)
}

/// Put the Extended XMP of JPEG `b` into `out`: when the standard packet links one, only that GUID's
/// chunks count, checked strictly ([`extended_xmp`]); otherwise every chunk, offset-sorted.
fn attach_extension(out: &mut Embedded, b: &[u8], mut ext: Vec<(u32, Vec<u8>)>, ext_headers: &[(&[u8], u32)]) {
    // a standard packet that links an extension: only that GUID's chunks count, checked strictly
    let reference = (!ext.is_empty())
        .then(|| out.xmp.as_deref().and_then(|x| crate::parse_xmp(x).ok()).and_then(|d| crate::xmp::extended_xmp_reference(&d).ok().flatten()))
        .flatten();
    if let Some(guid) = reference {
        match extended_xmp(b, &guid) {
            Ok(packet) => {
                out.xmp_extended = Some(packet);
                out.xmp_extended_guid = Some(guid);
            }
            Err(error) => out.xmp_extended_error = Some(error),
        }
    } else if !ext.is_empty() {
        ext.sort_by_key(|p| p.0);
        out.xmp_extended_guid = verified_extension_guid(ext_headers, &ext);
        out.xmp_extended = Some(String::from_utf8_lossy(&ext.into_iter().flat_map(|p| p.1).collect::<Vec<u8>>()).into_owned());
    }
}

/// Most bytes an Extended XMP packet may have, and most chunks it may be split into.
const MAX_EXTENDED_XMP_BYTES: usize = 16 * 1024 * 1024;
const MAX_EXTENDED_XMP_CHUNKS: usize = 4096;

/// The Extended XMP packet of a JPEG for `guid` (upper-case hex, from the standard packet's
/// `xmpNote:HasExtendedXMP`; Adobe XMP Specification Part 3, 1.1.3.1): the APP1 chunks carrying
/// that GUID, which must all declare one full length of at most [`MAX_EXTENDED_XMP_BYTES`] and
/// together cover it exactly once, in at most [`MAX_EXTENDED_XMP_CHUNKS`] complete segments.
/// Chunks of any other GUID are not looked at, so they cannot exhaust the limits.
fn extended_xmp(b: &[u8], guid: &str) -> Result<String, String> {
    let mut chunks: Vec<(u32, &[u8])> = Vec::new();
    let (mut total, mut received) = (None, 0usize);
    let mut i = 2usize;
    while let (Some(&0xff), Some(&marker)) = (b.get(i), b.get(i + 1)) {
        if marker == 0xff {
            i += 1;
            continue;
        }
        if marker == 0xd8 || (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        let Some(len) = b.get(i + 2..i + 4).map(|l| usize::from(u16::from_be_bytes([l[0], l[1]]))).filter(|&l| l >= 2) else { break };
        let (start, end) = (i + 4, i + 2 + len);
        let payload = b.get(start..end.min(b.len())).unwrap_or_default();
        if let Some(p) = payload.strip_prefix(XMP_EXT_SIG).filter(|_| marker == 0xe1)
            && p.get(..32).is_some_and(|g| g.eq_ignore_ascii_case(guid.as_bytes()))
        {
            if end > b.len() {
                return Err("Extended XMP APP1 segment is truncated".into());
            }
            let (Some(full), Some(offset), Some(data)) = (p.get(32..36), p.get(36..40), p.get(40..)) else {
                return Err("Extended XMP APP1 header is truncated".into());
            };
            let full = u32::from_be_bytes([full[0], full[1], full[2], full[3]]);
            if *total.get_or_insert(full) != full {
                return Err("Extended XMP chunks declare different full lengths".into());
            }
            received = received.saturating_add(data.len());
            if usize::try_from(full).map_or(true, |f| f > MAX_EXTENDED_XMP_BYTES) || received > MAX_EXTENDED_XMP_BYTES {
                return Err("Extended XMP is larger than 16 MiB".into());
            }
            if chunks.len() >= MAX_EXTENDED_XMP_CHUNKS {
                return Err("Extended XMP has too many chunks".into());
            }
            chunks.push((u32::from_be_bytes([offset[0], offset[1], offset[2], offset[3]]), data));
        }
        i = end;
    }
    let total = total.ok_or("the referenced Extended XMP is missing")?;
    chunks.sort_by_key(|c| c.0);
    let mut packet = Vec::with_capacity(received);
    for (offset, data) in chunks {
        if usize::try_from(offset).ok() != Some(packet.len()) {
            return Err("Extended XMP chunks have a gap, an overlap or a repeated offset".into());
        }
        packet.extend_from_slice(data);
    }
    if u32::try_from(packet.len()).ok() != Some(total) || total == 0 {
        return Err("Extended XMP chunks do not cover their declared full length".into());
    }
    String::from_utf8(packet).map_err(|_| "Extended XMP is not UTF-8".into())
}

/// The GUID shared by all Extended XMP segments when their offset-sorted chunks are contiguous from
/// zero and add up to the full length every segment declares; `None` otherwise.
fn verified_extension_guid(headers: &[(&[u8], u32)], chunks_by_offset: &[(u32, Vec<u8>)]) -> Option<String> {
    let (&(guid, full), rest) = headers.split_first()?;
    if rest.iter().any(|&(other_guid, other_full)| other_guid != guid || other_full != full) {
        return None;
    }
    let mut next = 0u64;
    for (offset, data) in chunks_by_offset {
        if u64::from(*offset) != next {
            return None;
        }
        next = next.checked_add(u64::try_from(data.len()).ok()?)?;
    }
    (next == u64::from(full)).then(|| String::from_utf8_lossy(guid).into_owned())
}

/// IPTC-IIM block (resource 0x0404) from a Photoshop image-resource stream (`8BIM` records).
fn photoshop_iptc(mut b: &[u8]) -> Option<Vec<u8>> {
    while b.len() >= 12 && &b[..4] == b"8BIM" {
        let id = u16::from_be_bytes([b[4], b[5]]);
        let name_len = b[6] as usize;
        let name_total = (1 + name_len).next_multiple_of(2);
        let size_at = 6 + name_total;
        let size = u32::from_be_bytes(b.get(size_at..size_at + 4)?.try_into().ok()?) as usize;
        let data_at = size_at + 4;
        let data = b.get(data_at..data_at.checked_add(size)?)?;
        if id == 0x0404 {
            return Some(data.to_vec());
        }
        b = b.get(data_at + size.next_multiple_of(2)..)?;
    }
    None
}

/// PNG chunks: `eXIf`, uncompressed `iTXt` XMP.
pub fn png_chunks(b: &[u8]) -> Embedded {
    let mut out = Embedded::default();
    let mut i = 8usize;
    let mut ended = false;
    loop {
        let Some(header_end) = i.checked_add(8).filter(|end| *end <= b.len()) else {
            out.chunk_walk_incomplete = true;
            break;
        };
        let Some(header) = b.get(i..header_end) else {
            out.chunk_walk_incomplete = true;
            break;
        };
        let Some(length_bytes) = header.get(..4) else {
            out.chunk_walk_incomplete = true;
            break;
        };
        let Ok(length_bytes) = <[u8; 4]>::try_from(length_bytes) else {
            out.chunk_walk_incomplete = true;
            break;
        };
        let len = u32::from_be_bytes(length_bytes) as usize;
        let Some(data_end) = header_end.checked_add(len) else {
            out.chunk_walk_incomplete = true;
            break;
        };
        let Some(chunk_end) = data_end.checked_add(4).filter(|end| *end <= b.len()) else {
            out.chunk_walk_incomplete = true;
            break;
        };
        let Some(typ) = header.get(4..8) else {
            out.chunk_walk_incomplete = true;
            break;
        };
        let Some(data) = b.get(header_end..data_end) else {
            out.chunk_walk_incomplete = true;
            break;
        };
        match typ {
            b"eXIf" => out.exif = Some(crate::strip_exif_header(data).to_vec()),
            b"iTXt" if data.starts_with(b"XML:com.adobe.xmp\0") => {
                // keyword\0 compression-flag compression-method language\0 translated\0 text
                let rest = &data[18..];
                if rest.len() >= 2 && rest[0] == 0 {
                    let rest = &rest[2..];
                    let mut parts = rest.splitn(3, |&c| c == 0);
                    let (_, _, text) = (parts.next(), parts.next(), parts.next());
                    if let Some(t) = text {
                        out.xmp = Some(String::from_utf8_lossy(t).into_owned());
                    }
                }
            }
            b"IEND" => {
                ended = len == 0 && chunk_end == b.len();
                out.chunk_walk_incomplete |= !ended;
                break;
            }
            _ => {}
        }
        i = chunk_end;
    }
    if !ended {
        out.chunk_walk_incomplete = true;
    }
    out
}

/// WebP RIFF chunks `EXIF`, `XMP `, `ICCP`.
pub fn webp_chunks(b: &[u8]) -> Embedded {
    let mut out = Embedded::default();
    let Some(size_bytes) = b.get(4..8) else {
        out.chunk_walk_incomplete = true;
        return out;
    };
    let Ok(size_bytes) = <[u8; 4]>::try_from(size_bytes) else {
        out.chunk_walk_incomplete = true;
        return out;
    };
    let riff_size = u32::from_le_bytes(size_bytes) as usize;
    let Some(riff_end) = 8usize.checked_add(riff_size) else {
        out.chunk_walk_incomplete = true;
        return out;
    };
    if riff_size < 4 || riff_end < 12 {
        out.chunk_walk_incomplete = true;
        return out;
    }
    if riff_end != b.len() {
        out.chunk_walk_incomplete = true;
    }
    let end = riff_end.min(b.len());
    let mut i = 12usize;
    while i < end {
        let Some(header_end) = i.checked_add(8).filter(|next| *next <= end) else {
            out.chunk_walk_incomplete = true;
            break;
        };
        let Some(header) = b.get(i..header_end) else {
            out.chunk_walk_incomplete = true;
            break;
        };
        let Some(length_bytes) = header.get(4..8) else {
            out.chunk_walk_incomplete = true;
            break;
        };
        let Ok(length_bytes) = <[u8; 4]>::try_from(length_bytes) else {
            out.chunk_walk_incomplete = true;
            break;
        };
        let len = u32::from_le_bytes(length_bytes) as usize;
        let Some(data_end) = header_end.checked_add(len) else {
            out.chunk_walk_incomplete = true;
            break;
        };
        let Some(chunk_end) = data_end.checked_add(len & 1).filter(|next| *next <= end) else {
            out.chunk_walk_incomplete = true;
            break;
        };
        let Some(id) = header.get(..4) else {
            out.chunk_walk_incomplete = true;
            break;
        };
        let Some(data) = b.get(header_end..data_end) else {
            out.chunk_walk_incomplete = true;
            break;
        };
        match id {
            b"EXIF" => out.exif = Some(crate::strip_exif_header(data).to_vec()),
            b"XMP " => out.xmp = Some(String::from_utf8_lossy(data).into_owned()),
            b"ICCP" => out.icc = Some(data.to_vec()),
            _ => {}
        }
        i = chunk_end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exif::tests::sample_exif;
    use craft_tiff::{ByteOrder, IfdBuilder, TiffWriter, Value};

    fn seg(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0xff, marker];
        v.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn icc_seg(sequence: u8, total: u8, data: &[u8]) -> Vec<u8> {
        let mut payload = b"ICC_PROFILE\0".to_vec();
        payload.extend_from_slice(&[sequence, total]);
        payload.extend_from_slice(data);
        seg(0xe2, &payload)
    }

    fn jpeg_with_icc(parts: &[Vec<u8>]) -> Vec<u8> {
        let mut jpeg = vec![0xff, 0xd8];
        let packet = crate::write_xmp(&crate::Metadata { title: Some("retained".into()), ..Default::default() });
        let mut xmp = XMP_SIG.to_vec();
        xmp.extend_from_slice(packet.as_bytes());
        jpeg.extend_from_slice(&seg(0xe1, &xmp));
        for part in parts {
            jpeg.extend_from_slice(part);
        }
        jpeg.extend_from_slice(&[0xff, 0xda, 0, 2]);
        jpeg
    }

    pub(crate) fn sample_jpeg() -> Vec<u8> {
        let mut j = vec![0xff, 0xd8];
        j.extend(seg(0xe0, b"JFIF\0\x01\x02\0\0\x01\0\x01\0\0"));
        let mut exif = b"Exif\0\0".to_vec();
        exif.extend(sample_exif(ByteOrder::Big));
        j.extend(seg(0xe1, &exif));
        let meta = crate::Metadata { title: Some("Sunset".into()), rating: Some(4), ..Default::default() };
        let mut xmp = XMP_SIG.to_vec();
        xmp.extend(crate::write_xmp(&meta).into_bytes());
        j.extend(seg(0xe1, &xmp));
        // ICC in two chunks, out of order
        j.extend(seg(0xe2, b"ICC_PROFILE\0\x02\x02WORLD"));
        j.extend(seg(0xe2, b"ICC_PROFILE\0\x01\x02HELLO"));
        // IPTC
        let iim = crate::iptc::tests::sample_iim();
        let mut ps = b"Photoshop 3.0\0".to_vec();
        ps.extend_from_slice(b"8BIM\x04\x0c\0\0");
        ps.extend_from_slice(&2u32.to_be_bytes());
        ps.extend_from_slice(&[1, 2]);
        ps.extend_from_slice(b"8BIM\x04\x04\0\0");
        ps.extend_from_slice(&(iim.len() as u32).to_be_bytes());
        ps.extend_from_slice(&iim);
        j.extend(seg(0xed, &ps));
        j.extend_from_slice(&[0xff, 0xda, 0, 2, 1, 2, 3, 0xff, 0xd9]);
        j
    }

    #[test]
    fn jpeg_blocks() {
        let j = sample_jpeg();
        let e = jpeg_segments(&j);
        assert_eq!(e.icc.as_deref(), Some(&b"HELLOWORLD"[..]));
        assert!(e.xmp.as_deref().unwrap().contains("Sunset"));
        assert!(e.iptc.is_some());
        let m = crate::extract(&j);
        assert_eq!(m.iso, Some(400));
        assert_eq!(m.title.as_deref(), Some("Sunset"));
        assert_eq!(m.rating, Some(4)); // XMP overrides EXIF rating 3
        assert_eq!(m.keywords, vec!["alpha".to_string(), "beta".to_string()]); // from IPTC
        assert_eq!(m.caption.as_deref(), Some("A caption"));
    }

    fn jpeg_with_orientation(exif_orientation: Option<crate::Orientation>, xmp_orientation: &str) -> Vec<u8> {
        let mut jpeg = vec![0xff, 0xd8];
        if let Some(orientation) = exif_orientation {
            let mut exif = b"Exif\0\0".to_vec();
            exif.extend(crate::exif::write_exif(&crate::Metadata { orientation: Some(orientation), ..Default::default() }));
            jpeg.extend(seg(0xe1, &exif));
        }
        let packet = format!(
            r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:tiff="http://ns.adobe.com/tiff/1.0/" tiff:Orientation="{xmp_orientation}"/></rdf:RDF>"#
        );
        let mut xmp = XMP_SIG.to_vec();
        xmp.extend_from_slice(packet.as_bytes());
        jpeg.extend(seg(0xe1, &xmp));
        jpeg.extend_from_slice(&[0xff, 0xd9]);
        jpeg
    }

    #[test]
    fn embedded_xmp_orientation_requires_integral_value_and_respects_exif() {
        let malformed = crate::extract(&jpeg_with_orientation(None, "6.9"));
        assert_eq!(malformed.orientation, None);

        let malformed_xmp_with_exif = crate::extract(&jpeg_with_orientation(Some(crate::Orientation::Rotate180), "6.5"));
        assert_eq!(malformed_xmp_with_exif.orientation.map(|orientation| orientation.to_exif()), Some(3));

        let valid_xmp = crate::extract(&jpeg_with_orientation(None, "6"));
        assert_eq!(valid_xmp.orientation.map(|orientation| orientation.to_exif()), Some(6));

        let valid_exif = crate::extract(&jpeg_with_orientation(Some(crate::Orientation::Rotate180), "6"));
        assert_eq!(valid_exif.orientation.map(|orientation| orientation.to_exif()), Some(3));
    }

    #[test]
    fn jpeg_invalid_xmp_iso_stays_absent_and_exif_iso_wins() {
        let make_jpeg = |exif: Option<Vec<u8>>, xmp: &str| {
            let mut jpeg = vec![0xff, 0xd8];
            if let Some(exif) = exif {
                let mut payload = b"Exif\0\0".to_vec();
                payload.extend(exif);
                jpeg.extend(seg(0xe1, &payload));
            }
            let mut payload = XMP_SIG.to_vec();
            payload.extend_from_slice(xmp.as_bytes());
            jpeg.extend(seg(0xe1, &payload));
            jpeg.extend_from_slice(&[0xff, 0xda, 0, 2, 0xff, 0xd9]);
            jpeg
        };
        let invalid = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:exif="http://ns.adobe.com/exif/1.0/" exif:ISOSpeedRatings="0.5"/></rdf:RDF>"#;
        assert_eq!(crate::extract(&make_jpeg(None, invalid)).iso, None);

        let valid = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:exif="http://ns.adobe.com/exif/1.0/" exif:ISOSpeedRatings="200"/></rdf:RDF>"#;
        assert_eq!(crate::extract(&make_jpeg(Some(sample_exif(ByteOrder::Big)), valid)).iso, Some(400));
    }

    #[test]
    fn jpeg_rejected_xmp_dimensions_keep_exif_dimensions_and_other_metadata() {
        let mut jpeg = vec![0xff, 0xd8];
        let mut exif = b"Exif\0\0".to_vec();
        exif.extend(crate::exif::tests::sample_exif(ByteOrder::Big));
        jpeg.extend(seg(0xe1, &exif));
        let packet = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description xmlns:tiff="http://ns.adobe.com/tiff/1.0/" xmlns:exif="http://ns.adobe.com/exif/1.0/"
                tiff:ImageWidth="640.9" exif:PixelYDimension="8000" dc:title="Retained"/>
        </rdf:RDF>"#;
        let mut xmp = XMP_SIG.to_vec();
        xmp.extend_from_slice(packet.as_bytes());
        jpeg.extend(seg(0xe1, &xmp));
        jpeg.extend_from_slice(&[0xff, 0xda, 0, 2]);

        let metadata = crate::extract(&jpeg);
        assert_eq!(metadata.width, Some(6000));
        assert_eq!(metadata.height, Some(4000));
        assert_eq!(metadata.title.as_deref(), Some("Retained"));
    }

    #[test]
    fn jpeg_rejected_xmp_dimension_falls_back_during_whole_file_extract() {
        let mut jpeg = vec![0xff, 0xd8];
        let packet = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description xmlns:tiff="http://ns.adobe.com/tiff/1.0/" xmlns:exif="http://ns.adobe.com/exif/1.0/"
                tiff:ImageWidth="640.9" exif:PixelXDimension="800"/>
        </rdf:RDF>"#;
        let mut xmp = XMP_SIG.to_vec();
        xmp.extend_from_slice(packet.as_bytes());
        jpeg.extend(seg(0xe1, &xmp));
        jpeg.extend_from_slice(&[0xff, 0xda, 0, 2]);

        assert_eq!(crate::extract(&jpeg).width, Some(800));
    }

    #[test]
    fn jpeg_invalid_iptc_time_created_allows_xmp_and_exif_fallbacks() {
        fn iim() -> Vec<u8> {
            let mut data = Vec::new();
            for (dataset, value) in [(55, &b"20260101"[..]), (60, b"121314+01"), (5, b"retained title")] {
                data.extend_from_slice(&[0x1c, 2, dataset]);
                data.extend_from_slice(&(value.len() as u16).to_be_bytes());
                data.extend_from_slice(value);
            }
            data
        }

        fn jpeg(xmp: &str, exif: Option<&[u8]>) -> Vec<u8> {
            let mut jpeg = vec![0xff, 0xd8];
            if let Some(exif) = exif {
                let mut payload = b"Exif\0\0".to_vec();
                payload.extend_from_slice(exif);
                jpeg.extend_from_slice(&seg(0xe1, &payload));
            }
            let mut payload = XMP_SIG.to_vec();
            payload.extend_from_slice(xmp.as_bytes());
            jpeg.extend_from_slice(&seg(0xe1, &payload));
            let mut resource = b"Photoshop 3.0\0".to_vec();
            resource.extend_from_slice(b"8BIM\x04\x04\0\0");
            let iim = iim();
            resource.extend_from_slice(&(iim.len() as u32).to_be_bytes());
            resource.extend_from_slice(&iim);
            if iim.len() % 2 == 1 {
                resource.push(0);
            }
            jpeg.extend_from_slice(&seg(0xed, &resource));
            jpeg.extend_from_slice(&[0xff, 0xda, 0, 2]);
            jpeg
        }

        let xmp = crate::write_xmp(&crate::Metadata {
            capture_time: Some(crate::DateTime { year: 2026, month: 1, day: 2, hour: 10, offset_minutes: Some(0), ..Default::default() }),
            ..Default::default()
        });
        let fallback = jpeg(&xmp, None);
        let embedded = embedded(&fallback);
        assert!(embedded.iptc.is_some(), "raw IIM remains available");
        assert_eq!(crate::extract(&fallback).capture_time.map(|time| time.to_iso()).as_deref(), Some("2026-01-02T10:00:00+00:00"));
        assert_eq!(crate::extract(&fallback).title.as_deref(), Some("retained title"));

        let mut exif_ifd = IfdBuilder::new();
        exif_ifd.set(0x9003, Value::Ascii("2026:01:03 11:12:13".into()));
        let mut ifd0 = IfdBuilder::new();
        ifd0.set_child(0x8769, exif_ifd);
        let exif = TiffWriter::new(ByteOrder::Little, false).write(&[ifd0]).unwrap();
        let exif_precedence = jpeg(&xmp, Some(&exif));
        assert_eq!(crate::extract(&exif_precedence).capture_time.map(|time| time.to_iso()).as_deref(), Some("2026-01-03T11:12:13"));
    }

    #[test]
    fn jpeg_extended_xmp() {
        let mut j = vec![0xff, 0xd8];
        for (off, part) in [(5u32, &b"world"[..]), (0, b"hello")] {
            let mut p = XMP_EXT_SIG.to_vec();
            p.extend_from_slice(&[b'A'; 32]);
            p.extend_from_slice(&10u32.to_be_bytes());
            p.extend_from_slice(&off.to_be_bytes());
            p.extend_from_slice(part);
            j.extend(seg(0xe1, &p));
        }
        assert_eq!(jpeg_segments(&j).xmp_extended.as_deref(), Some("helloworld"));
        assert_eq!(jpeg_segments(&j).xmp_extended_guid, Some("A".repeat(32)));
    }

    /// A GUID mix, a short chunk set or a wrong declared length leaves the extension unverified.
    #[test]
    fn jpeg_extended_xmp_guid_needs_matching_segments_and_length() {
        let build = |segments: &[(u8, u32, u32, &[u8])]| {
            let mut j = vec![0xff, 0xd8];
            for &(guid, full, off, part) in segments {
                let mut p = XMP_EXT_SIG.to_vec();
                p.extend_from_slice(&[guid; 32]);
                p.extend_from_slice(&full.to_be_bytes());
                p.extend_from_slice(&off.to_be_bytes());
                p.extend_from_slice(part);
                j.extend(seg(0xe1, &p));
            }
            jpeg_segments(&j).xmp_extended_guid
        };
        assert!(build(&[(b'A', 10, 0, b"hello"), (b'B', 10, 5, b"world")]).is_none(), "two GUIDs");
        assert!(build(&[(b'A', 10, 0, b"hello")]).is_none(), "half the declared length");
        assert!(build(&[(b'A', 9, 0, b"hello"), (b'A', 9, 5, b"world")]).is_none(), "longer than declared");
        assert!(build(&[(b'A', 10, 0, b"hello"), (b'A', 10, 4, b"oworl")]).is_none(), "overlapping chunks");
    }

    #[test]
    fn jpeg_icc_accepts_reordered_chunks() {
        let parts = [icc_seg(2, 2, b"world"), icc_seg(1, 2, b"hello")];
        let embedded = jpeg_segments(&jpeg_with_icc(&parts));
        assert_eq!(embedded.icc.as_deref(), Some(&b"helloworld"[..]));
    }

    #[test]
    fn jpeg_icc_rejects_incomplete_or_incoherent_chunks() {
        let cases = [
            ("missing", vec![icc_seg(1, 2, b"helloworld")]),
            ("duplicate", vec![icc_seg(1, 2, b"hello"), icc_seg(1, 2, b"world")]),
            ("zero sequence", vec![icc_seg(0, 1, b"helloworld")]),
            ("zero total", vec![icc_seg(1, 0, b"helloworld")]),
            ("out of range", vec![icc_seg(2, 1, b"helloworld")]),
            ("gap", vec![icc_seg(1, 3, b"hello"), icc_seg(3, 3, b"world")]),
            ("inconsistent totals", vec![icc_seg(1, 2, b"hello"), icc_seg(2, 3, b"world")]),
            ("short header", vec![seg(0xe2, b"ICC_PROFILE\0\x01")]),
        ];
        for (name, parts) in cases {
            let embedded = jpeg_segments(&jpeg_with_icc(&parts));
            assert!(embedded.icc.is_none(), "{name} ICC chunks must be rejected");
            assert!(embedded.xmp.as_deref().is_some_and(|x| x.contains("retained")), "{name}: unrelated XMP must remain readable");
        }
    }

    #[test]
    fn jpeg_icc_rejects_truncated_segment_and_keeps_other_metadata() {
        let mut truncated = icc_seg(1, 1, b"profile");
        let declared = u16::from_be_bytes([truncated[2], truncated[3]]).saturating_add(1);
        truncated[2..4].copy_from_slice(&declared.to_be_bytes());
        let mut jpeg = vec![0xff, 0xd8];
        let xmp = crate::write_xmp(&crate::Metadata { title: Some("kept".into()), ..Default::default() });
        let mut packet = XMP_SIG.to_vec();
        packet.extend_from_slice(xmp.as_bytes());
        jpeg.extend_from_slice(&seg(0xe1, &packet));
        jpeg.extend_from_slice(&truncated);

        let embedded = jpeg_segments(&jpeg);
        assert!(embedded.icc.is_none());
        assert!(embedded.xmp.as_deref().is_some_and(|x| x.contains("kept")));
    }

    #[test]
    fn png_and_webp() {
        let exif = sample_exif(ByteOrder::Little);
        let mut p = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut chunk = |typ: &[u8], data: &[u8]| {
            p.extend_from_slice(&(data.len() as u32).to_be_bytes());
            p.extend_from_slice(typ);
            p.extend_from_slice(data);
            p.extend_from_slice(&[0; 4]);
        };
        chunk(b"eXIf", &exif);
        let mut itxt = b"XML:com.adobe.xmp\0\0\0\0\0".to_vec();
        itxt.extend(crate::write_xmp(&crate::Metadata { label: Some("Red".into()), ..Default::default() }).into_bytes());
        chunk(b"iTXt", &itxt);
        chunk(b"IEND", &[]);
        let m = crate::extract(&p);
        assert_eq!(m.iso, Some(400));
        assert_eq!(m.label.as_deref(), Some("Red"));
        assert!(!embedded(&p).chunk_walk_incomplete);

        let mut w = b"RIFF\0\0\0\0WEBP".to_vec();
        let mut body = |id: &[u8], data: &[u8]| {
            w.extend_from_slice(id);
            w.extend_from_slice(&(data.len() as u32).to_le_bytes());
            w.extend_from_slice(data);
            if data.len() % 2 == 1 {
                w.push(0);
            }
        };
        body(b"VP8X", &[0; 10]);
        body(b"ICCP", b"abc");
        let mut ex = b"Exif\0\0".to_vec();
        ex.extend_from_slice(&exif);
        body(b"EXIF", &ex);
        let riff_size = w.len().saturating_sub(8) as u32;
        w[4..8].copy_from_slice(&riff_size.to_le_bytes());
        let e = webp_chunks(&w);
        assert!(!e.chunk_walk_incomplete);
        assert_eq!(e.icc.as_deref(), Some(&b"abc"[..]));
        assert_eq!(crate::extract(&w).f_number, Some(2.8));
    }

    #[test]
    fn chunk_walk_reports_truncated_png_and_webp() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&32u32.to_be_bytes());
        png.extend_from_slice(b"iTXtshort");
        assert!(png_chunks(&png).chunk_walk_incomplete);

        let mut webp = b"RIFF\0\0\0\0WEBP".to_vec();
        webp.extend_from_slice(b"XMP ");
        webp.extend_from_slice(&32u32.to_le_bytes());
        webp.extend_from_slice(b"short");
        let riff_size = webp.len().saturating_sub(8).saturating_add(27) as u32;
        webp[4..8].copy_from_slice(&riff_size.to_le_bytes());
        assert!(webp_chunks(&webp).chunk_walk_incomplete);
    }

    #[test]
    fn tiff_file_is_its_own_exif() {
        let t = sample_exif(ByteOrder::Big);
        assert_eq!(crate::extract(&t).model.as_deref(), Some("Model X"));
        assert_eq!(embedded(b"not an image"), Embedded::default());
    }

    #[test]
    fn raf_reads_its_embedded_jpeg() {
        let j = sample_jpeg();
        let mut raf = b"FUJIFILMCCD-RAW 0201FF000000TEST".to_vec();
        raf.resize(120, 0);
        raf[84..88].copy_from_slice(&120u32.to_be_bytes());
        raf[88..92].copy_from_slice(&(j.len() as u32).to_be_bytes());
        raf.extend_from_slice(&j);
        assert_eq!(crate::extract(&raf).iso, Some(400));
        for n in 0..raf.len() {
            let _ = crate::extract(&raf[..n]);
        }
    }

    #[test]
    fn truncated_containers_do_not_panic() {
        let j = sample_jpeg();
        for n in 0..j.len() {
            let _ = crate::extract(&j[..n]);
        }
    }
}
