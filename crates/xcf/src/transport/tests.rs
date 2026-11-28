//! Original small XCF/XZ framing fixtures, from the public format descriptions.
use super::*;
use std::io::Write;

fn image() -> Vec<u8> {
    let mut bytes = b"gimp xcf v026\0".to_vec();
    for value in [1_u32, 1, 0, 150] {
        bytes.extend(value.to_be_bytes());
    }
    // End image properties; empty layer/channel/path lists, all 64-bit pointers.
    bytes.extend([0; 32]);
    bytes
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut writer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap()
}

fn crc(bytes: &[u8]) -> [u8; 4] {
    let mut crc = flate2::Crc::new();
    crc.update(bytes);
    crc.sum().to_le_bytes()
}

fn vli(mut value: usize, output: &mut Vec<u8>) {
    loop {
        let byte = (value & 127) as u8;
        value >>= 7;
        output.push(byte | if value != 0 { 128 } else { 0 });
        if value == 0 {
            break;
        }
    }
}

// One uncompressed LZMA2 chunk inside an XZ stream, with no optional data Check.
// CRC32 still protects the header, block header, Index and footer.
fn xz(bytes: &[u8]) -> Vec<u8> {
    assert!(!bytes.is_empty() && bytes.len() <= 65_536);
    let flags = [0, 0];
    let mut output = b"\xfd7zXZ\0".to_vec();
    output.extend(flags);
    output.extend(crc(&flags));
    let block_header = [2, 0, 0x21, 1, 0, 0, 0, 0];
    output.extend(block_header);
    output.extend(crc(&block_header));
    output.push(1);
    output.extend(((bytes.len() - 1) as u16).to_be_bytes());
    output.extend(bytes);
    output.push(0);
    let unpadded = 12 + 4 + bytes.len();
    while !output.len().is_multiple_of(4) {
        output.push(0);
    }
    let mut index = vec![0, 1];
    vli(unpadded, &mut index);
    vli(bytes.len(), &mut index);
    while !index.len().is_multiple_of(4) {
        index.push(0);
    }
    index.extend(crc(&index));
    let backward = (index.len() / 4 - 1) as u32;
    output.extend(index);
    let mut footer = backward.to_le_bytes().to_vec();
    footer.extend(flags);
    output.extend(crc(&footer));
    output.extend(footer);
    output.extend(b"YZ");
    output
}

fn wrappers(bytes: &[u8]) -> Vec<(Wrapper, Vec<u8>)> {
    vec![(Wrapper::Gzip, gzip(bytes)), (Wrapper::Bzip2, crabz2::compress(bytes, crabz2::Level::default())), (Wrapper::Xz, xz(bytes))]
}

fn source<'a>(bytes: &'a [u8], hint: Option<Wrapper>, limits: Limits, keep_going: &'a dyn Fn() -> bool) -> Source<'a> {
    match probe(bytes, hint, limits, keep_going).unwrap() {
        Probe::Xcf(source) => source,
        Probe::Unrelated => panic!("the fixture must be recognized"),
    }
}

#[test]
fn equivalent_wrappers_continue_to_the_same_parse_and_plain_input_is_borrowed() {
    let plain = image();
    let decoded = source(&plain, None, Limits::default(), &|| true).read().unwrap();
    assert!(matches!(decoded, Cow::Borrowed(bytes) if std::ptr::eq(bytes.as_ptr(), plain.as_ptr())));
    for (wrapper, bytes) in wrappers(&plain) {
        let opened = source(&bytes, None, Limits::default(), &|| true);
        assert_eq!(opened.wrapper(), Some(wrapper));
        assert_eq!(opened.prefix(), &plain[..Limits::default().probe_bytes]);
        let decoded = opened.read().unwrap();
        assert_eq!(&*decoded, plain);
        let parsed = crate::Xcf::parse(&decoded).unwrap();
        assert_eq!((parsed.image().width, parsed.image().height), (1, 1));
    }
}

#[test]
fn unrelated_wrappers_are_not_claimed_but_explicit_hints_reject_bad_content() {
    for (wrapper, bytes) in wrappers(b"unrelated drawing bytes, not XCF") {
        assert!(matches!(probe(&bytes, None, Limits::default(), &|| true), Ok(Probe::Unrelated)));
        assert!(matches!(probe(&bytes, Some(wrapper), Limits::default(), &|| true), Err(Error::Invalid(_))));
    }
    assert!(matches!(probe(b"not wrapped", None, Limits::default(), &|| true), Ok(Probe::Unrelated)));
    assert!(matches!(probe(b"not wrapped", Some(Wrapper::Gzip), Limits::default(), &|| true), Err(Error::Invalid(_))));
    let recursive = gzip(&gzip(&image()));
    assert!(matches!(probe(&recursive, Some(Wrapper::Gzip), Limits::default(), &|| true), Err(Error::Invalid(_))));
}

