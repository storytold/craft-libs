//! Picture bytes: their kind and pixel size, read from the first bytes.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

/// Extensions of the picture kinds kept under their own name.
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "emf", "wmf", "svg", "webp"];

/// The picture kind of `data` by its signature.
pub(crate) fn sniff_image(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG") {
        Some("png")
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if data.starts_with(b"GIF8") {
        Some("gif")
    } else if data.starts_with(b"BM") {
        Some("bmp")
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else if data.starts_with(b"II*\0") || data.starts_with(b"MM\0*") {
        Some("tif")
    } else {
        None
    }
}

/// The extension a picture is kept under: its own when it names a picture kind, else the one its
/// bytes say (PNG when nothing tells).
pub(crate) fn image_ext(ext: &str, data: &[u8]) -> String {
    let ext = ext.to_ascii_lowercase();
    if IMAGE_EXTS.contains(&ext.as_str()) { ext } else { sniff_image(data).unwrap_or("png").to_string() }
}

/// Width and height in pixels of a PNG, GIF, BMP or JPEG.
pub(crate) fn pixel_size(data: &[u8]) -> Option<(u32, u32)> {
    let be32 = |o: usize| data.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let le16 = |o: usize| data.get(o..o + 2).map(|b| u32::from(u16::from_le_bytes([b[0], b[1]])));
    let be16 = |o: usize| data.get(o..o + 2).map(|b| u32::from(u16::from_be_bytes([b[0], b[1]])));
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((be32(16)?, be32(20)?));
    }
    if data.starts_with(b"GIF8") {
        return Some((le16(6)?, le16(8)?));
    }
    if data.starts_with(b"BM") {
        let w = data.get(18..22).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))?;
        let h = data.get(22..26).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))?;
        return Some((w.unsigned_abs(), h.unsigned_abs()));
    }
    if data.starts_with(b"\xff\xd8") {
        let mut i = 2usize;
        let mut guard = 0;
        while i + 9 < data.len() && data.get(i) == Some(&0xFF) && guard < 10_000 {
            guard += 1;
            // a marker may follow any number of 0xFF fill bytes (ITU-T T.81, B.1.1.2)
            while data.get(i + 1) == Some(&0xFF) {
                i += 1;
            }
            let marker = *data.get(i + 1)?;
            let length = be16(i + 2)? as usize;
            if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
                return Some((be16(i + 7)?, be16(i + 5)?));
            }
            i = i.checked_add(2 + length)?;
        }
    }
    None
}
