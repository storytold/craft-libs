//! Sample entries (`stsd` children) and codec configuration records: parsing and serialization.

use crate::CodecId;
use crate::bitstream::BitReader;
use crate::isobmff::bytes::{BoxBuf, Cur, FourCc, boxes};
use crate::isobmff::error::{Error, Result};

/// One sample description (an entry of `stsd`).
#[derive(Clone, Debug, PartialEq)]
pub struct SampleEntry {
    /// Sample entry format (`avc1`, `mp4a`, `apch`, …).
    pub format: FourCc,
    /// Index into the `dref` of the track's `dinf` box (1-based; usually 1).
    pub data_reference_index: u16,
    #[allow(missing_docs)]
    pub codec: CodecConfig,
    /// Present for visual sample entries.
    pub video: Option<VideoParams>,
    /// Present for audio sample entries.
    pub audio: Option<AudioParams>,
    /// `btrt` box.
    pub bitrate: Option<BitRate>,
}

/// `btrt` box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BitRate {
    /// Decoding buffer size in bytes.
    pub buffer_size: u32,
    /// Maximum bitrate in bits per second.
    pub max_bitrate: u32,
    /// Average bitrate in bits per second.
    pub avg_bitrate: u32,
}

/// Codec configuration decoded from the sample entry.
#[derive(Clone, Debug, PartialEq)]
pub enum CodecConfig {
    #[allow(missing_docs)]
    Avc(AvcConfig),
    #[allow(missing_docs)]
    Hevc(HevcConfig),
    #[allow(missing_docs)]
    Av1(Av1Config),
    #[allow(missing_docs)]
    Vp9(VpcConfig),
    /// Apple ProRes; fourcc is `apch`/`apcn`/`apcs`/`apco`/`ap4h`/`ap4x`.
    ProRes {
        #[allow(missing_docs)]
        fourcc: FourCc,
    },
    /// Motion JPEG (`jpeg`, `mjpa`, `mjpb`).
    Jpeg {
        #[allow(missing_docs)]
        fourcc: FourCc,
    },
    /// Avid DNxHD/DNxHR (`AVdn`, `AVdh`).
    Dnx {
        #[allow(missing_docs)]
        fourcc: FourCc,
    },
    /// Advanced Professional Video (`apv1` + `apvC`).
    Apv(ApvConfig),
    #[allow(missing_docs)]
    Aac(AacConfig),
    /// MPEG-1/2 audio (esds object type 0x69/0x6B, or QuickTime `.mp3` / `.mp2`). Usually layer
    /// III, but these name no layer: ffmpeg stores MP2 as `mp4a` + 0x6B. Each frame header has it,
    /// and [`crate::isobmff::Track::codec_params`] reports the layer the first frame names.
    Mp3,
    #[allow(missing_docs)]
    Pcm(PcmConfig),
    /// ALAC magic cookie (ALACSpecificConfig, 24 bytes, optionally followed by a channel layout).
    Alac {
        #[allow(missing_docs)]
        cookie: Vec<u8>,
    },
    #[allow(missing_docs)]
    Opus(OpusConfig),
    #[allow(missing_docs)]
    Flac(FlacConfig),
    /// `dac3` payload.
    Ac3 {
        #[allow(missing_docs)]
        dac3: Vec<u8>,
    },
    /// `dec3` payload.
    Eac3 {
        #[allow(missing_docs)]
        dec3: Vec<u8>,
    },
    #[allow(missing_docs)]
    Timecode(TimecodeConfig),
    /// Anything else; `raw` is the full sample entry payload (after the 8-byte box header).
    Unknown {
        #[allow(missing_docs)]
        fourcc: FourCc,
        #[allow(missing_docs)]
        raw: Vec<u8>,
    },
}

