use super::*;

fn pat(depth: u16, alpha: bool) -> PsdPattern {
    let (w, h) = (5u32, 3u32);
    let bpp = usize::from(depth / 8);
    let plane = |s: u8| (0..w as usize * h as usize * bpp).map(|i| (i as u8).wrapping_mul(7).wrapping_add(s)).collect::<Vec<u8>>();
    PsdPattern {
        mode: 3,
        width: w,
        height: h,
        name: "Test ✓".into(),
        id: "0bd2d3ba-1234-11d4-8f8f-aabbccddeeff".into(),
        palette: None,
        depth,
        channels: vec![plane(1), plane(2), plane(3)],
        alpha: alpha.then(|| plane(9)),
    }
}

/// Compare direct pattern serialization with staged bytes at supported depths.
#[test]
fn direct_writes_match_staged_bytes() {
    for depth in [1, 8, 16, 32] {
        for alpha in [false, true] {
            let mut p = pat(depth, alpha);
            if depth == 1 {
                p.channels = vec![vec![0xA8; 3]; 3];
                p.alpha = alpha.then(|| vec![0xF8; 3]);
            }
            p.id = "é".repeat(140);
            let mut indexed = p.clone();
            indexed.mode = 2;
            indexed.channels.truncate(1);
            indexed.palette = Some(vec![73; 17]);
            let patterns = [p, indexed];
            let mut block = Vec::new();
            let mut file = b"8BPT\0\x01\0\0\0\x02".to_vec();
            for p in &patterns {
                let mut one = Vec::new();
                staged_pattern(p, &mut one).unwrap();
                block.extend_from_slice(&(one.len() as u32).to_be_bytes());
                block.extend_from_slice(&one);
                block.resize(block.len().next_multiple_of(4), 0);
                file.extend_from_slice(&one);
            }
            assert_eq!(write_pattern_block(&patterns).unwrap(), block);
            assert_eq!(write_pat_file(&patterns).unwrap(), file);
        }
    }
}

/// Verify borrowed raw pattern planes retain layout and length validation.
#[test]
fn raw_pattern_writes_still_validate_layout_and_plane_length() {
    for depth in [1, 16, 32, 7] {
        let p = pat(depth, false);
        assert!(write_pattern_block(&[PsdPattern { channels: vec![vec![0; 2]], ..p.clone() }]).is_err());
        assert!(write_pat_file(&[PsdPattern { channels: Vec::new(), alpha: Some(vec![0; 2]), ..p }]).is_err());
    }
}

/// Verify matching plane lengths cannot bypass unsupported pattern-depth checks.
#[test]
fn writers_reject_unsupported_depth_with_matching_plane_lengths() {
    let p = PsdPattern { depth: 7, channels: vec![vec![0; 15]; 3], alpha: Some(vec![0; 15]), ..pat(8, false) };
    assert!(write_pattern_block(std::slice::from_ref(&p)).is_err());
    assert!(write_pat_file(&[p]).is_err());
}

/// photocraft#9318 review: a name the reader refuses (more than 65,536 UTF-16 units with its NUL) is
/// refused on write, so a saved library can't become a file that loads nothing.
#[test]
fn a_name_the_reader_refuses_is_refused_on_write() {
    let long = PsdPattern { name: "a".repeat(65_536), ..pat(8, false) };
    assert!(matches!(write_pat_file(&[long]), Err(PsdError::LimitExceeded(_))));
    let longest = PsdPattern { name: "a".repeat(65_535), ..pat(8, false) };
    assert_eq!(parse_pat_file(&write_pat_file(std::slice::from_ref(&longest)).unwrap()).unwrap(), [longest]);
}

