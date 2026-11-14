//! DPX (SMPTE 268M) read and write, against fixtures packed here independently of the codec:
//! every descriptor, bit depth and packing it reads, both byte orders, the header fields kept
//! through a round trip, the bit-depth choice on export and the files it refuses (#1049).

use craft_image_codecs::*;

const HEADER: usize = 2048;
const ELEMENT0: usize = 780;

/// A DPX file built field by field.
#[derive(Clone)]
struct Fixture {
    big: bool,
    width: u32,
    height: u32,
    descriptor: u8,
    bits: u8,
    packing: u16,
    encoding: u16,
    signed: bool,
    transfer: u8,
    colorimetric: u8,
    orientation: u16,
    elements: u16,
    strings: Vec<((usize, usize), &'static str)>,
    frame_position: u32,
    frame_rate: f32,
    time_code: u32,
    /// Code values, interleaved, top line first.
    codes: Vec<u16>,
}

impl Fixture {
    fn new(descriptor: u8, bits: u8, packing: u16, width: u32, height: u32) -> Self {
        let channels = channels(descriptor);
        let max = (1u32 << bits) - 1;
        // Spread over the whole range, ends included.
        let n = (width * height) as usize * channels;
        let codes = (0..n)
            .map(|i| {
                if i == 0 {
                    0
                } else if i == 1 {
                    max as u16
                } else {
                    ((i as u64 * 2654435761) % (u64::from(max) + 1)) as u16
                }
            })
            .collect();
        Fixture {
            big: true,
            width,
            height,
            descriptor,
            bits,
            packing,
            encoding: 0,
            signed: false,
            transfer: 255,
            colorimetric: 255,
            orientation: 0,
            elements: 1,
            strings: Vec::new(),
            frame_position: u32::MAX,
            frame_rate: f32::from_bits(u32::MAX),
            time_code: u32::MAX,
            codes,
        }
    }

    fn u16(&self, v: u16) -> [u8; 2] {
        if self.big { v.to_be_bytes() } else { v.to_le_bytes() }
    }

    fn u32(&self, v: u32) -> [u8; 4] {
        if self.big { v.to_be_bytes() } else { v.to_le_bytes() }
    }

    fn line(&self, codes: &[u16]) -> Vec<u8> {
        let mut out = Vec::new();
        match (self.bits, self.packing) {
            (8, _) => out.extend(codes.iter().map(|&c| c as u8)),
            (16, _) => out.extend(codes.iter().flat_map(|&c| self.u16(c))),
            (10, 1 | 2) => {
                let base = if self.packing == 1 { 22 } else { 20 };
                for chunk in codes.chunks(3) {
                    let w = chunk.iter().enumerate().fold(0u32, |w, (i, &c)| w | (u32::from(c) << (base - 10 * i as u32)));
                    out.extend(self.u32(w));
                }
            }
            (12, 1) => out.extend(codes.iter().flat_map(|&c| self.u16(c << 4))),
            _ => {
                // One bit stream, high bits first, cut into 32-bit words.
                let mut bits: Vec<bool> = codes.iter().flat_map(|&c| (0..self.bits).rev().map(move |b| (c >> b) & 1 == 1)).collect();
                bits.resize(bits.len().div_ceil(32) * 32, false);
                for word in bits.chunks(32) {
                    out.extend(self.u32(word.iter().fold(0u32, |w, &b| (w << 1) | u32::from(b))));
                }
            }
        }
        out.resize(out.len().div_ceil(4) * 4, 0);
        out
    }