impl CodecConfig {
    /// The codec this configuration is for. `Mp3` covers every MPEG audio layer: these entries
    /// name none, and each frame header has it (ffmpeg stores MP2 as `mp4a` + 0x6B);
    /// [`crate::isobmff::Track::codec_params`] reads it from the first frame.
    pub fn codec_id(&self) -> CodecId {
        match self {
            CodecConfig::Avc(_) => CodecId::H264,
            CodecConfig::Hevc(_) => CodecId::Hevc,
            CodecConfig::Av1(_) => CodecId::Av1,
            CodecConfig::Vp9(_) => CodecId::Vp9,
            CodecConfig::ProRes { .. } => CodecId::ProRes,
            CodecConfig::Jpeg { .. } => CodecId::Mjpeg,
            CodecConfig::Aac(_) => CodecId::Aac,
            CodecConfig::Mp3 => CodecId::Mp3,
            CodecConfig::Pcm(_) => CodecId::Pcm,
            CodecConfig::Alac { .. } => CodecId::Alac,
            CodecConfig::Opus(_) => CodecId::Opus,
            CodecConfig::Flac(_) => CodecId::Flac,
            CodecConfig::Ac3 { .. } => CodecId::Ac3,
            CodecConfig::Eac3 { .. } => CodecId::Eac3,
            CodecConfig::Dnx { .. } => CodecId::Dnxhd,
            CodecConfig::Apv(_) => CodecId::Apv,
            CodecConfig::Timecode(_) | CodecConfig::Unknown { .. } => CodecId::Unknown,
        }
    }

    /// The codec configuration in Matroska's `CodecPrivate` form (see [`crate::CodecParams`]):
    /// the `OpusHead` built from `dOps`, `fLaC` + the `dfLa` metadata blocks, the other records
    /// as stored.
    pub fn extradata(&self) -> Vec<u8> {
        match self {
            CodecConfig::Avc(c) => c.to_bytes(),
            CodecConfig::Hevc(c) => c.to_bytes(),
            CodecConfig::Av1(c) => c.to_bytes(),
            CodecConfig::Vp9(c) => c.to_bytes(),
            CodecConfig::Apv(c) => c.record_bytes(),
            CodecConfig::Aac(a) => a.asc.clone(),
            CodecConfig::Alac { cookie } => cookie.clone(),
            CodecConfig::Opus(o) => o.opus_head(),
            CodecConfig::Flac(f) => [b"fLaC".as_slice(), &f.metadata_blocks].concat(),
            CodecConfig::Ac3 { dac3 } => dac3.clone(),
            CodecConfig::Eac3 { dec3 } => dec3.clone(),
            _ => Vec::new(),
        }
    }

    /// A short human-readable codec name.
    pub fn name(&self) -> &'static str {
        match self {
            CodecConfig::Avc(_) => "h264",
            CodecConfig::Hevc(_) => "hevc",
            CodecConfig::Av1(_) => "av1",
            CodecConfig::Vp9(_) => "vp9",
            CodecConfig::ProRes { .. } => "prores",
            CodecConfig::Jpeg { .. } => "mjpeg",
            CodecConfig::Dnx { .. } => "dnxhd",
            CodecConfig::Apv(_) => "apv",
            CodecConfig::Aac(_) => "aac",
            CodecConfig::Mp3 => "mp3",
            CodecConfig::Pcm(_) => "pcm",
            CodecConfig::Alac { .. } => "alac",
            CodecConfig::Opus(_) => "opus",
            CodecConfig::Flac(_) => "flac",
            CodecConfig::Ac3 { .. } => "ac3",
            CodecConfig::Eac3 { .. } => "eac3",
            CodecConfig::Timecode(_) => "timecode",
            CodecConfig::Unknown { .. } => "unknown",
        }
    }
}

mod audio;
mod parse;
mod video;
mod write;

pub use audio::{AacConfig, AudioParams, FlacConfig, OpusConfig, PcmConfig, TimecodeConfig, parse_asc};
pub(crate) use parse::parse_sample_entry;

