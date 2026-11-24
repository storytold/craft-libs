use crate::{Family, HEAD, Kind, identify};

/// `prefix` followed by zeros up to `len` bytes.
fn padded(prefix: &[u8], len: usize) -> Vec<u8> {
    let mut bytes = prefix.to_vec();
    bytes.resize(len.max(prefix.len()), 0);
    bytes
}

/// An ISO BMFF `ftyp` box with the given major and compatible brands.
fn ftyp(major: &[u8; 4], compatible: &[&[u8; 4]]) -> Vec<u8> {
    let size = u32::try_from(16 + compatible.len() * 4).unwrap();
    let mut bytes = size.to_be_bytes().to_vec();
    bytes.extend_from_slice(b"ftyp");
    bytes.extend_from_slice(major);
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    for brand in compatible {
        bytes.extend_from_slice(*brand);
    }
    bytes
}

/// A RIFF or FORM container header of the given form type.
fn riff(chunk: &[u8; 4], form: &[u8; 4]) -> Vec<u8> {
    let mut bytes = chunk.to_vec();
    bytes.extend_from_slice(&[0x24, 0, 0, 0]);
    bytes.extend_from_slice(form);
    bytes
}

/// An Ogg page whose first packet starts with `packet`.
fn ogg(packet: &[u8]) -> Vec<u8> {
    let mut bytes = b"OggS\0\x02".to_vec();
    bytes.resize(26, 0);
    bytes.push(1);
    bytes.push(u8::try_from(packet.len()).unwrap());
    bytes.extend_from_slice(packet);
    bytes
}

/// An EBML header naming `doc_type`.
fn ebml(doc_type: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x1a, 0x45, 0xdf, 0xa3, 0x9f, 0x42, 0x86, 0x81, 0x01, 0x42, 0x82];
    bytes.push(0x80 | u8::try_from(doc_type.len()).unwrap());
    bytes.extend_from_slice(doc_type);
    bytes
}