    fn bytes(&self) -> Vec<u8> {
        let mut b = vec![0u8; HEADER];
        let mut put = |at: usize, v: &[u8]| b[at..at + v.len()].copy_from_slice(v);
        put(0, if self.big { b"SDPX" } else { b"XPDS" });
        put(4, &self.u32(HEADER as u32));
        put(8, b"V2.0");
        put(24, &self.u32(1664));
        put(28, &self.u32(384));
        put(768, &self.u16(self.orientation));
        put(770, &self.u16(self.elements));
        put(772, &self.u32(self.width));
        put(776, &self.u32(self.height));
        for el in 0..usize::from(self.elements.min(8)) {
            let e = ELEMENT0 + el * 72;
            put(e, &self.u32(u32::from(self.signed)));
            put(e + 20, &[self.descriptor, self.transfer, self.colorimetric, self.bits]);
            put(e + 24, &self.u16(self.packing));
            put(e + 26, &self.u16(self.encoding));
            put(e + 28, &self.u32(HEADER as u32));
        }
        for &((at, len), s) in &self.strings {
            assert!(s.len() < len);
            put(at, s.as_bytes());
        }
        put(1712, &self.u32(self.frame_position));
        put(1724, &self.u32(self.frame_rate.to_bits()));
        put(1920, &self.u32(self.time_code));
        let per_line = self.width as usize * channels(self.descriptor);
        for line in self.codes.chunks(per_line) {
            b.extend(self.line(line));
        }
        b
    }
}

fn channels(descriptor: u8) -> usize {
    match descriptor {
        6 => 1,
        50 => 3,
        _ => 4,
    }
}

fn layout(descriptor: u8) -> ChannelLayout {
    match descriptor {
        6 => ChannelLayout::Gray,
        50 => ChannelLayout::Rgb,
        _ => ChannelLayout::Rgba,
    }
}

/// The 16-bit sample a code value opens as: `round(c × 65535 / (2^b − 1))`.
fn expected(c: u16, bits: u8) -> u16 {
    (f64::from(c) * 65535.0 / f64::from((1u32 << bits) - 1)).round() as u16
}

/// Reads the code values of a big-endian file we wrote (packing 0 at 8/16 bits, 1 at 10/12).
fn written_codes(b: &[u8]) -> (u8, u16, Vec<u16>) {
    let be16 = |at: usize| u16::from_be_bytes([b[at], b[at + 1]]);
    let be32 = |at: usize| u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    let (w, h) = (be32(772) as usize, be32(776) as usize);
    let nc = channels(b[ELEMENT0 + 20]);
    let (bits, packing) = (b[ELEMENT0 + 23], be16(ELEMENT0 + 24));
    let off = be32(ELEMENT0 + 28) as usize;
    let per_line = w * nc;
    let stride = match bits {
        10 => per_line.div_ceil(3) * 4,
        _ => (per_line * if bits == 8 { 1 } else { 2 }).div_ceil(4) * 4,
    };
    let mut codes = Vec::new();
    for y in 0..h {
        let line = &b[off + y * stride..];
        for i in 0..per_line {
            codes.push(match bits {
                8 => u16::from(line[i]),
                10 => ((be32_at(line, i / 3 * 4) >> (22 - 10 * (i % 3) as u32)) & 0x3FF) as u16,
                12 => u16::from_be_bytes([line[i * 2], line[i * 2 + 1]]) >> 4,
                _ => u16::from_be_bytes([line[i * 2], line[i * 2 + 1]]),
            });
        }
    }
    (bits, packing, codes)
}

fn be32_at(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn opts(bits: Option<DpxBits>) -> EncodeOptions {
    EncodeOptions { dpx_bits: bits, ..EncodeOptions::default() }
}

const COMBOS: [(u8, u16); 7] = [(8, 0), (10, 0), (10, 1), (10, 2), (12, 0), (12, 1), (16, 0)];

#[test]
fn every_descriptor_depth_and_packing_opens_with_its_model_and_values() {
    for descriptor in [6, 50, 51] {
        for (bits, packing) in COMBOS {
            // Widths that leave partly filled words at the end of a line.
            let fx = Fixture::new(descriptor, bits, packing, 5, 3);
            let img = decode(&fx.bytes()).unwrap_or_else(|e| panic!("{descriptor} {bits}/{packing}: {e}"));
            let what = format!("descriptor {descriptor}, {bits} bits, packing {packing}");
            assert_eq!(detect(&fx.bytes()), Some(Format::Dpx));
            assert_eq!((img.width(), img.height()), (5, 3), "{what}");
            assert_eq!(img.layout(), layout(descriptor), "{what}");
            assert_eq!(img.layout().has_alpha(), descriptor == 51, "{what}");
            match bits {
                8 => assert_eq!(img.as_u8().unwrap(), fx.codes.iter().map(|&c| c as u8).collect::<Vec<_>>(), "{what}"),
                16 => assert_eq!(img.to_u16_samples().unwrap(), fx.codes, "{what}"),
                _ => assert_eq!(img.to_u16_samples().unwrap(), fx.codes.iter().map(|&c| expected(c, bits)).collect::<Vec<_>>(), "{what}"),
            }
        }
    }
}

#[test]
fn big_and_little_endian_files_open_identically() {
    for (bits, packing) in COMBOS {
        let be = Fixture::new(50, bits, packing, 4, 2);
        let le = Fixture { big: false, ..be.clone() };
        assert_eq!(&le.bytes()[..4], b"XPDS");
        assert_eq!(decode(&be.bytes()).unwrap(), decode(&le.bytes()).unwrap(), "{bits} bits, packing {packing}");
    }
}

#[test]
fn ten_bit_code_value_opens_as_the_rounded_16_bit_value() {
    let mut fx = Fixture::new(6, 10, 1, 1, 1);
    fx.codes = vec![512];
    assert_eq!(decode(&fx.bytes()).unwrap().to_u16_samples().unwrap(), vec![(512.0f64 * 65535.0 / 1023.0).round() as u16]);
}

#[test]
fn ten_and_twelve_bit_frames_written_back_at_their_depth_keep_their_code_values() {
    for (bits, choice) in [(10, DpxBits::Bits10), (12, DpxBits::Bits12)] {
        for packing in [0, 1] {
            let fx = Fixture::new(50, bits, packing, 7, 3);
            let out = encode(&decode(&fx.bytes()).unwrap(), Format::Dpx, &opts(Some(choice))).unwrap();
            let (wbits, wpacking, codes) = written_codes(&out);
            assert_eq!((wbits, wpacking), (bits, 1));
            assert_eq!(codes, fx.codes, "{bits} bits, packing {packing}");
        }
    }
}

#[test]
fn sixteen_bit_frames_round_trip_losslessly_and_files_start_with_sdpx() {
    let fx = Fixture { big: false, ..Fixture::new(51, 16, 0, 6, 4) };
    let img = decode(&fx.bytes()).unwrap();
    let out = encode(&img, Format::Dpx, &EncodeOptions::default()).unwrap();
    assert_eq!(&out[..4], b"SDPX");
    let (bits, packing, codes) = written_codes(&out);
    assert_eq!((bits, packing, codes), (16, 0, fx.codes));
    assert_eq!(decode(&out).unwrap().to_u16_samples(), img.to_u16_samples());
}

#[test]
fn log_frames_report_their_encoding_and_keep_the_plain_mapping() {
    let mut fx = Fixture::new(50, 10, 1, 3, 2);
    (fx.transfer, fx.colorimetric) = (3, 1);
    let img = decode(&fx.bytes()).unwrap();
    assert_eq!(img.warnings, vec![DecodeWarning::DpxEncoding { transfer: 3, colorimetric: 1 }]);
    let note = img.warnings[0].to_string();
    assert!(note.contains("logarithmic (3)") && note.contains("printing density (1)"), "{note}");
    assert_eq!(img.to_u16_samples().unwrap(), fx.codes.iter().map(|&c| expected(c, 10)).collect::<Vec<_>>());
}

#[test]
fn only_the_first_of_several_image_elements_is_imported() {
    let mut fx = Fixture::new(6, 8, 0, 2, 2);
    fx.elements = 2;
    let img = decode(&fx.bytes()).unwrap();
    assert_eq!(img.warnings.iter().map(ToString::to_string).collect::<Vec<_>>(), ["only the first of 2 image elements was imported"]);
}

#[test]
fn listed_header_fields_survive_a_round_trip() {
    let mut fx = Fixture::new(50, 10, 1, 2, 2);
    fx.strings = vec![((160, 100), "Resolve 20.1"), ((260, 200), "Reel 7"), ((460, 200), "(c) 2026 Studio"), ((136, 24), "2026:10:12:09:30:00UTC")];
    (fx.transfer, fx.colorimetric, fx.frame_position, fx.frame_rate, fx.time_code) = (3, 1, 86400, 23.976, 0x0102_0304);
    let img = decode(&fx.bytes()).unwrap();
    for (k, v) in [
        ("DPX:Creator", "Resolve 20.1"),
        ("DPX:Project", "Reel 7"),
        ("DPX:Copyright", "(c) 2026 Studio"),
        ("DPX:CreationTime", "2026:10:12:09:30:00UTC"),
        ("DPX:Transfer", "3"),
        ("DPX:Colorimetric", "1"),
        ("DPX:FramePosition", "86400"),
        ("DPX:FrameRate", "23.976"),
        ("DPX:TimeCode", "01:02:03:04"),
    ] {
        assert!(img.meta.text.contains(&(k.into(), v.into())), "{k} = {v} in {:?}", img.meta.text);
    }
    let out = encode(&img, Format::Dpx, &opts(Some(DpxBits::Bits10))).unwrap();
    let src = fx.bytes();
    // Field by field: the strings, the element's encoding codes, film and TV fields.
    for (at, len) in [(160, 100), (260, 200), (460, 200), (136, 24), (ELEMENT0 + 21, 2), (1712, 4), (1724, 4), (1920, 4)] {
        assert_eq!(out[at..at + len], src[at..at + len], "field at {at}");
    }
    assert_eq!(decode(&out).unwrap().meta.text, img.meta.text);
}

#[test]
fn chosen_bit_depth_sets_the_header_and_packing() {
    let img = Image::from_u16(5, 2, ChannelLayout::Rgb, &(0..30u16).map(|i| i * 2000).collect::<Vec<_>>()).unwrap();
    for (choice, bits, packing) in [(DpxBits::Bits8, 8, 0), (DpxBits::Bits10, 10, 1), (DpxBits::Bits12, 12, 1), (DpxBits::Bits16, 16, 0)] {
        let out = encode(&img, Format::Dpx, &opts(Some(choice))).unwrap();
        assert_eq!((out[ELEMENT0 + 23], u16::from_be_bytes([out[ELEMENT0 + 24], out[ELEMENT0 + 25]])), (bits, packing), "{choice:?}");
        let back = decode(&out).unwrap();
        assert_eq!((back.width(), back.height(), back.layout()), (5, 2, ChannelLayout::Rgb));
    }
    // Without a choice the depth follows the image.
    let gray8 = Image::from_u8(3, 1, ChannelLayout::Gray, vec![0, 128, 255]).unwrap();
    assert_eq!(encode(&gray8, Format::Dpx, &EncodeOptions::default()).unwrap()[ELEMENT0 + 23], 8);
    assert_eq!(encode(&img, Format::Dpx, &EncodeOptions::default()).unwrap()[ELEMENT0 + 23], 16);
    let float = img.convert(ChannelLayout::Rgb, SampleType::F32);
    assert_eq!(encode(&float, Format::Dpx, &EncodeOptions::default()).unwrap()[ELEMENT0 + 23], 16);
    assert!(fidelity_warnings(&float, Format::Dpx).contains(&FidelityWarning::DepthReduced { from: SampleType::F32, to: SampleType::U16 }));
}

/// The depth warnings of `img` written as `format` with `o`.
fn depth_warnings(img: &Image, format: Format, o: &EncodeOptions) -> Vec<FidelityWarning> {
    fidelity_warnings_with(img, format, o)
        .into_iter()
        .filter(|w| matches!(w, FidelityWarning::DepthReduced { .. } | FidelityWarning::BitsReduced { .. }))
        .collect()
}

#[test]
fn saving_sixteen_bits_at_ten_or_twelve_warns_with_the_stored_bits() {
    let img = Image::from_u16(5, 2, ChannelLayout::Rgb, &(0..30u16).map(|i| i * 2000 + 1).collect::<Vec<_>>()).unwrap();
    for (choice, bits) in [(DpxBits::Bits10, 10), (DpxBits::Bits12, 12)] {
        assert_eq!(depth_warnings(&img, Format::Dpx, &opts(Some(choice))), [FidelityWarning::BitsReduced { from: SampleType::U16, bits }]);
        assert_eq!(FidelityWarning::BitsReduced { from: SampleType::U16, bits }.to_string(), format!("16-bit will be reduced to {bits}-bit"));
        // Float samples name the stored bits too, not the 16-bit words they are packed in.
        let float = img.convert(ChannelLayout::Rgb, SampleType::F32);
        assert_eq!(depth_warnings(&float, Format::Dpx, &opts(Some(choice))), [FidelityWarning::BitsReduced { from: SampleType::F32, bits }]);
        // 8-bit samples widen into 10 and 12 bits without loss.
        let eight = img.convert(ChannelLayout::Rgb, SampleType::U8);
        assert_eq!(depth_warnings(&eight, Format::Dpx, &opts(Some(choice))), []);
    }
    assert_eq!(depth_warnings(&img, Format::Dpx, &opts(Some(DpxBits::Bits16))), []);
    assert_eq!(depth_warnings(&img, Format::Dpx, &opts(None)), []);
    assert_eq!(
        depth_warnings(&img, Format::Dpx, &opts(Some(DpxBits::Bits8))),
        [FidelityWarning::DepthReduced { from: SampleType::U16, to: SampleType::U8 }]
    );
    assert_eq!(depth_warnings(&img, Format::Cineon, &EncodeOptions::default()), [FidelityWarning::BitsReduced { from: SampleType::U16, bits: 10 }]);
}

#[test]
fn a_frame_opened_at_ten_or_twelve_bits_saves_back_without_a_warning() {
    let img = Image::from_u16(5, 2, ChannelLayout::Rgb, &(0..30u16).map(|i| i * 2000 + 1).collect::<Vec<_>>()).unwrap();
    for (choice, bits) in [(DpxBits::Bits10, 10), (DpxBits::Bits12, 12)] {
        let opened = decode(&encode(&img, Format::Dpx, &opts(Some(choice))).unwrap()).unwrap();
        assert_eq!(depth_warnings(&opened, Format::Dpx, &opts(Some(choice))), [], "{bits}-bit");
        assert_eq!(decode(&encode(&opened, Format::Dpx, &opts(Some(choice))).unwrap()).unwrap(), opened, "{bits}-bit");
    }
    // A 12-bit frame saved at 10 bits does lose its two low bits.
    let twelve = decode(&encode(&img, Format::Dpx, &opts(Some(DpxBits::Bits12))).unwrap()).unwrap();
    assert_eq!(
        depth_warnings(&twelve, Format::Dpx, &opts(Some(DpxBits::Bits10))),
        [FidelityWarning::BitsReduced { from: SampleType::U16, bits: 10 }]
    );
    let cineon = decode(&cineon_bytes(3, 4, 2, &cineon_codes(24))).unwrap();
    assert_eq!(depth_warnings(&cineon, Format::Cineon, &EncodeOptions::default()), []);
    assert_eq!(depth_warnings(&cineon, Format::Dpx, &opts(Some(DpxBits::Bits10))), []);
}

#[test]
fn gray_with_alpha_is_written_as_luma_with_a_warning() {
    let img = Image::from_u8(2, 1, ChannelLayout::GrayA, vec![10, 128, 200, 255]).unwrap();
    let out = encode(&img, Format::Dpx, &EncodeOptions::default()).unwrap();
    assert_eq!(out[ELEMENT0 + 20], 6);
    assert_eq!(decode(&out).unwrap().as_u8().unwrap(), &[10, 200]);
    assert!(fidelity_warnings(&img, Format::Dpx).contains(&FidelityWarning::AlphaDiscarded));
}

#[test]
fn only_free_form_text_is_reported_as_dropped() {
    let mut img = Image::from_u8(1, 1, ChannelLayout::Rgb, vec![1, 2, 3]).unwrap();
    img.meta.text = vec![("DPX:Project".into(), "Reel 7".into())];
    assert!(!fidelity_warnings(&img, Format::Dpx).contains(&FidelityWarning::TextDropped));
    img.meta.text.push(("Comment".into(), "from a PNG".into()));
    assert!(fidelity_warnings(&img, Format::Dpx).contains(&FidelityWarning::TextDropped));
}

#[test]
fn stored_orientation_is_turned_upright() {
    // Orientation 4: the stored lines are the image's columns, left to right.
    let mut fx = Fixture::new(6, 8, 0, 3, 1);
    fx.codes = vec![1, 2, 3];
    fx.orientation = 4;
    let img = decode(&fx.bytes()).unwrap();
    assert_eq!((img.width(), img.height(), img.as_u8().unwrap()), (1, 3, &[1u8, 2, 3][..]));
}

#[test]
fn unsupported_and_damaged_files_error_instead_of_panicking() {
    let unsupported = |bytes: Vec<u8>, what: &str| {
        assert!(matches!(decode(&bytes), Err(CodecError::Unsupported { format: Format::Dpx, .. })), "{what}");
    };
    unsupported(Fixture { descriptor: 100, ..Fixture::new(50, 8, 0, 2, 2) }.bytes(), "ABGR descriptor");
    unsupported(Fixture { encoding: 1, ..Fixture::new(50, 10, 1, 2, 2) }.bytes(), "run-length encoding");
    unsupported(Fixture { signed: true, ..Fixture::new(50, 16, 0, 2, 2) }.bytes(), "signed samples");
    let mut float = Fixture::new(50, 16, 0, 2, 2).bytes();
    float[ELEMENT0 + 23] = 32;
    unsupported(float, "32-bit float");
    let mut method_b = Fixture::new(50, 12, 1, 2, 2).bytes();
    method_b[ELEMENT0 + 24..ELEMENT0 + 26].copy_from_slice(&2u16.to_be_bytes());
    unsupported(method_b, "12-bit method B");

    let good = Fixture::new(51, 10, 1, 9, 9).bytes();
    let mut no_elements = good.clone();
    no_elements[770..772].copy_from_slice(&0u16.to_be_bytes());
    let mut past_end = good.clone();
    past_end[ELEMENT0 + 28..ELEMENT0 + 32].copy_from_slice(&u32::MAX.wrapping_sub(1).to_be_bytes());
    let mut huge = good.clone();
    huge[772..780].copy_from_slice(&[0xFF; 8]);
    for (what, bytes) in [
        ("header cut short", good[..900].to_vec()),
        ("truncated element", good[..good.len() - 1].to_vec()),
        ("no image elements", no_elements),
        ("data offset past the end", past_end),
        ("absurd size", huge),
        ("magic alone", b"SDPX".to_vec()),
    ] {
        assert!(decode_as(Format::Dpx, &bytes).is_err(), "{what}");
    }
}

// ---------------------------------------------------------------- Cineon (Kodak .cin)

/// A Cineon 4.5 file built field by field: the 1024-byte generic header, a 1024-byte film header,
/// then `codes` (interleaved, top line first) three 10-bit values to a big-endian word, left
/// justified, each line on a word boundary.
fn cineon_bytes(channels: u8, width: u32, height: u32, codes: &[u16]) -> Vec<u8> {
    let mut b = vec![0u8; 2048];
    b[0..4].copy_from_slice(&0x802A_5FD7u32.to_be_bytes());
    b[4..8].copy_from_slice(&2048u32.to_be_bytes());
    b[193] = channels;
    b[198] = 10;
    b[200..204].copy_from_slice(&width.to_be_bytes());
    b[204..208].copy_from_slice(&height.to_be_bytes());
    for line in codes.chunks(width as usize * usize::from(channels)) {
        for chunk in line.chunks(3) {
            let w = chunk.iter().enumerate().fold(0u32, |w, (i, &c)| w | (u32::from(c) << (22 - 10 * i as u32)));
            b.extend(w.to_be_bytes());
        }
    }
    b
}

/// Code values spread over 0..=1023, ends included.
fn cineon_codes(n: usize) -> Vec<u16> {
    (0..n).map(|i| if i == 1 { 1023 } else { ((i as u64 * 2654435761) % 1024) as u16 }).collect()
}

/// Reads back what we wrote: (channels, width, height, code values), from the header's own fields.
fn cineon_written(b: &[u8]) -> (u8, u32, u32, Vec<u16>) {
    let off = be32_at(b, 4) as usize;
    let (nc, w, h) = (b[193], be32_at(b, 200), be32_at(b, 204));
    let per_line = w as usize * usize::from(nc);
    let stride = per_line.div_ceil(3) * 4;
    let mut codes = Vec::new();
    for y in 0..h as usize {
        let line = &b[off + y * stride..];
        codes.extend((0..per_line).map(|i| ((be32_at(line, i / 3 * 4) >> (22 - 10 * (i % 3) as u32)) & 0x3FF) as u16));
    }
    (nc, w, h, codes)
}

#[test]
fn cineon_frames_saved_as_cineon_keep_their_code_values_and_reopen_identically() {
    // Widths that leave partly filled words at the end of each line.
    for (channels, layout) in [(1u8, ChannelLayout::Gray), (3, ChannelLayout::Rgb)] {
        let codes = cineon_codes(5 * 3 * usize::from(channels));
        let img = decode(&cineon_bytes(channels, 5, 3, &codes)).unwrap();
        assert_eq!(img.layout(), layout);
        let out = encode(&img, Format::Cineon, &EncodeOptions::default()).unwrap_or_else(|e| panic!("{layout:?}: {e}"));
        assert_eq!(detect(&out), Some(Format::Cineon), "{layout:?}");
        assert_eq!(cineon_written(&out), (channels, 5, 3, codes), "{layout:?}");
        assert_eq!(decode(&out).unwrap(), img, "{layout:?}");
    }
}

#[test]
fn cineon_header_follows_the_kodak_layout() {
    let img = decode(&cineon_bytes(3, 4, 2, &cineon_codes(24))).unwrap();
    let out = encode(&img, Format::Cineon, &EncodeOptions::default()).unwrap();
    let f32_at = |at: usize| f32::from_bits(be32_at(&out, at));
    // Generic and film header lengths, user data, total size, version.
    assert_eq!((be32_at(&out, 4), be32_at(&out, 8), be32_at(&out, 12), be32_at(&out, 16)), (2048, 1024, 1024, 0));
    assert_eq!(be32_at(&out, 20) as usize, out.len());
    assert_eq!(&out[24..29], b"V4.5\0");
    // Upright, three channels, each the universal metric's red, green, blue at 10 bits.
    assert_eq!((out[192], out[193]), (0, 3));
    for (i, colour) in [1u8, 2, 3].into_iter().enumerate() {
        let c = 196 + 28 * i;
        assert_eq!(&out[c..c + 3], &[0, colour, 10], "channel {i}");
        assert_eq!((be32_at(&out, c + 4), be32_at(&out, c + 8)), (4, 2), "channel {i}");
        // Minimum and maximum data value and quantity, all four 32-bit floats.
        assert_eq!((f32_at(c + 12), f32_at(c + 16), f32_at(c + 20), f32_at(c + 24)), (0.0, 0.0, 1023.0, 2.048), "channel {i}");
    }
    // Pixel interleave, 32-bit words left justified, unsigned, positive, no padding.
    assert_eq!(&out[680..692], &[0, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    // Two lines of twelve samples: four words each.
    assert_eq!(out.len(), 2048 + 2 * 16);
}

#[test]
fn any_image_saves_as_ten_bit_cineon() {
    // 16-bit samples land on the nearest 10-bit code; colour with alpha drops it, gray stays gray.
    let wide = Image::from_u16(3, 1, ChannelLayout::Rgba, &[0, 32, 65535, 65535, 30000, 40000, 50000, 0, 1, 2, 3, 4]).unwrap();
    let out = encode(&wide, Format::Cineon, &EncodeOptions::default()).unwrap();
    let to_code = |v: u16| (f64::from(v) * 1023.0 / 65535.0).round() as u16;
    let want: Vec<u16> = [0u16, 32, 65535, 30000, 40000, 50000, 1, 2, 3].into_iter().map(to_code).collect();
    assert_eq!(cineon_written(&out), (3, 3, 1, want));
    assert!(fidelity_warnings(&wide, Format::Cineon).contains(&FidelityWarning::AlphaDiscarded));
    let gray8 = Image::from_u8(2, 1, ChannelLayout::Gray, vec![0, 255]).unwrap();
    assert_eq!(cineon_written(&encode(&gray8, Format::Cineon, &EncodeOptions::default()).unwrap()), (1, 2, 1, vec![0, 1023]));
}