/// What a `logs` box may hold: a non-empty identifier of at most 255 bytes with no NUL.
fn valid_log_identifier(s: &str) -> bool {
    !s.is_empty() && s.len() <= 255 && !s.as_bytes().contains(&0)
}
pub use video::{
    ApvConfig, Av1Config, AvcConfig, CleanAperture, ColorInfo, FieldInfo, HevcConfig, HevcNalArray, MasteringDisplay, VideoParams, VpcConfig,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isobmff::DolbyVisionConfig;

    fn roundtrip(e: &SampleEntry, qt: bool, handler: &[u8; 4]) -> SampleEntry {
        let mut b = BoxBuf::new();
        e.write(&mut b, qt).unwrap();
        read_entry(&b.buf, handler, qt)
    }

    /// The sample entry box at the start of `bytes`, parsed under a `handler` track.
    fn read_entry(bytes: &[u8], handler: &[u8; 4], qt: bool) -> SampleEntry {
        let bx = crate::isobmff::bytes::boxes(bytes).next().unwrap().unwrap();
        parse_sample_entry(bx.kind, bx.payload, FourCc(*handler), qt).unwrap()
    }

    fn avc_config() -> AvcConfig {
        AvcConfig::new(vec![vec![0x67, 0x64, 0x00, 0x1F, 0xAC]], vec![vec![0x68, 0xEB]], 4)
    }

    fn hevc_config() -> HevcConfig {
        HevcConfig {
            general_profile_idc: 1,
            general_profile_compatibility_flags: 0x6000_0000,
            general_constraint_indicator_flags: 0x9000_0000_0000,
            general_level_idc: 93,
            chroma_format_idc: 1,
            bit_depth_luma: 8,
            bit_depth_chroma: 8,
            length_size: 4,
            num_temporal_layers: 1,
            temporal_id_nested: true,
            arrays: vec![
                HevcNalArray { completeness: true, nal_type: 32, nalus: vec![vec![0x40, 1]] },
                HevcNalArray { completeness: true, nal_type: 33, nalus: vec![vec![0x42, 1, 2]] },
                HevcNalArray { completeness: true, nal_type: 34, nalus: vec![vec![0x44, 1]] },
            ],
            ..Default::default()
        }
    }

    fn av1_config() -> Av1Config {
        Av1Config { seq_profile: 1, seq_level_idx_0: 8, high_bitdepth: true, config_obus: vec![1, 2], ..Default::default() }
    }

    /// A visual sample entry of type `format` (64×64, no compressor name, depth 24) with `children`
    /// after its fixed fields, laid out field by field as the writer lays them out.
    fn visual_entry(format: &[u8; 4], children: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut b = BoxBuf::new();
        let m = b.start(format);
        b.zeros(6);
        b.u16(1); // data_reference_index
        b.zeros(16); // version, revision, vendor, temporal and spatial quality
        b.u16(64);
        b.u16(64);
        b.u32(0x0048_0000);
        b.u32(0x0048_0000);
        b.u32(0);
        b.u16(1); // frame count
        b.zeros(32); // compressor name
        b.u16(24);
        b.i16(-1);
        for (kind, payload) in children {
            b.leaf(kind, payload);
        }
        b.end(m);
        b.buf
    }

    /// The types of the child boxes of the visual sample entry at the start of `entry`, in order.
    fn child_kinds(entry: &[u8]) -> Vec<FourCc> {
        // 8-byte box header, then the 78 bytes of fixed visual sample entry fields.
        crate::isobmff::bytes::boxes(&entry[86..]).map(|c| c.unwrap().kind).collect()
    }

    /// A 24-byte Dolby Vision record: `head` (version, profile/level/flags, compatibility ID in
    /// the top 4 bits of byte 4), then zeros.
    fn dv_record(head: [u8; 5]) -> Vec<u8> {
        let mut r = head.to_vec();
        r.resize(24, 0);
        r
    }

    /// `colr` `nclx`: BT.2020, HLG, BT.2020 non-constant luminance, limited range.
    const HLG_NCLX: &[u8] = b"nclx\0\x09\0\x12\0\x09\0";

    /// Entry type, codec configuration box type, its payload, and the configuration it reads as.
    type EntryCodec = (&'static [u8; 4], &'static [u8; 4], Vec<u8>, CodecConfig);

    /// The base layer's codec configuration box for each Dolby Vision entry type.
    fn base_codecs() -> Vec<EntryCodec> {
        let (avc, hevc, av1) = (avc_config(), hevc_config(), av1_config());
        let avc_case = |f| (f, b"avcC", avc.to_bytes(), CodecConfig::Avc(avc.clone()));
        let hevc_case = |f| (f, b"hvcC", hevc.to_bytes(), CodecConfig::Hevc(hevc.clone()));
        vec![
            hevc_case(b"dvh1"),
            hevc_case(b"dvhe"),
            avc_case(b"dva1"),
            avc_case(b"dvav"),
            (b"dav1", b"av1C", av1.to_bytes(), CodecConfig::Av1(av1.clone())),
        ]
    }

    /// Dolby Vision entries carry their base layer's codec configuration: `dvh1` and `dvhe` read
    /// their `hvcC` as HEVC, `dva1` and `dvav` their `avcC` as H.264, `dav1` its `av1C` as AV1.
    /// The entry keeps its own type, whatever the track's handler says.
    #[test]
    fn dolby_vision_entries_read_their_base_codec() {
        for (format, kind, payload, want) in base_codecs() {
            for handler in [b"vide", b"\0\0\0\0"] {
                let e = read_entry(&visual_entry(format, &[(kind, &payload)]), handler, false);
                let ctx = format!("{} under {handler:?}", FourCc(*format));
                assert_eq!(e.format, *format, "{ctx}");
                assert_eq!(e.codec, want, "{ctx}");
                assert_ne!(e.codec.codec_id(), CodecId::Unknown, "{ctx}");
                assert_eq!(e.video.map(|v| (v.width, v.height)), Some((64, 64)), "{ctx}");
            }
        }
    }

    /// A `dvcC`, `dvvC` or `dvwC` box is read under any video entry, plain or Dolby Vision, and
    /// says whether the base layer displays without the Dolby Vision metadata.
    #[test]
    fn dolby_vision_configurations_are_read_under_any_video_entry() {
        let hevc = hevc_config().to_bytes();
        // (entry, configuration box, record head, profile, level, compatibility ID, base layer displayable)
        let cases = [
            (b"hvc1", b"dvvC", [1, 0, 0x10, 0x0D, 0x10], 8, 1, 1, true),
            (b"hvc1", b"dvvC", [1, 0, 0x10, 0x25, 0x40], 8, 4, 4, true),
            (b"hev1", b"dvvC", [1, 0, 0x10, 0x25, 0x40], 8, 4, 4, true),
            (b"dvh1", b"dvvC", [1, 0, 0x10, 0x25, 0x40], 8, 4, 4, true),
            (b"dvhe", b"dvvC", [1, 0, 0x10, 0x0D, 0x10], 8, 1, 1, true),
            (b"dvh1", b"dvcC", [1, 0, 0x0A, 0x25, 0x00], 5, 4, 0, false),
            (b"dvhe", b"dvcC", [1, 0, 0x0A, 0x25, 0x00], 5, 4, 0, false),
            (b"dvh1", b"dvwC", [1, 0, 0x28, 0x25, 0x40], 20, 4, 4, true),
            (b"dvhe", b"dvcC", [1, 0, 0x0E, 0x37, 0x60], 7, 6, 6, true),
        ];
        for (format, dv_box, head, profile, level, compat, compatible) in cases {
            let record = dv_record(head);
            let e = read_entry(&visual_entry(format, &[(b"hvcC", &hevc), (dv_box, &record), (b"colr", HLG_NCLX)]), b"vide", false);
            let ctx = format!("{} + {}", FourCc(*format), FourCc(*dv_box));
            assert_eq!(e.codec, CodecConfig::Hevc(hevc_config()), "{ctx}");
            let v = e.video.unwrap();
            assert_eq!(v.color, Some(ColorInfo::Nclx { primaries: 9, transfer: 18, matrix: 9, full_range: false }), "{ctx}");
            let dv = v.dolby_vision.unwrap();
            assert_eq!((dv.box_kind(), dv.record()), (FourCc(*dv_box), &record[..]), "{ctx}");
            assert_eq!((dv.version_major(), dv.version_minor()), (1, 0), "{ctx}");
            assert_eq!((dv.profile(), dv.level(), dv.bl_signal_compatibility_id()), (profile, level, compat), "{ctx}");
            assert_eq!((dv.rpu_present(), dv.el_present(), dv.bl_present()), (true, profile == 7, true), "{ctx}");
            assert_eq!(dv.base_layer_compatible(), compatible, "{ctx}");
        }
        // Profile 9 (H.264) and profile 10 (AV1) under their own entry types.
        for (format, kind, payload, _) in base_codecs().into_iter().filter(|c| c.1 != b"hvcC") {
            let (dv_box, head, profile) =
                if kind == b"avcC" { (b"dvcC", [1, 0, 0x12, 0x25, 0x20], 9) } else { (b"dvvC", [1, 0, 0x14, 0x25, 0x10], 10) };
            let e = read_entry(&visual_entry(format, &[(kind, &payload), (dv_box, &dv_record(head))]), b"vide", false);
            let dv = e.video.unwrap().dolby_vision.unwrap();
            assert_eq!((dv.profile(), dv.level(), dv.base_layer_compatible()), (profile, 4, true), "{}", FourCc(*format));
        }
        // An entry without a configuration box has none.
        let plain = read_entry(&visual_entry(b"hvc1", &[(b"hvcC", &hevc), (b"colr", HLG_NCLX)]), b"vide", false);
        assert_eq!(plain.video.unwrap().dolby_vision, None);
    }

    /// Read → write → read keeps the entry's type, the configuration box's type, the record byte for
    /// byte (bits after the compatibility ID included) and the order of the children: the whole
    /// entry comes back identical.
    #[test]
    fn dolby_vision_entries_round_trip_byte_for_byte() {
        let mut unknown_bits = vec![1, 0, 0x10, 0x25, 0x4F];
        unknown_bits.extend((0..19).map(|i| 0xA0 | i));
        let records = [
            (b"dvvC", dv_record([1, 0, 0x10, 0x25, 0x40])),
            (b"dvcC", dv_record([1, 0, 0x0A, 0x25, 0x00])),
            (b"dvvC", unknown_bits),
            (b"dvwC", vec![2, 3, 0x28, 0x25, 0x4C, 0xFF]),
        ];
        let mut entries = base_codecs();
        entries.push((b"hvc1", b"hvcC", hevc_config().to_bytes(), CodecConfig::Hevc(hevc_config())));
        for (format, kind, payload, _) in entries {
            for (dv_box, record) in &records {
                let bytes = visual_entry(format, &[(kind, &payload), (dv_box, record), (b"colr", HLG_NCLX)]);
                for qt in [false, true] {
                    let e = read_entry(&bytes, b"vide", qt);
                    assert_eq!(e.video.as_ref().and_then(|v| v.dolby_vision.as_ref()).map(|d| d.record()), Some(&record[..]));
                    let mut out = BoxBuf::new();
                    e.write(&mut out, qt).unwrap();
                    assert_eq!(out.buf, bytes, "{} + {} (qt {qt})", FourCc(*format), FourCc(**dv_box));
                    assert_eq!(read_entry(&out.buf, b"vide", qt), e);
                }
            }
        }
    }

    /// The configuration box goes right after the codec configuration, before the descriptive
    /// boxes, where an iPhone recording has it.
    #[test]
    fn dolby_vision_box_follows_the_codec_configuration() {
        let mut e = SampleEntry::hevc(hevc_config(), 64, 64);
        let v = e.video.as_mut().unwrap();
        v.dolby_vision = DolbyVisionConfig::new(8, 4, true, false, true, 4).ok();
        v.gamma = Some(0x0002_0000);
        v.field_info = Some(FieldInfo { fields: 1, detail: 0 });
        v.color = Some(ColorInfo::Nclx { primaries: 9, transfer: 18, matrix: 9, full_range: false });
        v.pixel_aspect = Some((1, 1));
        let mut b = BoxBuf::new();
        e.write(&mut b, true).unwrap();
        assert_eq!(child_kinds(&b.buf), [*b"hvcC", *b"dvvC", *b"gama", *b"fiel", *b"colr", *b"pasp"].map(FourCc));
        assert_eq!(roundtrip(&e, true, b"vide"), e);
    }

    /// An entry built for a new encode claims no Dolby Vision configuration, and writes none.
    #[test]
    fn new_entries_have_no_dolby_vision_configuration() {
        for e in [SampleEntry::hevc(hevc_config(), 64, 64), SampleEntry::avc(avc_config(), 64, 64), SampleEntry::av1(av1_config(), 64, 64)] {
            assert_eq!(e.video.as_ref().unwrap().dolby_vision, None);
            let mut b = BoxBuf::new();
            e.write(&mut b, false).unwrap();
            assert!(!child_kinds(&b.buf).iter().any(|k| DolbyVisionConfig::BOX_KINDS.contains(k)), "{:?}", e.format);
        }
    }

    /// A truncated, oversized, unknown or repeated configuration box reads as no configuration.
    /// The entry is otherwise read as it would be without it; nothing fails or panics.
    #[test]
    fn malformed_or_duplicate_dolby_vision_boxes_read_as_none() {
        let hevc = hevc_config().to_bytes();
        let good = dv_record([1, 0, 0x10, 0x25, 0x40]);
        let read = |dv: &[(&[u8; 4], &[u8])]| {
            let mut children: Vec<(&[u8; 4], &[u8])> = vec![(b"hvcC", &hevc)];
            children.extend_from_slice(dv);
            children.push((b"colr", HLG_NCLX));
            let e = read_entry(&visual_entry(b"dvh1", &children), b"vide", false);
            assert_eq!(e.codec, CodecConfig::Hevc(hevc_config()));
            let v = e.video.unwrap();
            assert!(v.color.is_some());
            v.dolby_vision
        };
        assert!(read(&[(b"dvvC", &good)]).is_some());
        assert!(read(&[(b"dvvC", &good[..5])]).is_some(), "5 bytes hold every field read");
        for len in [0, 1, 4, 25, 64] {
            let record: Vec<u8> = good.iter().copied().chain(std::iter::repeat(0)).take(len).collect();
            assert_eq!(read(&[(b"dvvC", &record)]), None, "{len}-byte record");
        }
        assert_eq!(read(&[(b"dvxC", &good)]), None);
        assert_eq!(read(&[(b"dvvC", &good), (b"dvvC", &good)]), None);
        assert_eq!(read(&[(b"dvcC", &good), (b"dvvC", &good)]), None);
        assert_eq!(read(&[(b"dvvC", &good[..4]), (b"dvvC", &good)]), None);
        assert_eq!(read(&[(b"dvvC", &good), (b"dvvC", &good[..4])]), None);
        assert_eq!(read(&[(b"dvvC", &good), (b"dvvC", &good), (b"dvwC", &good)]), None);

        // A configuration box whose size runs past the end of the entry ends the child boxes; the
        // ones before it are kept.
        let mut bytes = visual_entry(b"dvh1", &[(b"hvcC", &hevc), (b"colr", HLG_NCLX), (b"dvvC", &good)]);
        let at = bytes.len() - good.len() - 8;
        bytes[at..at + 4].copy_from_slice(&1000u32.to_be_bytes());
        let e = read_entry(&bytes, b"vide", false);
        assert_eq!(e.codec, CodecConfig::Hevc(hevc_config()));
        let v = e.video.unwrap();
        assert!(v.color.is_some());
        assert_eq!(v.dolby_vision, None);

        // A `logs` box is not a second configuration box, nor the reverse: in either order, both
        // are read.
        let logs: (&[u8; 4], &[u8]) = (b"logs", b"vendor.curve");
        for children in [[(b"hvcC", &hevc[..]), logs, (b"dvvC", &good)], [(b"hvcC", &hevc[..]), (b"dvvC", &good), logs]] {
            let v = read_entry(&visual_entry(b"dvh1", &children), b"vide", true).video.unwrap();
            assert_eq!(v.log_transfer_function.as_deref(), Some("vendor.curve"));
            assert_eq!(v.dolby_vision.map(|d| d.profile()), Some(8));
        }
    }

    /// A QuickTime `logs` box keeps its identifier whatever curve it names; the reader does not
    /// guess what an unknown one means.
    #[test]
    fn log_transfer_function_roundtrip() {
        for id in ["com.apple.rec2020.apple-log", "com.apple.rec2020.apple-log2", "another.vendor.unknown-curve"] {
            let mut e = SampleEntry::prores(FourCc(*b"apch"), 64, 64);
            e.video.as_mut().unwrap().log_transfer_function = Some(id.into());
            let back = roundtrip(&e, true, b"vide");
            assert_eq!(back.video.as_ref().unwrap().log_transfer_function.as_deref(), Some(id));
            assert_eq!(back, e);
        }
        // Apple Log is a 35-byte box: the 27 identifier bytes with no terminator.
        let mut e = SampleEntry::prores(FourCc(*b"apch"), 64, 64);
        e.video.as_mut().unwrap().log_transfer_function = Some("com.apple.rec2020.apple-log".into());
        let mut b = BoxBuf::new();
        e.write(&mut b, true).unwrap();
        let logs = [&35u32.to_be_bytes()[..], b"logscom.apple.rec2020.apple-log"].concat();
        assert!(b.buf.windows(logs.len()).any(|w| w == logs));
    }

    /// The writer refuses an identifier the reader would not accept; it is never truncated.
    #[test]
    fn invalid_log_transfer_functions_are_not_written() {
        for id in [String::new(), "a".repeat(256), "com.apple.rec2020.apple-log\0".into()] {
            let mut e = SampleEntry::prores(FourCc(*b"apch"), 64, 64);
            e.video.as_mut().unwrap().log_transfer_function = Some(id);
            assert!(e.write(&mut BoxBuf::new(), true).is_err());
        }
        let mut e = SampleEntry::prores(FourCc(*b"apch"), 64, 64);
        e.video.as_mut().unwrap().log_transfer_function = Some("a".repeat(255));
        assert!(e.write(&mut BoxBuf::new(), true).is_ok());
    }

    /// An empty, oversized, non-UTF-8 or NUL-holding `logs` box, or two of them, reads as unknown.
    #[test]
    fn malformed_or_duplicate_logs_boxes_read_as_unknown() {
        let read = |children: &[&[u8]]| {
            let mut b = BoxBuf::new();
            SampleEntry::prores(FourCc(*b"apch"), 64, 64).write(&mut b, true).unwrap();
            for payload in children {
                b.leaf(b"logs", payload);
            }
            let len = (b.buf.len() as u32).to_be_bytes();
            b.buf[..4].copy_from_slice(&len);
            let bx = crate::isobmff::bytes::boxes(&b.buf).next().unwrap().unwrap();
            parse_sample_entry(bx.kind, bx.payload, FourCc(*b"vide"), true).unwrap().video.unwrap().log_transfer_function
        };
        assert_eq!(read(&[b"vendor.curve"]).as_deref(), Some("vendor.curve"));
        assert_eq!(read(&[b""]), None);
        assert_eq!(read(&[&[b'a'; 256]]), None);
        assert_eq!(read(&[b"bad\xFFutf8"]), None);
        assert_eq!(read(&[b"nul\0inside"]), None);
        assert_eq!(read(&[b"vendor.curve", b"vendor.curve"]), None);
        assert_eq!(read(&[b"vendor.curve", b""]), None);
    }
}
