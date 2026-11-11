use craft_raster_codecs::{ChannelLayout, CodecError, Image, Limits, SampleType, pnm};

fn decode(b: &[u8]) -> Result<Image, CodecError> {
    pnm::decode(b, &Limits::default())
}

const PAM_GRAY: &[u8] = b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 1\nMAXVAL 255\nENDHDR";
const ENDINGS: &[&[u8]] = &[b"\n", b" \n", b"\t \n", b"\r\n", b" \t\r\n"];

#[test]
fn pam_consumes_the_complete_endhdr_line() {
    for ending in ENDINGS {
        let mut bytes = PAM_GRAY.to_vec();
        bytes.extend_from_slice(ending);
        bytes.push(0x80);
        let img = decode(&bytes).unwrap();
        assert_eq!(img.dimensions(), (1, 1));
        assert_eq!(img.layout(), ChannelLayout::Gray);
        assert_eq!(img.data(), &[0x80], "ENDHDR ending {ending:?}");
    }
}

#[test]
fn pam_preserves_every_possible_first_raster_byte() {
    // In particular, LF, CR, space, tab and '#' must not be skipped as header syntax.
    for ending in ENDINGS {
        for first in 0..=255u8 {
            let mut bytes = PAM_GRAY.to_vec();
            bytes.extend_from_slice(ending);
            bytes.push(first);
            assert_eq!(decode(&bytes).unwrap().data(), &[first], "ENDHDR ending {ending:?}, raster byte {first}");
        }
    }
}

#[test]
fn pam_crlf_header_preserves_u16_samples_and_alpha() {
    for ending in ENDINGS {
        let mut bytes = b"P7\r\nWIDTH 1\r\nHEIGHT 1\r\nDEPTH 2\r\nMAXVAL 65535\r\nTUPLTYPE GRAYSCALE_ALPHA\r\nENDHDR".to_vec();
        bytes.extend_from_slice(ending);
        bytes.extend_from_slice(b" \n#\r");
        let img = decode(&bytes).unwrap();
        assert_eq!(img.layout(), ChannelLayout::GrayA);
        assert_eq!(img.sample_type(), SampleType::U16);
        assert_eq!(img.to_u16_samples().unwrap(), vec![0x200a, 0x230d], "ENDHDR ending {ending:?}");
    }
}

#[test]
fn pam_rejects_unterminated_or_invalid_endhdr_lines() {
    for suffix in [b"".as_slice(), b" ", b"\r", b" \t\r", b" \x80", b" \x80\n\x7f", b"#comment\n\x7f"] {
        let mut bytes = PAM_GRAY.to_vec();
        bytes.extend_from_slice(suffix);
        assert!(matches!(decode(&bytes), Err(CodecError::Malformed { .. })), "ENDHDR suffix {suffix:?}");
    }
}

#[test]
fn pam_truncation_never_uses_header_whitespace_as_pixels() {
    for ending in ENDINGS {
        let mut bytes = PAM_GRAY.to_vec();
        bytes.extend_from_slice(ending);
        bytes.push(0x80);
        for cut in 0..bytes.len() {
            assert!(decode(&bytes[..cut]).is_err(), "ENDHDR ending {ending:?}, truncated at {cut}");
        }
    }
}

#[test]
fn binary_pnm_keeps_its_single_whitespace_separator() {
    for separator in *b" \t\r\n" {
        for first in *b" \t\r\n#" {
            for (header, channels) in [(b"P5\n1 1\n255".as_slice(), 1), (b"P6\n1 1\n255", 3)] {
                let mut bytes = header.to_vec();
                bytes.push(separator);
                bytes.extend(std::iter::repeat_n(first, channels));
                assert_eq!(decode(&bytes).unwrap().data(), vec![first; channels]);
            }
            let mut bytes = b"P4\n8 1".to_vec();
            bytes.extend_from_slice(&[separator, first]);
            let expected: Vec<u8> = (0..8).map(|bit| if first & (0x80 >> bit) == 0 { 255 } else { 0 }).collect();
            assert_eq!(decode(&bytes).unwrap().data(), expected);
        }
    }
}

fn is_limit(r: Result<Image, CodecError>) -> bool {
    matches!(r, Err(CodecError::LimitExceeded(_)))
}

#[test]
fn pnm_bomb_header_rejected() {
    assert!(is_limit(decode(b"P6\n200000 200000\n255\n\0\0\0")));
    assert!(is_limit(decode(b"P7\nWIDTH 99999\nHEIGHT 99999\nDEPTH 4\nMAXVAL 65535\nTUPLTYPE RGB_ALPHA\nENDHDR\n")));
    assert!(is_limit(decode(b"PF\n100000 100000\n-1.0\n")));
}

/// ASCII PGM/PPM once reserved four bytes per declared sample before reading any, and PBM
/// reserved 2^62 bytes before finding the raster missing.
#[test]
fn pnm_header_alone_reserves_nothing_without_limits() {
    let side = 1u32 << 31;
    for f in
        [format!("P1\n{side} {side}\n"), format!("P2\n{side} {side}\n255\n"), format!("P3\n{side} {side}\n255\n"), format!("P4\n{side} {side}\n")]
    {
        assert!(pnm::decode(f.as_bytes(), &Limits::none()).is_err(), "{f:?}");
    }
}