#[test]
fn unrelated_prefix_is_bounded_and_retains_partial_evidence_and_errors() {
    let plain = [b"other format ".as_slice(), &vec![b'x'; CHUNK * 4]].concat();
    let limits = Limits { probe_bytes: 37, ..Limits::default() };
    for (_, bytes) in wrappers(&plain[..1024]) {
        match probe_prefix(&bytes, None, limits, &|| true).unwrap() {
            PrefixProbe::Unrelated { bytes, error } => {
                assert_eq!(&*bytes, &plain[..37]);
                assert!(error.is_none());
            }
            PrefixProbe::Xcf(_) => panic!("unrelated source was claimed"),
        }
    }
    let mut short = gzip(b"<svg/>");
    let crc_offset = short.len() - 8;
    short[crc_offset] ^= 1;
    match probe_prefix(&short, None, limits, &|| true).unwrap() {
        PrefixProbe::Unrelated { bytes, error } => {
            assert_eq!(&*bytes, b"<svg/>");
            assert!(matches!(error, Some(Error::Invalid(_))));
        }
        PrefixProbe::Xcf(_) => panic!("unrelated source was claimed"),
    }
    assert!(matches!(probe_prefix(&short, None, limits, &|| false), Err(Error::Cancelled)));
}

#[test]
fn provisional_prefix_does_not_validate_the_unread_trailer_but_full_read_does() {
    let plain = image();
    for (wrapper, bytes) in wrappers(&plain) {
        for end in bytes.len().saturating_sub(5)..bytes.len() {
            let truncated = &bytes[..end];
            let opened = source(truncated, Some(wrapper), Limits::default(), &|| true);
            assert_eq!(opened.prefix(), &plain[..Limits::default().probe_bytes]);
            assert!(matches!(opened.read(), Err(Error::Invalid(_))), "{wrapper:?}, truncated at {end}");
        }
    }
    let mut damaged = gzip(&plain);
    let crc_offset = damaged.len() - 8;
    damaged[crc_offset] ^= 1;
    assert!(matches!(source(&damaged, None, Limits::default(), &|| true).read(), Err(Error::Invalid(_))));
}

#[test]
fn prefix_and_complete_output_have_independent_exact_limits() {
    let plain = image();
    for (wrapper, bytes) in wrappers(&plain) {
        let limits = Limits { probe_bytes: MAGIC.len(), max_inflated: plain.len(), ..Limits::default() };
        let opened = source(&bytes, Some(wrapper), limits, &|| true);
        assert_eq!(opened.prefix(), MAGIC);
        assert_eq!(&*opened.read().unwrap(), plain);
        let limits = Limits { max_inflated: plain.len() - 1, ..limits };
        assert!(matches!(source(&bytes, Some(wrapper), limits, &|| true).read(), Err(Error::Invalid(_))));
    }
}

#[test]
fn gzip_members_require_checksums_and_only_terminal_zero_padding() {
    let plain = image();
    let first = gzip(&plain[..20]);
    let second = gzip(&plain[20..]);
    let valid = [first.clone(), gzip(&[]), second.clone(), vec![0; CHUNK + 1]].concat();
    assert_eq!(&*source(&valid, None, Limits::default(), &|| true).read().unwrap(), plain);
    let mut damaged = second;
    let crc_offset = damaged.len() - 8;
    damaged[crc_offset] ^= 1;
    let invalid_member = [first, damaged].concat();
    assert!(matches!(
        source(&invalid_member, None, Limits { probe_bytes: MAGIC.len(), ..Limits::default() }, &|| true).read(),
        Err(Error::Invalid(_))
    ));
    for tail in [b"junk".to_vec(), vec![0x1f], vec![0, 0, 1], [vec![0; 8], gzip(&[])].concat()] {
        let bytes = [gzip(&plain), tail].concat();
        assert!(matches!(source(&bytes, None, Limits::default(), &|| true).read(), Err(Error::Invalid(_))));
    }
}

