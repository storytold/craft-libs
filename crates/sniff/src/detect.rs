//! The signature checks behind [`identify`].

use crate::Kind;

/// How many leading bytes [`identify`] reads at most. Every signature it knows lies within them.
pub const HEAD: usize = 4096;

/// The largest universal Mach-O architecture count taken as Mach-O rather than a Java class.
/// Both start `CAFEBABE`; a class file's next four bytes are its minor and major version, and
/// every released class version (major 45, JDK 1.0, onwards) reads as at least 45 there, while
/// real universal binaries hold a handful of architectures.
const MAX_FAT_ARCHES: u32 = 44;

/// The ISO BMFF brands read from an `ftyp` box: the major brand and up to this many compatible ones.
const MAX_BRANDS: usize = 32;

/// The MPEG transport stream packet size and the sync byte each packet starts with.
const TS_PACKET: usize = 188;
const TS_SYNC: u8 = 0x47;

/// Name the format of a file from its leading bytes, or `None` when no signature matches.
///
/// Only the first [`HEAD`] bytes are read. Signatures at the start of the file are checked
/// before the weaker ones further in or built from few bits (TAR at byte 257, MP3 and AAC frame
/// headers, MPEG transport stream packets), so a file is never named by a weak signature when a
/// strong one matches.
///
/// `CAFEBABE` starts both a universal Mach-O and a Java class file. The next big-endian 32-bit
/// value is a Mach-O architecture count or a class file's version: up to 44 is taken as
/// [`Kind::MachOFat`], 45 and over as [`Kind::JavaClass`]. A caller that knows the file name
/// (`.class`) may prefer that.
pub fn identify(bytes: &[u8]) -> Option<Kind> {
    let head = bytes.get(..HEAD).unwrap_or(bytes);
    leading(head).or_else(|| container(head)).or_else(|| trailing(head))
}

/// Signatures at byte 0 that need nothing more than a fixed prefix.
fn leading(b: &[u8]) -> Option<Kind> {
    const PREFIXES: &[(&[u8], Kind)] = &[
        (b"\x89PNG\r\n\x1a\n", Kind::Png),
        (b"\xff\xd8\xff", Kind::Jpeg),
        (b"GIF87a", Kind::Gif),
        (b"GIF89a", Kind::Gif),
        (b"\xff\x0a", Kind::Jxl),
        (b"\x00\x00\x00\x0cJXL \r\n\x87\n", Kind::Jxl),
        (b"qoif", Kind::Qoi),
        (b"\x76\x2f\x31\x01", Kind::OpenExr),
        (b"fLaC", Kind::Flac),
        (b"MThd\x00\x00\x00\x06", Kind::Midi),
        (b"#!AMR", Kind::Amr),
        (b"FLV\x01", Kind::Flv),
        (b"\x00\x00\x01\xba", Kind::MpegPs),
        (b"glTF", Kind::Glb),
        (b"Kaydara FBX Binary  ", Kind::Fbx),
        (b"ply\n", Kind::Ply),
        (b"ply\r", Kind::Ply),
        (b"PK\x03\x04", Kind::Zip),
        (b"PK\x05\x06", Kind::Zip),
        (b"7z\xbc\xaf\x27\x1c", Kind::SevenZip),
        (b"Rar!\x1a\x07\x00", Kind::Rar),
        (b"Rar!\x1a\x07\x01\x00", Kind::Rar),
        (b"\x1f\x8b\x08", Kind::Gzip),
        (b"\xfd7zXZ\x00", Kind::Xz),
        (b"\x28\xb5\x2f\xfd", Kind::Zstd),
        (b"070701", Kind::Cpio),
        (b"070702", Kind::Cpio),
        (b"070707", Kind::Cpio),
        (b"\xc7\x71", Kind::Cpio),
        (b"\x71\xc7", Kind::Cpio),
        (b"\x7fELF", Kind::Elf),
        (b"\xfe\xed\xfa\xce", Kind::MachO),
        (b"\xfe\xed\xfa\xcf", Kind::MachO),
        (b"\xce\xfa\xed\xfe", Kind::MachO),
        (b"\xcf\xfa\xed\xfe", Kind::MachO),
        (b"\xca\xfe\xba\xbf", Kind::MachOFat),
        (b"\0asm", Kind::Wasm),
        (b"dex\n", Kind::Dex),
        (b"SQLite format 3\0", Kind::Sqlite),
        (b"bplist00", Kind::BinaryPlist),
        (b"%PDF-", Kind::Pdf),
        (b"ID3", Kind::Mp3),
    ];
    PREFIXES.iter().find(|(magic, _)| b.starts_with(magic)).map(|&(_, kind)| kind).or_else(|| checked(b))
}

