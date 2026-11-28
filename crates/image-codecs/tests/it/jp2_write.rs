//! Lossless JPEG 2000 writing, read back through the crate's own JPEG 2000 decoder.

use crate::common;
use common::*;
use craft_image_codecs::*;

const LAYOUTS: [ChannelLayout; 4] = [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba];

fn options(codestream: bool) -> EncodeOptions {
    EncodeOptions { jp2_codestream: codestream, ..Default::default() }
}

/// The codestream inside `bytes`, from its SOC and SIZ markers.
fn codestream(bytes: &[u8]) -> &[u8] {
    let at = bytes.windows(4).position(|w| w == [0xFF, 0x4F, 0xFF, 0x51]).expect("a codestream");
    &bytes[at..]
}

/// The boxes at one level of a JP2 file: type and payload.
fn boxes(data: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    let mut at = 0;
    std::iter::from_fn(move || {
        let size = u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?) as usize;
        let item = (data.get(at + 4..at + 8)?, data.get(at + 8..at + size)?);
        at += size;
        Some(item)
    })
}

/// The payload of the first `kind` box inside the JP2 header box.
fn header_box<'a>(bytes: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    let (_, header) = boxes(bytes).find(|(ty, _)| *ty == b"jp2h")?;
    boxes(header).find(|(ty, _)| ty == kind).map(|(_, data)| data)
}

/// A profile header the decoder reads as `space` ("RGB " or "GRAY").
fn profile(space: &[u8; 4]) -> Vec<u8> {
    let mut icc = sample_icc(200);
    icc[16..20].copy_from_slice(space);
    icc
}

#[test]
fn raw_codestream_round_trips_losslessly_at_8_and_16_bits() {
    for layout in LAYOUTS {
        for (sample, bits) in [(SampleType::U8, 8u8), (SampleType::U16, 16)] {
            let what = format!("{layout:?} {sample:?}");
            let img = synth(37, 23, layout, sample, 3, 0.4);
            let bytes = encode(&img, Format::Jp2, &options(true)).unwrap_or_else(|e| panic!("{what}: {e}"));

            assert_eq!(detect(&bytes), Some(Format::Jp2), "{what}");
            assert!(bytes.starts_with(&[0xFF, 0x4F]), "{what}");
            // SIZ: the component count, then each component's depth less one (unsigned)
            let siz = codestream(&bytes);
            assert_eq!(usize::from(u16::from_be_bytes([siz[40], siz[41]])), layout.channels(), "{what}");
            for component in 0..layout.channels() {
                assert_eq!(siz[42 + 3 * component], bits - 1, "{what} component {component}");
            }

            let back = decode(&bytes).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!((back.layout(), back.sample_type()), (layout, sample), "{what}");
            assert!(back.data() == img.data(), "{what}: max diff {}", max_abs_diff(&img, &back));
        }
    }
}

#[test]
fn jp2_declares_the_colour_space_and_alpha_and_a_raw_codestream_declares_neither() {
    for layout in LAYOUTS {
        let img = synth(9, 7, layout, SampleType::U8, 1, 0.2);
        let jp2 = encode(&img, Format::Jp2, &options(false)).unwrap();

        // Enumerated colour: 16 is sRGB, 17 grayscale
        let colr = header_box(&jp2, b"colr").unwrap_or_else(|| panic!("{layout:?}"));
        let space = if layout.is_gray() { 17u32 } else { 16 };
        assert_eq!((colr[0], &colr[3..7]), (1, &space.to_be_bytes()[..]), "{layout:?}");

        // Alpha: the last channel is opacity for the whole image (type 1, association 0)
        let cdef = header_box(&jp2, b"cdef");
        if layout.has_alpha() {
            let cdef = cdef.unwrap_or_else(|| panic!("{layout:?}"));
            let entries: Vec<[u16; 3]> =
                cdef[2..].as_chunks::<6>().0.iter().map(|&[a, b, c, d, e, f]| [[a, b], [c, d], [e, f]].map(u16::from_be_bytes)).collect();
            let last = layout.channels() as u16 - 1;
            assert_eq!(entries.last(), Some(&[last, 1, 0]), "{layout:?}");
            assert!(entries[..entries.len() - 1].iter().all(|&[_, kind, _]| kind == 0), "{layout:?} colour channels");
        } else {
            assert!(cdef.is_none(), "{layout:?}");
        }

        let raw = encode(&img, Format::Jp2, &options(true)).unwrap();
        assert!(!raw.windows(4).any(|w| w == b"colr" || w == b"jp2h"), "{layout:?}");
    }
}