#[test]
fn names_fixed_prefixes() {
    let cases: &[(&[u8], Kind)] = &[
        (b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR", Kind::Png),
        (b"\xff\xd8\xff\xe0\0\x10JFIF", Kind::Jpeg),
        (b"GIF89a\x01\0\x01\0", Kind::Gif),
        (b"GIF87a", Kind::Gif),
        (b"\xff\x0a\xfa", Kind::Jxl),
        (b"\x00\x00\x00\x0cJXL \r\n\x87\n", Kind::Jxl),
        (b"qoif\0\0\0\x01", Kind::Qoi),
        (b"\x76\x2f\x31\x01\x02\0\0\0", Kind::OpenExr),
        (b"fLaC\0\0\0\x22", Kind::Flac),
        (b"MThd\0\0\0\x06\0\x01", Kind::Midi),
        (b"#!AMR\n", Kind::Amr),
        (b"#!AMR-WB\n", Kind::Amr),
        (b"FLV\x01\x05", Kind::Flv),
        (b"\x00\x00\x01\xba\x44", Kind::MpegPs),
        (b"glTF\x02\0\0\0", Kind::Glb),
        (b"Kaydara FBX Binary  \0\x1a\0", Kind::Fbx),
        (b"ply\nformat ascii 1.0\n", Kind::Ply),
        (b"ply\r\nformat binary_little_endian 1.0\r\n", Kind::Ply),
        (b"PK\x03\x04\x14\0", Kind::Zip),
        (b"PK\x05\x06\0\0\0\0", Kind::Zip),
        (b"7z\xbc\xaf\x27\x1c\0\x04", Kind::SevenZip),
        (b"Rar!\x1a\x07\x00", Kind::Rar),
        (b"Rar!\x1a\x07\x01\x00", Kind::Rar),
        (b"\x1f\x8b\x08\0", Kind::Gzip),
        (b"\xfd7zXZ\x00\x00", Kind::Xz),
        (b"\x28\xb5\x2f\xfd\x24", Kind::Zstd),
        (b"070701000000", Kind::Cpio),
        (b"070707000000", Kind::Cpio),
        (b"\xc7\x71\0\0", Kind::Cpio),
        (b"\x7fELF\x02\x01\x01", Kind::Elf),
        (b"\xcf\xfa\xed\xfe\x07\0\0\x01", Kind::MachO),
        (b"\xfe\xed\xfa\xce", Kind::MachO),
        (b"\xca\xfe\xba\xbf\0\0\0\x02", Kind::MachOFat),
        (b"\0asm\x01\0\0\0", Kind::Wasm),
        (b"dex\n035\0", Kind::Dex),
        (b"SQLite format 3\0\x10\0", Kind::Sqlite),
        (b"bplist00\xd1", Kind::BinaryPlist),
        (b"%PDF-1.7\n", Kind::Pdf),
        (b"ID3\x04\0\0\0\0\0\0", Kind::Mp3),
    ];
    for &(bytes, kind) in cases {
        assert_eq!(identify(bytes), Some(kind), "{bytes:02x?}");
    }
}

#[test]
fn java_class_and_universal_mach_o_share_cafebabe() {
    assert_eq!(identify(b"\xca\xfe\xba\xbe\0\0\0\x02"), Some(Kind::MachOFat));
    assert_eq!(identify(b"\xca\xfe\xba\xbe\0\0\0\x2c"), Some(Kind::MachOFat));
    // JDK 1.1 (45.3) and Java 21 (65.0).
    assert_eq!(identify(b"\xca\xfe\xba\xbe\0\x03\0\x2d"), Some(Kind::JavaClass));
    assert_eq!(identify(b"\xca\xfe\xba\xbe\0\0\0\x41"), Some(Kind::JavaClass));
    assert_eq!(identify(b"\xca\xfe\xba\xbe"), None);
}

#[test]
fn checks_fields_after_short_prefixes() {
    assert_eq!(identify(b"II*\0\x08\0\0\0"), Some(Kind::Tiff));
    assert_eq!(identify(b"MM\0*\0\0\0\x08"), Some(Kind::Tiff));
    assert_eq!(identify(b"II+\0\x08\0\0\0"), Some(Kind::Tiff));
    assert_eq!(identify(b"8BPS\0\x01\0\0"), Some(Kind::Psd));
    assert_eq!(identify(b"8BPS\0\x02\0\0"), Some(Kind::Psd));
    assert_eq!(identify(b"8BPS\0\x03\0\0"), None);
    assert_eq!(identify(b"\0\0\x01\0\x01\0\x10\x10\0\0"), Some(Kind::Ico));
    assert_eq!(identify(b"\0\0\x01\0\0\0\x10\x10\0\0"), None);
    assert_eq!(identify(b"BZh91AY&SY"), Some(Kind::Bzip2));
    assert_eq!(identify(b"BZh0"), None);
}

#[test]
fn tells_pe_from_dos_programs() {
    let mut pe = padded(b"MZ\x90\0", 0x84);
    pe[0x3c] = 0x80;
    pe[0x80..0x84].copy_from_slice(b"PE\0\0");
    assert_eq!(identify(&pe), Some(Kind::Pe));
    // The PE header lies past the bytes given, or isn't there at all.
    pe[0x3c] = 0xf0;
    assert_eq!(identify(&pe), Some(Kind::DosExecutable));
    let mut far = padded(b"MZ", 0x40);
    far[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(identify(&far), Some(Kind::DosExecutable));
    assert_eq!(identify(b"MZ"), Some(Kind::DosExecutable));
}

#[test]
fn bmp_needs_reserved_zeros_and_a_known_dib_header() {
    let mut bmp = padded(b"BM\x36\0\0\0", 54);
    bmp[10] = 54;
    bmp[14] = 40;
    assert_eq!(identify(&bmp), Some(Kind::Bmp));
    bmp[14] = 41;
    assert_eq!(identify(&bmp), None);
    assert_eq!(identify(b"BMW is a car brand, not a bitmap."), None);
}

#[test]
fn names_riff_and_form_containers_by_form_type() {
    assert_eq!(identify(&riff(b"RIFF", b"WAVE")), Some(Kind::Wav));
    assert_eq!(identify(&riff(b"RIFF", b"WEBP")), Some(Kind::WebP));
    assert_eq!(identify(&riff(b"RIFF", b"AVI ")), Some(Kind::Avi));
    assert_eq!(identify(&riff(b"FORM", b"AIFF")), Some(Kind::Aiff));
    assert_eq!(identify(&riff(b"FORM", b"AIFC")), Some(Kind::Aiff));
    assert_eq!(identify(&riff(b"RIFF", b"CDXA")), None);
}

#[test]
fn names_ogg_streams_by_first_packet() {
    assert_eq!(identify(&ogg(b"OpusHead\x01\x02")), Some(Kind::Opus));
    assert_eq!(identify(&ogg(b"\x80theora\x03\x02")), Some(Kind::OggVideo));
    assert_eq!(identify(&ogg(b"\x01vorbis\0\0")), Some(Kind::Ogg));
    // A page cut off before its segment table is still Ogg.
    assert_eq!(identify(&ogg(b"")[..27]), Some(Kind::Ogg));
    assert_eq!(identify(b"OggS\0\x02"), None);
}

#[test]
fn names_ebml_by_doc_type() {
    assert_eq!(identify(&ebml(b"webm")), Some(Kind::WebM));
    assert_eq!(identify(&ebml(b"matroska")), Some(Kind::Matroska));
    // No readable DocType: still Matroska's container.
    assert_eq!(identify(&[0x1a, 0x45, 0xdf, 0xa3, 0x42, 0x82, 0x04]), Some(Kind::Matroska));
    assert_eq!(identify(&[0x1a, 0x45, 0xdf, 0xa3, 0x42, 0x82, 0x84, b'w']), Some(Kind::Matroska));
}

#[test]
fn names_iso_bmff_by_brand() {
    assert_eq!(identify(&ftyp(b"isom", &[b"isom", b"iso2", b"avc1", b"mp41"])), Some(Kind::Mp4));
    assert_eq!(identify(&ftyp(b"mp42", &[])), Some(Kind::Mp4));
    assert_eq!(identify(&ftyp(b"qt  ", &[b"qt  "])), Some(Kind::QuickTime));
    assert_eq!(identify(&ftyp(b"M4A ", &[b"isom"])), Some(Kind::M4a));
    assert_eq!(identify(&ftyp(b"3gp5", &[b"isom"])), Some(Kind::ThreeGp));
    assert_eq!(identify(&ftyp(b"heic", &[b"mif1"])), Some(Kind::Heif));
    assert_eq!(identify(&ftyp(b"jxl ", &[])), Some(Kind::Jxl));
    // A still image lists generic brands too; the image brand wins wherever it is.
    assert_eq!(identify(&ftyp(b"mif1", &[b"avif", b"miaf"])), Some(Kind::Avif));
    assert_eq!(identify(&ftyp(b"isom", &[b"heic"])), Some(Kind::Heif));
    // Unknown brands (a camera's raw container, say) are not guessed at.
    assert_eq!(identify(&ftyp(b"crx ", &[b"isom"])), Some(Kind::Mp4));
    assert_eq!(identify(&ftyp(b"crx ", &[])), None);
    let mut short = ftyp(b"isom", &[]);
    short[3] = 12;
    assert_eq!(identify(&short), None);
}

#[test]
fn names_old_quicktime_atoms() {
    assert_eq!(identify(b"\0\0\0\x08wide\0\0\0\0mdat"), Some(Kind::QuickTime));
    assert_eq!(identify(b"\0\0\x10\0moov"), Some(Kind::QuickTime));
    assert_eq!(identify(b"\0\0\0\x04moov"), None);
    assert_eq!(identify(b"\0\0\0\x10free"), None);
}

#[test]
fn names_tar_by_its_ustar_magic() {
    let mut tar = padded(b"hello.txt", 512);
    tar[257..263].copy_from_slice(b"ustar\0");
    assert_eq!(identify(&tar), Some(Kind::Tar));
    tar[257..263].copy_from_slice(b"ustar ");
    assert_eq!(identify(&tar), Some(Kind::Tar));
    tar[257..263].copy_from_slice(b"ustaR ");
    assert_eq!(identify(&tar), None);
}

#[test]
fn transport_streams_need_three_packets() {
    let mut ts = vec![0u8; 3 * 188];
    for n in 0..3 {
        ts[n * 188] = 0x47;
    }
    assert_eq!(identify(&ts), Some(Kind::MpegTs));
    assert_eq!(identify(&ts[..377]), Some(Kind::MpegTs));
    assert_eq!(identify(&ts[..376]), None);
}

#[test]
fn reads_mpeg_audio_frame_headers() {
    // MPEG-1, MPEG-2 and MPEG-2.5 layer III.
    assert_eq!(identify(b"\xff\xfb\x90\x64"), Some(Kind::Mp3));
    assert_eq!(identify(b"\xff\xf3\x48\xc4"), Some(Kind::Mp3));
    assert_eq!(identify(b"\xff\xe3\x18\xc4"), Some(Kind::Mp3));
    // Reserved version, bad bitrate, reserved sampling rate, layers I and II.
    assert_eq!(identify(b"\xff\xeb\x90\x64"), None);
    assert_eq!(identify(b"\xff\xfb\xf0\x64"), None);
    assert_eq!(identify(b"\xff\xfb\x9c\x64"), None);
    assert_eq!(identify(b"\xff\xff\x90\x64"), None);
    assert_eq!(identify(b"\xff\xfd\x90\x64"), None);
    // ADTS AAC, MPEG-4 and MPEG-2 ids; a reserved sampling index is not AAC.
    assert_eq!(identify(b"\xff\xf1\x50\x80"), Some(Kind::Aac));
    assert_eq!(identify(b"\xff\xf9\x4c\x80"), Some(Kind::Aac));
    assert_eq!(identify(b"\xff\xf1\x7c\x80"), None);
    assert_eq!(identify(b"\xff\x81\x50"), None);
    assert_eq!(identify(b"\xff\xf1"), None);
}

#[test]
fn names_nothing_without_a_signature() {
    for bytes in [&b""[..], b"\0", b"hello, world\n", b"; FBX 7.3.0 project file", b"plywood", b"name,value\n1,2\n", b"<svg>"] {
        assert_eq!(identify(bytes), None, "{bytes:?}");
    }
}

#[test]
fn reads_only_the_head() {
    let mut late = vec![0u8; HEAD + 600];
    late[HEAD + 257..HEAD + 263].copy_from_slice(b"ustar\0");
    assert_eq!(identify(&late), None);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.resize(HEAD * 4, 0xff);
    assert_eq!(identify(&png), Some(Kind::Png));
}

#[test]
fn every_prefix_of_every_sample_is_safe() {
    let mut samples = vec![ftyp(b"mif1", &[b"avif"]), ebml(b"webm"), ogg(b"OpusHead"), riff(b"RIFF", b"WAVE")];
    samples.push(padded(b"MZ", 0x100));
    samples.push((0..=255).collect());
    for sample in &samples {
        for end in 0..=sample.len() {
            let _ = identify(&sample[..end]);
        }
    }
}

#[test]
fn every_kind_has_a_type_extension_and_family() {
    let mut seen = std::collections::HashSet::new();
    for kind in Kind::ALL {
        assert!(seen.insert(kind), "{kind:?} listed twice");
        let mime = kind.mime_type();
        let (top, sub) = mime.split_once('/').unwrap();
        assert!(!top.is_empty() && !sub.is_empty(), "{kind:?}: {mime}");
        assert!(!kind.extension().is_empty() && !kind.extension().starts_with('.'));
        assert_eq!(kind.extension(), kind.extension().to_ascii_lowercase());
        let expected = match top {
            "image" => Family::Image,
            "audio" => Family::Audio,
            "video" => Family::Video,
            "model" => Family::Model,
            _ => kind.family(),
        };
        assert_eq!(kind.family(), expected, "{kind:?}");
    }
}

#[test]
fn media_types_apps_send() {
    assert_eq!(Kind::Png.mime_type(), "image/png");
    assert_eq!(Kind::Jpeg.extension(), "jpg");
    assert_eq!(Kind::Wav.mime_type(), "audio/wav");
    assert_eq!(Kind::Flac.mime_type(), "audio/flac");
    assert_eq!(Kind::Glb.mime_type(), "model/gltf-binary");
    assert_eq!(Kind::Fbx.mime_type(), "model/fbx");
    assert_eq!(Kind::Ply.mime_type(), "model/ply");
    assert_eq!(Kind::Mp4.family(), Family::Video);
    assert_eq!(Kind::Zip.family(), Family::Archive);
    assert_eq!(Kind::Gzip.family(), Family::Compressed);
    assert_eq!(Kind::Elf.family(), Family::Executable);
    assert_eq!(Kind::Sqlite.family(), Family::Database);
    assert_eq!(Kind::Pdf.family(), Family::Document);
}