/// Signatures at byte 0 whose prefix is short or shared, confirmed by a field after it.
fn checked(b: &[u8]) -> Option<Kind> {
    if b.starts_with(b"MZ") {
        return Some(dos(b));
    }
    match b.get(..4)? {
        [0xca, 0xfe, 0xba, 0xbe] => Some(if be32(b, 4)? <= MAX_FAT_ARCHES { Kind::MachOFat } else { Kind::JavaClass }),
        b"II*\0" | b"MM\0*" | b"II+\0" | b"MM\0+" => Some(Kind::Tiff),
        b"8BPS" => matches!(be16(b, 4)?, 1 | 2).then_some(Kind::Psd),
        [0, 0, 1, 0] => (le16(b, 4)? > 0 && b.get(9).copied()? == 0).then_some(Kind::Ico),
        [b'B', b'Z', b'h', level] => (b'1'..=b'9').contains(level).then_some(Kind::Bzip2),
        [b'B', b'M', ..] => bmp(b),
        _ => None,
    }
}

/// `MZ`: a PE file when `e_lfanew` (at 0x3C) points at `PE\0\0` within the bytes given.
fn dos(b: &[u8]) -> Kind {
    let pe = le32(b, 0x3c)
        .and_then(|offset| usize::try_from(offset).ok())
        .and_then(|offset| b.get(offset..offset.checked_add(4)?))
        .is_some_and(|sig| sig == b"PE\0\0");
    if pe { Kind::Pe } else { Kind::DosExecutable }
}

/// `BM` alone is two letters of text, so a BMP also needs zero reserved fields and a known
/// DIB header size.
fn bmp(b: &[u8]) -> Option<Kind> {
    let reserved = b.get(6..10)? == [0, 0, 0, 0];
    let dib = matches!(le32(b, 14)?, 12 | 16 | 40 | 52 | 56 | 64 | 108 | 124);
    (reserved && dib).then_some(Kind::Bmp)
}

/// Container formats named by what the container says it holds.
fn container(b: &[u8]) -> Option<Kind> {
    match (b.get(..4)?, b.get(8..12)) {
        (b"RIFF", Some(b"WAVE")) => Some(Kind::Wav),
        (b"RIFF", Some(b"WEBP")) => Some(Kind::WebP),
        (b"RIFF", Some(b"AVI ")) => Some(Kind::Avi),
        (b"FORM", Some(b"AIFF" | b"AIFC")) => Some(Kind::Aiff),
        (b"OggS", _) => ogg(b),
        ([0x1a, 0x45, 0xdf, 0xa3], _) => Some(ebml(b)),
        _ => iso_bmff(b),
    }
}

/// An Ogg stream is named by its first packet: Opus and Theora have their own types.
fn ogg(b: &[u8]) -> Option<Kind> {
    let segments = usize::from(*b.get(26)?);
    let packet = b.get(27usize.checked_add(segments)?..).unwrap_or_default();
    Some(if packet.starts_with(b"OpusHead") {
        Kind::Opus
    } else if packet.starts_with(b"\x80theora") {
        Kind::OggVideo
    } else {
        Kind::Ogg
    })
}

/// EBML: WebM and Matroska differ by the `DocType` element (ID 0x4282) in the EBML header.
fn ebml(b: &[u8]) -> Kind {
    let header = b.get(..64).unwrap_or(b);
    let doc_type = header.windows(2).position(|w| w == [0x42, 0x82]).and_then(|at| {
        let size = header.get(at.checked_add(2)?).copied()?;
        // A one-byte EBML size has its top bit set; the value is the remaining seven bits.
        let len = usize::from(size & 0x7f);
        let start = at.checked_add(3)?;
        if size & 0x80 == 0 {
            return None;
        }
        header.get(start..start.checked_add(len)?)
    });
    if doc_type == Some(b"webm") { Kind::WebM } else { Kind::Matroska }
}

