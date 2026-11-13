//! Bounded file kind detection and extension resolution shared by Craft applications.
//!
//! This crate identifies a format; it does not parse, validate, or decode the file.
//! Callers remain responsible for choosing which identified formats their product supports.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

/// A supported file family, independent of the application model that consumes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Eps,
    PostScript,
    Illustrator,
    Dxf,
    Svg,
    Png,
    Jpeg,
    Bmp,
    Gif,
    Tiff,
    WebP,
    Heif,
    Psd,
    Affinity,
    Ora,
    Kra,
    Xcf,
    Pdn,
    Raw,
    Pcraft,
    Gcode,
    Studio,
    StencilCraft,
}

/// Resolve a name and file bytes to a format. Strong signatures take precedence over
/// extension hints. Only format-specific bounded probes and extension tables live here.
pub fn detect(name: &str, bytes: &[u8]) -> Option<Format> {
    if let Some(found) = craft_eps::sniff(bytes) {
        return Some(match (found.kind, found.illustrator) {
            (_, true) => Format::Illustrator,
            (craft_eps::Kind::Pdf, _) => Format::Pdf,
            (craft_eps::Kind::Eps, _) => Format::Eps,
            (craft_eps::Kind::PostScript, _) => Format::PostScript,
        });
    }
    if bytes.starts_with(b"8BPS") {
        return Some(Format::Psd);
    }
    if bytes.starts_with(b"PDN3") {
        return Some(Format::Pdn);
    }
    if bytes.starts_with(b"gimp xcf ") || gzipped_xcf(name, bytes) {
        return Some(Format::Xcf);
    }
    if bytes.starts_with(b"\0\xffKA") {
        return Some(Format::Affinity);
    }
    if let Some(raster) = raster_signature(bytes) {
        return Some(raster);
    }
    if bytes.starts_with(b"silhouette04;") || bytes.starts_with(b"silhouette05;") {
        return Some(Format::Studio);
    }
    if let Some(ext) = extension(name) {
        if let Some(format) = from_extension(ext) {
            return Some(format);
        }
    }
    let head = bytes.get(..4096).unwrap_or(bytes);
    let xml = head.trim_ascii_start();
    if xml.starts_with(b"<svg") || (xml.starts_with(b"<?xml") && xml.windows(4).any(|w| w == b"<svg")) {
        return Some(Format::Svg);
    }
    if dxf_ascii_prefix(head) || head.starts_with(b"AutoCAD Binary DXF\r\n\x1a\0") {
        return Some(Format::Dxf);
    }
    None
}

/// Resolve a file extension to a family, case-insensitively.
pub fn from_extension(ext: &str) -> Option<Format> {
    match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "pdf" => Some(Format::Pdf),
        "eps" => Some(Format::Eps),
        "ps" => Some(Format::PostScript),
        "ai" => Some(Format::Illustrator),
        "dxf" => Some(Format::Dxf),
        "svg" | "svgz" => Some(Format::Svg),
        "png" => Some(Format::Png),
        "jpg" | "jpeg" => Some(Format::Jpeg),
        "bmp" => Some(Format::Bmp),
        "gif" => Some(Format::Gif),
        "tif" | "tiff" => Some(Format::Tiff),
        "webp" => Some(Format::WebP),
        "heic" | "heif" => Some(Format::Heif),
        "psd" | "psb" => Some(Format::Psd),
        "afdesign" | "afphoto" | "afpub" => Some(Format::Affinity),
        "ora" => Some(Format::Ora),
        "kra" => Some(Format::Kra),
        "xcf" => Some(Format::Xcf),
        "pdn" => Some(Format::Pdn),
        "dng" | "3fr" | "ari" | "arw" | "bay" | "cap" | "cr2" | "cr3" | "dcs" | "dcr" | "drf" | "eip" | "erf" | "fff" | "gpr" | "iiq" | "k25"
        | "kdc" | "mdc" | "mef" | "mos" | "mrw" | "nef" | "nrw" | "obm" | "orf" | "pef" | "ptx" | "pxn" | "r3d" | "raf" | "raw" | "rwl" | "rw2"
        | "rwz" | "sr2" | "srf" | "sti" | "x3f" => Some(Format::Raw),
        "pcraft" => Some(Format::Pcraft),
        "studio" | "studio3" => Some(Format::Studio),
        "stencilcraft" => Some(Format::StencilCraft),
        "gcode" | "nc" => Some(Format::Gcode),
        _ => None,
    }
}

fn extension(name: &str) -> Option<&str> {
    Some(name.rsplit(['/', '\\']).next()?.rsplit_once('.')?.1)
}

fn gzipped_xcf(name: &str, bytes: &[u8]) -> bool {
    let lower = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    bytes.starts_with(&[0x1f, 0x8b]) && (lower.ends_with(".xcf.gz") || lower.ends_with(".xcfgz"))
}

fn dxf_ascii_prefix(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines().map(str::trim);
    matches!((lines.next(), lines.next(), lines.next(), lines.next()), (Some("0"), Some("SECTION"), Some("2"), Some(section)) if matches!(section, "HEADER" | "CLASSES" | "TABLES" | "BLOCKS" | "ENTITIES" | "OBJECTS"))
}

fn raster_signature(bytes: &[u8]) -> Option<Format> {
    let head = bytes.get(..16).unwrap_or(bytes);
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(Format::Png);
    }
    if head.starts_with(b"\xff\xd8\xff") {
        return Some(Format::Jpeg);
    }
    if head.starts_with(b"BM") {
        return Some(Format::Bmp);
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Some(Format::Gif);
    }
    if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") || head.starts_with(b"II+\0") || head.starts_with(b"MM\0+") {
        return Some(Format::Tiff);
    }
    if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP") {
        return Some(Format::WebP);
    }
    None
}