/// photocraft#9318 review: more patterns than a `.pat` holds are refused on write, and a file claiming
/// more is refused on read rather than silently cut to the first 100,000.
#[test]
fn more_patterns_than_a_pat_file_holds_are_refused() {
    let many = vec![pat(8, false); MAX_PAT_PATTERNS + 1];
    assert!(matches!(write_pat_file(&many), Err(PsdError::LimitExceeded(_))));
    let mut header = b"8BPT".to_vec();
    header.extend_from_slice(&1u16.to_be_bytes());
    header.extend_from_slice(&(MAX_PAT_PATTERNS as u32 + 1).to_be_bytes());
    assert!(matches!(parse_pat_file(&header), Err(PsdError::LimitExceeded(_))));
}

#[test]
fn block_and_pat_file_round_trip_at_every_depth() {
    for depth in [8, 16, 32] {
        for alpha in [false, true] {
            let ps = vec![pat(depth, alpha), PsdPattern { name: "Second".into(), ..pat(depth, false) }];
            let blk = write_pattern_block(&ps).unwrap();
            assert_eq!(blk.len() % 4, 0);
            assert_eq!(parse_pattern_block(&blk).unwrap(), ps);
            let f = write_pat_file(&ps).unwrap();
            assert_eq!(parse_pat_file(&f).unwrap(), ps);
            // Re-serialising is byte-stable.
            assert_eq!(write_pattern_block(&parse_pattern_block(&blk).unwrap()).unwrap(), blk);
        }
    }
}

/// Offsets of the virtual memory arrays of a one-pattern block (each at its "written" flag),
/// with whether each is written, and the end of the array list.
fn vma_slots(block: &[u8], p: &PsdPattern) -> (Vec<(usize, bool)>, usize) {
    let rect = pattern_vma_rect_offset(4, p);
    let end = rect + u32::from_be_bytes(block[rect - 4..rect].try_into().unwrap()) as usize;
    let (mut at, mut slots) = (rect + 20, Vec::new());
    while at < end {
        let written = u32::from_be_bytes(block[at..at + 4].try_into().unwrap()) != 0;
        slots.push((at, written));
        at += 4;
        if written {
            at += 4 + u32::from_be_bytes(block[at..at + 4].try_into().unwrap()) as usize;
        }
    }
    (slots, end)
}

#[test]
fn transparency_goes_in_the_sheet_mask_array_like_photoshop() {
    // photocraft#8054: transparency in the user mask array (slot 24) made Photoshop refuse the file;
    // Photoshop keeps it in the sheet mask array (slot 25).
    for depth in [8, 16, 32] {
        let p = pat(depth, true);
        let mut block = write_pattern_block(std::slice::from_ref(&p)).unwrap();
        let (slots, end) = vma_slots(&block, &p);
        let written: Vec<usize> = slots.iter().enumerate().filter(|(_, s)| s.1).map(|(i, _)| i).collect();
        assert_eq!((slots.len(), written), (26, vec![0, 1, 2, 25]), "{depth}-bit");
        assert!(!block_has_user_mask(&block));
        // The layout older PhotoCraft builds wrote still opens, and is detected.
        block[slots[24].0..end].rotate_left(4);
        let (slots, _) = vma_slots(&block, &p);
        assert!(slots[24].1 && !slots[25].1);
        assert!(block_has_user_mask(&block));
        assert_eq!(parse_pattern_block(&block).unwrap(), vec![p]);
    }
    assert!(!block_has_user_mask(&write_pattern_block(&[pat(8, false)]).unwrap()));
    for junk in [&[][..], &[0, 0, 0, 9, 1], &[0xff; 64]] {
        assert!(!block_has_user_mask(junk));
    }
}

#[test]
fn malformed_input_errors_without_panicking() {
    let blk = write_pattern_block(&[pat(8, true)]).unwrap();
    for cut in [3, 10, 40, blk.len() - 5] {
        let _ = parse_pattern_block(&blk[..cut]);
    }
    assert!(parse_pat_file(b"8BPS\0\x01").is_err());
    assert!(parse_pattern_block(&[]).unwrap().is_empty());
}