#[test]
fn jp2_embeds_the_images_icc_profile_and_a_raw_codestream_reports_dropping_it() {
    for (layout, space) in [(ChannelLayout::Rgb, b"RGB "), (ChannelLayout::GrayA, b"GRAY")] {
        let mut img = synth(9, 7, layout, SampleType::U16, 2, 0.2);
        img.icc = Some(profile(space));

        let jp2 = encode(&img, Format::Jp2, &options(false)).unwrap();
        assert_eq!(decode(&jp2).unwrap().icc, img.icc, "{layout:?}");
        assert!(!fidelity_warnings_with(&img, Format::Jp2, &options(false)).contains(&FidelityWarning::IccDropped), "{layout:?}");

        let without = EncodeOptions { embed_icc: false, ..options(false) };
        assert_eq!(decode(&encode(&img, Format::Jp2, &without).unwrap()).unwrap().icc, None, "{layout:?}");

        let raw = encode(&img, Format::Jp2, &options(true)).unwrap();
        assert_eq!(decode(&raw).unwrap().icc, None, "{layout:?}");
        assert!(fidelity_warnings_with(&img, Format::Jp2, &options(true)).contains(&FidelityWarning::IccDropped), "{layout:?}");
    }
}

#[test]
fn a_16_bit_jp2_too_large_for_16_bit_samples_is_read_at_8_bits() {
    let img = synth(37, 23, ChannelLayout::Rgb, SampleType::U16, 4, 0.3);
    let bytes = encode(&img, Format::Jp2, &options(false)).unwrap();
    let decode_within = |max_alloc: u64| decode_with(&bytes, &DecodeOptions { limits: Limits { max_alloc, ..Limits::none() }, ..Default::default() });

    let (mut refused, mut decoded) = (0u64, 1u64 << 30);
    assert_eq!(decode_within(decoded).unwrap().sample_type(), SampleType::U16);
    while decoded - refused > 1 {
        let budget = refused + (decoded - refused) / 2;
        if decode_within(budget).is_ok() { decoded = budget } else { refused = budget }
    }

    assert_eq!(decode_within(decoded).unwrap().sample_type(), SampleType::U8, "the tightest budget that decodes");
}

#[test]
fn a_12_bit_codestream_is_read_widened_to_the_full_16_bit_range() {
    let (width, height) = (19u32, 11u32);
    let max = (1u32 << 12) - 1;
    // Spread over the 12-bit range, from 0 up to the maximum
    let mut values: Vec<u16> = (0..width * height).map(|i| (i * 37 % (max + 1)) as u16).collect();
    *values.last_mut().unwrap() = max as u16;
    let data: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    let samples = j2k::J2kLosslessSamples { data: &data, width, height, components: 1, bit_depth: 12, signed: false };
    let bytes = j2k::encode_j2k_lossless(samples, &j2k::J2kLosslessEncodeOptions::default()).unwrap().codestream;

    let back = decode(&bytes).unwrap();

    assert_eq!((back.layout(), back.sample_type()), (ChannelLayout::Gray, SampleType::U16));
    let widened: Vec<u16> = values.iter().map(|&v| ((u32::from(v) * 65535 + max / 2) / max) as u16).collect();
    assert_eq!(back.to_u16_samples().unwrap(), widened);
}