#[test]
fn bzip_members_empty_runs_and_tails_are_bounded_and_verified() {
    let plain = image();
    let pack = |bytes: &[u8]| crabz2::compress(bytes, crabz2::Level::default());
    let members = [pack(&[]).repeat(64), pack(&plain[..20]), pack(&plain[20..])].concat();
    assert_eq!(&*source(&members, None, Limits::default(), &|| true).read().unwrap(), plain);
    let tail = [pack(&plain), b"garbage".to_vec()].concat();
    assert!(matches!(source(&tail, Some(Wrapper::Bzip2), Limits::default(), &|| true).read(), Err(Error::Invalid(_))));
}

#[test]
fn cancellation_in_long_headers_empty_members_and_after_probe_is_sticky() {
    let checks = Cell::new(0);
    let stop_at = Cell::new(8);
    let cancelled_once = || {
        let count = checks.get();
        checks.set(count + 1);
        count != stop_at.get()
    };
    let mut header = gzip(&image());
    header[3] |= 8;
    header.splice(10..10, [vec![b'x'; CHUNK * 16], vec![0]].concat());
    let members = [gzip(&[]).repeat(256), gzip(&image())].concat();
    // Bzip polls between borrowed block-sized windows. Cross the first 900 kB
    // window with empty members, so the cancellation is after real zero-output work.
    let bzip_empty = [crabz2::compress(&[], crabz2::Level::default()).repeat(70_000), crabz2::compress(&image(), crabz2::Level::default())].concat();
    for (bytes, stop) in [(header, 8), (members, 8), (bzip_empty, 3)] {
        checks.set(0);
        stop_at.set(stop);
        assert!(
            matches!(probe(&bytes, None, Limits::default(), &cancelled_once), Err(Error::Cancelled)),
            "stop at poll {stop}, observed {}",
            checks.get()
        );
    }
    let armed = Cell::new(false);
    let keep_going = || !armed.replace(false);
    let bytes = gzip(&image());
    let opened = source(&bytes, None, Limits::default(), &keep_going);
    armed.set(true);
    assert!(matches!(opened.read(), Err(Error::Cancelled)));
}

#[test]
fn xz_dictionary_overhead_indexes_and_terminal_alignment_remain_checked() {
    // Original minimal v26 XCF compressed with liblzma preset9/9e; 64 MiB dictionary.
    let encoded = "fd377a585a000004e6d6b446020021011c00000010cf58cce0003d001b5d00339a49fbe397e4455415f47d24eb590444701acb0ca7bde17c6700000022f43d0d7a807d030001373eb408e0711fb6f37d010000000004595a";
    let bytes: Vec<u8> =
        encoded.as_bytes().as_chunks::<2>().0.iter().map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect();
    assert_eq!(&*source(&bytes, None, Limits::default(), &|| true).read().unwrap(), image());
    let too_small = Limits { max_xz_memory_kib: 64 * 1024, ..Limits::default() };
    assert!(matches!(probe(&bytes, Some(Wrapper::Xz), too_small, &|| true), Err(Error::Invalid(_))));
    let mut tail = xz(&image());
    tail.push(0);
    assert!(matches!(source(&tail, None, Limits::default(), &|| true).read(), Err(Error::Invalid(_))));
    let mut wrong_footer = xz(&image());
    let start = wrong_footer.len() - 12;
    wrong_footer[start + 4] ^= 1;
    let check = crc(&wrong_footer[start + 4..start + 10]);
    wrong_footer[start..start + 4].copy_from_slice(&check);
    assert!(matches!(source(&wrong_footer, None, Limits::default(), &|| true).read(), Err(Error::Invalid(_))));
}

#[test]
fn xz_index_size_mismatch_is_rejected_even_with_a_valid_index_crc() {
    let plain = image();
    let valid = xz(&plain);
    assert_eq!(&*source(&valid, None, Limits::default(), &|| true).read().unwrap(), plain);
    // Both one-block Index sizes fit a single VLI byte in this original fixture.
    let footer = valid.len() - 12;
    let backward = u32::from_le_bytes(valid[footer + 4..footer + 8].try_into().unwrap()) as usize;
    let index = footer - (backward + 1) * 4;
    assert_eq!(&valid[index..index + 2], &[0, 1]);
    assert!(valid[index + 2] < 127 && valid[index + 3] < 127);
    for field in [2, 3] {
        let mut damaged = valid.clone();
        damaged[index + field] += 1;
        let check = crc(&damaged[index..footer - 4]);
        damaged[footer - 4..footer].copy_from_slice(&check);
        let opened = source(&damaged, None, Limits { probe_bytes: MAGIC.len(), ..Limits::default() }, &|| true);
        assert_eq!(opened.prefix(), MAGIC);
        assert!(matches!(opened.read(), Err(Error::Invalid(_))));
    }
}