fn pattern_vma_rect_offset(pattern_start: usize, p: &PsdPattern) -> usize {
    let name_units = p.name.encode_utf16().count() + 1;
    pattern_start + 16 + name_units * 2 + 1 + p.id.len() + if p.mode == 2 { 768 } else { 0 } + 8
}

fn set_i32(bytes: &mut [u8], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

#[test]
fn hostile_pattern_rectangles_are_rejected_through_all_import_routes() {
    let p = pat(8, false);

    // A subtraction of the hostile endpoints overflowed before rectangle validation.
    let mut global = write_pattern_block(std::slice::from_ref(&p)).unwrap();
    let vma = pattern_vma_rect_offset(4, &p);
    set_i32(&mut global, vma + 4, i32::MIN);
    set_i32(&mut global, vma + 12, i32::MAX);
    assert!(parse_pattern_block(&global).is_err());

    let mut standalone = write_pat_file(std::slice::from_ref(&p)).unwrap();
    let vma = pattern_vma_rect_offset(10, &p);
    set_i32(&mut standalone, vma + 4, i32::MIN);
    set_i32(&mut standalone, vma + 12, i32::MAX);
    assert!(parse_pat_file(&standalone).is_err());

    let mut abr = crate::abr::write_v6(
        1,
        &[crate::abr::AbrSample { id: String::new(), width: 1, height: 1, depth: 8, data: vec![0] }],
        std::slice::from_ref(&p),
        &[],
        true,
    )
    .unwrap();
    let section = abr.windows(4).position(|w| w == b"patt").unwrap();
    let vma = pattern_vma_rect_offset(section + 8, &p) + 4;
    set_i32(&mut abr, vma + 4, i32::MIN);
    set_i32(&mut abr, vma + 12, i32::MAX);
    let parsed = crate::abr::parse(&abr).unwrap();
    assert!(parsed.patterns.is_empty());
    assert!(parsed.warnings.iter().any(|warning| warning.contains("embedded patterns unreadable")));
}

#[test]
fn huge_pattern_rect_with_tiny_channel_payload_is_rejected() {
    // RGB32 needs 10.8 GB; CMYK16 plus alpha needs 9 GB. Both exceed the
    // existing pattern budget, but their first plane fits on 64-bit targets.
    // The complete metadata must be checked before allocating that first plane.
    huge_pattern_rect_is_rejected(pat(32, false));
    let mut cmyk = pat(16, true);
    cmyk.mode = 4;
    cmyk.channels.push(vec![0; 5 * 3 * 2]);
    huge_pattern_rect_is_rejected(cmyk);
}

fn huge_pattern_rect_is_rejected(p: PsdPattern) {
    let mut global = write_pattern_block(std::slice::from_ref(&p)).unwrap();
    let vma = pattern_vma_rect_offset(4, &p);
    set_i32(&mut global, vma + 8, 30_000);
    set_i32(&mut global, vma + 12, 30_000);
    assert!(matches!(parse_pattern_block(&global), Err(PsdError::LimitExceeded(_))));

    let mut standalone = write_pat_file(std::slice::from_ref(&p)).unwrap();
    let vma = pattern_vma_rect_offset(10, &p);
    set_i32(&mut standalone, vma + 8, 30_000);
    set_i32(&mut standalone, vma + 12, 30_000);
    assert!(matches!(parse_pat_file(&standalone), Err(PsdError::LimitExceeded(_))));

    let mut abr = crate::abr::write_v6(
        1,
        &[crate::abr::AbrSample { id: String::new(), width: 1, height: 1, depth: 8, data: vec![0] }],
        std::slice::from_ref(&p),
        &[],
        true,
    )
    .unwrap();
    let section = abr.windows(4).position(|w| w == b"patt").unwrap();
    let vma = pattern_vma_rect_offset(section + 8, &p) + 4;
    set_i32(&mut abr, vma + 8, 30_000);
    set_i32(&mut abr, vma + 12, 30_000);
    let parsed = crate::abr::parse(&abr).unwrap();
    assert!(parsed.patterns.is_empty());
    assert!(parsed.warnings.iter().any(|warning| warning.contains("embedded patterns unreadable")));
}

#[test]
fn offset_channel_rectangles_are_placed_inside_the_pattern() {
    for depth in [8, 16, 32] {
        let original = pat(depth, true);
        let mut block = write_pattern_block(std::slice::from_ref(&original)).unwrap();
        let vma = pattern_vma_rect_offset(4, &original);
        set_i32(&mut block, vma, -2);
        set_i32(&mut block, vma + 4, -3);
        set_i32(&mut block, vma + 8, 4);
        set_i32(&mut block, vma + 12, 5);

        let parsed = parse_pattern_block(&block).unwrap();
        let parsed = parsed.first().unwrap();
        assert_eq!((parsed.width, parsed.height), (8, 6));
        let bytes_per_sample = usize::from(depth / 8);
        for (actual, expected) in parsed.channels.iter().zip(&original.channels).chain(parsed.alpha.iter().zip(original.alpha.iter())) {
            for y in 0..6usize {
                for x in 0..8usize {
                    let at = (y * 8 + x) * bytes_per_sample;
                    let actual_sample = actual.get(at..at + bytes_per_sample).unwrap();
                    if (2..5).contains(&y) && (3..8).contains(&x) {
                        let source_at = ((y - 2) * 5 + (x - 3)) * bytes_per_sample;
                        assert_eq!(actual_sample, expected.get(source_at..source_at + bytes_per_sample).unwrap());
                    } else {
                        assert!(actual_sample.iter().all(|&sample| sample == 0));
                    }
                }
            }
        }
    }
}

#[test]
fn one_bit_offset_channel_placement_preserves_msb_first_pixels() {
    let source = PlaneLayout { planes: 1, width: 5, height: 2, depth: 1, version: Version::Psd };
    let destination = PlaneLayout { planes: 1, width: 8, height: 4, depth: 1, version: Version::Psd };
    let placed = place_channel_plane(vec![0b1010_1000, 0b0101_0000], &source, 2, 1, &destination).unwrap();
    assert_eq!(placed, [0, 0b0010_1010, 0b0001_0100, 0]);
}

#[test]
fn full_size_channels_reuse_the_decoded_allocation() {
    for depth in [1, 8, 16, 32] {
        let layout = PlaneLayout { planes: 1, width: 5, height: 3, depth, version: Version::Psd };
        let plane = vec![0x55; layout.decoded_len().unwrap()];
        let original = plane.as_ptr();
        let placed = place_channel_plane(plane, &layout, 0, 0, &layout).unwrap();
        assert_eq!(placed.as_ptr(), original);
        let expected = if depth == 1 { 0x50 } else { 0x55 };
        assert!(placed.iter().all(|&byte| byte == expected));
    }
}

#[test]
fn large_channel_padding_within_the_existing_budget_is_allowed() {
    let mut p = pat(8, false);
    p.mode = 1;
    let mut block = write_pattern_block(std::slice::from_ref(&p)).unwrap();
    let vma = pattern_vma_rect_offset(4, &p);
    // More than 64 MiB of implicit padding is valid: the fix must not introduce
    // a smaller, padding-specific format limit to avoid the oversized RGB case.
    set_i32(&mut block, vma + 8, 8193);
    set_i32(&mut block, vma + 12, 8193);
    let parsed = parse_pattern_block(&block).unwrap();
    let plane = &parsed[0].channels[0];
    assert_eq!(plane.len(), 8193 * 8193);
    for y in 0..3 {
        assert_eq!(&plane[y * 8193..y * 8193 + 5], &p.channels[0][y * 5..y * 5 + 5]);
    }
    assert_eq!(plane[5], 0);
    assert_eq!(plane[plane.len() - 1], 0);
}

#[test]
fn padded_plane_allocation_failure_is_an_error() {
    // Capacity overflow is deterministic and never asks the OS for huge memory.
    assert!(matches!(zeroed_pattern_plane(usize::MAX), Err(PsdError::OutOfMemory("not enough memory for the padded pattern channel"))));
}

#[test]
fn tiny_channels_in_a_huge_pattern_rect_are_not_zero_padded_without_bound() {
    // 3 channels of 5x3 padded to 30000x30000 would allocate 2.7 GB of zeros.
    let p = pat(8, false);
    let mut global = write_pattern_block(std::slice::from_ref(&p)).unwrap();
    let vma = pattern_vma_rect_offset(4, &p);
    set_i32(&mut global, vma + 8, 30_000);
    set_i32(&mut global, vma + 12, 30_000);
    assert!(matches!(parse_pattern_block(&global), Err(PsdError::LimitExceeded("padded pattern channels exceed MAX_PADDED_PATTERN_BYTES"))));
}

/// A one-channel gray pattern whose 5x3 channel is padded into a `size`x`size` rectangle.
fn padded_gray_block(size: i32) -> Vec<u8> {
    let mut p = pat(8, false);
    p.mode = 1;
    p.channels.truncate(1);
    let mut block = write_pattern_block(std::slice::from_ref(&p)).unwrap();
    let vma = pattern_vma_rect_offset(4, &p);
    set_i32(&mut block, vma + 8, size);
    set_i32(&mut block, vma + 12, size);
    block
}

#[test]
fn failed_pattern_block_keeps_partial_charge_for_the_next_block() {
    let one = padded_gray_block(100);
    let mut malformed = one.clone();
    malformed.extend_from_slice(&100u32.to_be_bytes());
    let mut budget = PatternBudget::with_decoded_limit(15_000);
    assert!(parse_pattern_block_with(&malformed, &mut budget).is_err());
    assert_eq!(budget.decoded_left(), 5_000);
    assert!(parse_pattern_block_with(&one, &mut budget).is_err());
    assert_eq!(budget.decoded_left(), 5_000);
}

#[test]
fn padding_budget_is_shared_by_every_pattern_of_a_parse() {
    // Each pattern pads 100x100 = 10000 bytes; a 15000-byte budget fits one, not two. Before
    // the fix each pattern got a fresh allowance, so N tiny patterns committed N times the cap.
    let one = padded_gray_block(100);
    let mut budget = PatternBudget::with_decoded_limit(15_000);
    assert_eq!(parse_pattern_block_with(&one, &mut budget).unwrap().len(), 1);
    assert_eq!(budget.decoded_left(), 5_000);

    let two = [one.clone(), one.clone()].concat();
    let mut budget = PatternBudget::with_decoded_limit(15_000);
    assert!(matches!(parse_pattern_block_with(&two, &mut budget), Err(PsdError::LimitExceeded(_))));
    // Charges are monotonic: the first pattern's 10000 bytes stay charged even when the second
    // record fails.
    assert_eq!(budget.decoded_left(), 5_000);

    // Two blocks of one document share a budget too.
    let mut budget = PatternBudget::with_decoded_limit(15_000);
    assert!(parse_pattern_block_with(&one, &mut budget).is_ok());
    assert!(parse_pattern_block_with(&one, &mut budget).is_err());
}

#[test]
fn channels_at_different_depths_are_rejected() {
    // The second colour channel claims 8 bits in a 16-bit pattern. It used to be accepted with
    // the pattern's depth taken from the last channel, so the result couldn't be written back.
    let p = pat(16, false);
    let mut block = write_pattern_block(std::slice::from_ref(&p)).unwrap();
    let first = pattern_vma_rect_offset(4, &p) + 20;
    let first_len = u32::from_be_bytes(block[first + 4..first + 8].try_into().unwrap()) as usize;
    let second = first + 8 + first_len;
    block[second + 8..second + 12].copy_from_slice(&8u32.to_be_bytes());
    block[second + 28..second + 30].copy_from_slice(&8u16.to_be_bytes());
    let err = parse_pattern_block(&block).unwrap_err().to_string();
    assert!(err.contains("different depths"), "{err}");
}

#[test]
fn zero_size_pattern_rectangles_are_rejected() {
    let p = pat(8, false);
    let mut block = write_pattern_block(std::slice::from_ref(&p)).unwrap();
    let vma = pattern_vma_rect_offset(4, &p);
    set_i32(&mut block, vma + 8, 0);
    assert!(parse_pattern_block(&block).is_err());
}

/// Serialize a pattern with the previous staged writer as a byte oracle.
fn staged_pattern(p: &PsdPattern, out: &mut Vec<u8>) -> Result<()> {
    if p.width > MAX_EDGE || p.height > MAX_EDGE || p.width > i16::MAX as u32 || p.height > i16::MAX as u32 {
        return Err(PsdError::LimitExceeded("pattern size"));
    }
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&p.mode.to_be_bytes());
    out.extend_from_slice(&(p.height as u16).to_be_bytes());
    out.extend_from_slice(&(p.width as u16).to_be_bytes());
    let units: Vec<u16> = p.name.encode_utf16().chain(std::iter::once(0)).collect();
    // The reader refuses longer names (see `read_pattern`).
    if units.len() > 65_536 {
        return Err(PsdError::LimitExceeded("pattern name length"));
    }
    out.extend_from_slice(&(units.len() as u32).to_be_bytes());
    for u in units {
        out.extend_from_slice(&u.to_be_bytes());
    }
    let id = p.id.as_bytes();
    let idl = id.len().min(255);
    out.push(idl as u8);
    out.extend_from_slice(&id[..idl]);
    if p.mode == 2 {
        let mut pal = p.palette.clone().unwrap_or_default();
        pal.resize(768, 0);
        out.extend_from_slice(&pal);
    }
    let rect = [0i32, 0, p.height as i32, p.width as i32];
    let mut body = Vec::new();
    for v in rect {
        body.extend_from_slice(&v.to_be_bytes());
    }
    const SLOTS: u32 = 24;
    body.extend_from_slice(&SLOTS.to_be_bytes());
    let layout = PlaneLayout { planes: 1, width: p.width as usize, height: p.height as usize, depth: p.depth, version: Version::Psd };
    let array = |plane: &[u8], body: &mut Vec<u8>| -> Result<()> {
        let (comp, data) = if p.depth == 8 {
            (1u8, encode_planes(Compression::Rle, plane, &layout)?)
        } else {
            (0u8, encode_planes(Compression::Raw, plane, &layout)?)
        };
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&((23 + data.len()) as u32).to_be_bytes());
        body.extend_from_slice(&u32::from(p.depth).to_be_bytes());
        for v in rect {
            body.extend_from_slice(&v.to_be_bytes());
        }
        body.extend_from_slice(&p.depth.to_be_bytes());
        body.push(comp);
        body.extend_from_slice(&data);
        Ok(())
    };
    let nc = p.channels.len().min(SLOTS as usize);
    for c in &p.channels[..nc] {
        array(c, &mut body)?;
    }
    for _ in nc..SLOTS as usize {
        body.extend_from_slice(&0u32.to_be_bytes());
    }
    // User mask slot: unused; sheet mask slot: transparency, where Photoshop keeps it (photocraft#8054).
    body.extend_from_slice(&0u32.to_be_bytes());
    match &p.alpha {
        Some(a) => array(a, &mut body)?,
        None => body.extend_from_slice(&0u32.to_be_bytes()),
    }
    out.extend_from_slice(&3u32.to_be_bytes());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(())
}