#[test]
fn pnm_header_sizes_that_overflow_usize_are_errors_without_limits() {
    let m = u32::MAX;
    for f in [
        format!("P3\n{m} {m}\n255\n"),
        format!("P6\n{m} {m}\n255\n"),
        format!("P5\n{m} {m}\n65535\n"),
        format!("PF\n{m} {m}\n-1.0\n"),
        format!("P7\nWIDTH {m}\nHEIGHT {m}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n"),
    ] {
        assert!(pnm::decode(f.as_bytes(), &Limits::none()).is_err(), "{f:?}");
    }
}

/// PBM digits need no separator, and the last ASCII sample needs no trailing white space.
#[test]
fn ascii_pnm_with_the_tightest_layout_still_decodes() {
    assert_eq!(decode(b"P1\n3 1\n010").unwrap().data(), &[255, 0, 255]);
    assert_eq!(decode(b"P2\n3 1\n255\n1 2 3").unwrap().data(), &[1, 2, 3]);
    assert_eq!(decode(b"P3\n1 1\n255\n9 8 7").unwrap().data(), &[9, 8, 7]);
}

#[test]
fn pnm_specific_malformations() {
    let cases: &[&[u8]] = &[
        b"P6\n",
        b"P6\n2 2\n",
        b"P6\n2 2\n255",
        b"P6\n2 2\n0\n",
        b"P6\n2 2\n70000\n",
        b"P6\n-2 2\n255\n",
        b"P6\n2 2\n255\n\x01\x02",
        b"P5\n2 2\n65535\n\x00\x00\x00",
        b"P2\n2 2\n10\n1 2 3 11\n",
        b"P1\n2 2\n0 1 2 0\n",
        b"P4\n9 2\n\x00",
        b"P7\nWIDTH 2\nHEIGHT 2\nDEPTH 9\nMAXVAL 255\nENDHDR\n",
        b"P7\nWIDTH 2\nHEIGHT 2\nMAXVAL 255\nENDHDR\n",
        b"P7\nWIDTH 2\nBOGUS 3\nENDHDR\n",
        b"PF\n2 2\n0.0\n",
        b"PF\n2 2\nnan\n",
        b"PF\n2 2\n-1.0\n\0\0\0\0",
        b"P6 4294967295 4294967295 255\n",
    ];
    for c in cases {
        assert!(decode(c).is_err(), "{:?}", String::from_utf8_lossy(c));
    }
}

#[test]
fn pnm_valid_variants() {
    let img = decode(b"P1\n# comment\n3 2\n1 0 1\n0 1 0\n").unwrap();
    assert_eq!(img.data(), &[0, 255, 0, 255, 0, 255]);
    let img = decode(b"P1\n3 2\n101010").unwrap();
    assert_eq!(img.data(), &[0, 255, 0, 255, 0, 255]);
    let img = decode(b"P4\n3 2\n\xA0\x40").unwrap();
    assert_eq!(img.data(), &[0, 255, 0, 255, 0, 255]);
    let img = decode(b"P2\n2 1\n15\n0 15\n").unwrap();
    assert_eq!(img.data(), &[0, 255]);
    let img = decode(b"P2\n2 1\n1023\n0 1023\n").unwrap();
    assert_eq!(img.sample_type(), SampleType::U16);
    assert_eq!(img.to_u16_samples().unwrap(), vec![0, 65535]);
    let img = decode(b"P3\n1 1\n255\n1 2 3\n").unwrap();
    assert_eq!((img.layout(), img.data()), (ChannelLayout::Rgb, &[1u8, 2, 3][..]));
    let img = decode(b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 2\nMAXVAL 255\nTUPLTYPE GRAYSCALE_ALPHA\nENDHDR\n\x10\x20").unwrap();
    assert_eq!((img.layout(), img.data()), (ChannelLayout::GrayA, &[0x10u8, 0x20][..]));
    let mut pfm = b"PF\n1 2\n1.0\n".to_vec(); // big-endian, bottom-up
    for v in [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0] {
        pfm.extend_from_slice(&v.to_be_bytes());
    }
    let img = decode(&pfm).unwrap();
    assert_eq!(img.to_f32_samples().unwrap(), vec![4.0, 5.0, 6.0, 1.0, 2.0, 3.0]);
}

/// `Limits::max_alloc` budgets the decoded pixels, so the decoder must not build a wider scratch
/// copy next to them (it once collected a `u32` per sample). Reads the process's peak RSS, so it
/// runs alone: `cargo test -p craft-raster-codecs --test pnm -- --ignored --test-threads 1`.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "measures the peak RSS of the process; run it alone"]
fn pnm_decode_peak_memory_stays_near_the_output_size() {
    fn peak_rss_bytes() -> Option<u64> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
        let kib: u64 = line.trim_start_matches("VmHWM:").trim().trim_end_matches("kB").trim().parse().ok()?;
        Some(kib * 1024)
    }
    const SIDE: u32 = 4096; // 16 MiB of 8-bit gray output
    let out_bytes = (SIDE * SIDE) as u64;
    let mut file = format!("P5\n{SIDE} {SIDE}\n255\n").into_bytes();
    file.extend((0..out_bytes).map(|i| (i % 251) as u8));
    // Writing 5 resets the peak RSS to the current RSS; skip where the kernel doesn't allow it.
    if std::fs::write("/proc/self/clear_refs", "5").is_err() {
        return;
    }
    let Some(before) = peak_rss_bytes() else { return };
    let img = decode(&file).unwrap();
    let after = peak_rss_bytes().unwrap();
    assert_eq!(img.data().len() as u64, out_bytes);
    assert_eq!(img.data()[300], (300 % 251) as u8);
    let grown = after.saturating_sub(before);
    assert!(grown < 2 * out_bytes, "decoding a {out_bytes} byte image raised peak RSS by {grown} bytes");
}