/// ISO base media files: an `ftyp` box whose brands say what it holds, or an old QuickTime
/// movie starting with one of its own top-level atoms.
fn iso_bmff(b: &[u8]) -> Option<Kind> {
    let box_type = b.get(4..8)?;
    let size = usize::try_from(be32(b, 0)?).ok()?;
    if box_type != b"ftyp" {
        let atom = matches!(box_type, b"moov" | b"mdat" | b"wide") && size >= 8;
        return atom.then_some(Kind::QuickTime);
    }
    if size < 16 {
        return None;
    }
    let end = size.min(b.len()).min(16usize.saturating_add(MAX_BRANDS.saturating_mul(4)));
    let major: &[u8; 4] = b.get(8..12)?.try_into().ok()?;
    let compatible = b.get(16..end).unwrap_or_default().as_chunks::<4>().0;
    let brands = core::iter::once(major).chain(compatible.iter());
    // A still image also lists generic brands, so image brands win wherever they are listed:
    // AVIF and JPEG XL over HEVC-coded HEIF, over the bare HEIF structure brands, over video.
    let mut best: Option<(u8, Kind)> = None;
    for kind in brands.filter_map(brand_kind) {
        let rank = brand_rank(kind);
        if best.is_none_or(|(top, _)| rank > top) {
            best = Some((rank, kind));
        }
    }
    best.map(|(_, kind)| kind)
}

fn brand_rank(kind: Kind) -> u8 {
    match kind {
        Kind::Avif | Kind::Jxl => 3,
        Kind::Heif => 2,
        _ => 0,
    }
}

fn brand_kind(brand: &[u8; 4]) -> Option<Kind> {
    Some(match brand {
        b"avif" | b"avis" => Kind::Avif,
        b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" => Kind::Heif,
        // The HEIF structure brands. AVIF files list them too, and AVIF outranks them.
        b"mif1" | b"msf1" => Kind::Heif,
        b"jxl " => Kind::Jxl,
        b"qt  " => Kind::QuickTime,
        b"M4A " | b"M4B " | b"M4P " => Kind::M4a,
        b"3gp4" | b"3gp5" | b"3gp6" | b"3gp7" | b"3gg6" | b"3g2a" | b"3g2b" | b"3g2c" => Kind::ThreeGp,
        b"isom" | b"iso2" | b"iso3" | b"iso4" | b"iso5" | b"iso6" | b"mp41" | b"mp42" | b"avc1" | b"dash" | b"mmp4" | b"M4V " | b"M4VH" | b"M4VP"
        | b"MSNV" | b"f4v " => Kind::Mp4,
        _ => return None,
    })
}

/// Signatures that sit past byte 0 or are made of a few bits, checked last.
fn trailing(b: &[u8]) -> Option<Kind> {
    if let Some(magic) = b.get(257..263)
        && (magic == b"ustar\0" || magic == b"ustar ")
    {
        return Some(Kind::Tar);
    }
    if transport_stream(b) {
        return Some(Kind::MpegTs);
    }
    frame_sync(b)
}

/// Three transport stream packets in a row, each starting with the sync byte.
fn transport_stream(b: &[u8]) -> bool {
    (0..3).all(|n| b.get(n * TS_PACKET).copied() == Some(TS_SYNC))
}

/// An MPEG audio frame header: 11 sync bits, then the layer bits tell MP3 (layer III) from AAC
/// in ADTS (layer 00). Bitrate and sampling indexes must not be the reserved ones.
fn frame_sync(b: &[u8]) -> Option<Kind> {
    let [0xff, second, third, ..] = *b else {
        return None;
    };
    if second & 0xe0 != 0xe0 {
        return None;
    }
    let version = (second >> 3) & 0b11;
    let layer = (second >> 1) & 0b11;
    match layer {
        // ADTS uses only the MPEG-4 (0) and MPEG-2 (1) id bit, so the second byte is 0xFx.
        0b00 if second & 0xf0 == 0xf0 && (third >> 2) & 0x0f < 13 => Some(Kind::Aac),
        0b01 if version != 0b01 && third >> 4 != 0x0f && (third >> 2) & 0b11 != 0b11 => Some(Kind::Mp3),
        _ => None,
    }
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}
