//! Metadata-boundary regressions ported from FilmCraft #1932 (nuclearspike).
use super::*;
fn header(kind: u8, w: u16, h: u16, depth: u8, descriptor: u8) -> Vec<u8> {
    let mut b = vec![0, 0, kind, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    b.extend_from_slice(&w.to_le_bytes());
    b.extend_from_slice(&h.to_le_bytes());
    b.extend_from_slice(&[depth, descriptor]);
    b
}
fn dec(bytes: &[u8]) -> Result<Picture> {
    decode(bytes, &Limits::default())
}
#[test]
fn legacy_pixels_that_end_in_signature_are_not_a_footer_before_header() {
    for prefix in [0usize, 7] {
        let width = (prefix + SIGNATURE.len()) as u16;
        let mut file = header(3, width, 1, 8, 0x20);
        file.extend(vec![0; prefix]);
        file.extend_from_slice(SIGNATURE);
        let image = dec(&file).unwrap();
        assert_eq!((image.width, image.height), (u32::from(width), 1));
        let pixels: Vec<u8> = std::iter::repeat_n(0, prefix).chain(SIGNATURE.iter().copied()).flat_map(|gray| [gray, gray, gray, 255]).collect();
        assert_eq!(image.rgba, pixels);
        let source = dec(&file).unwrap();
        assert_eq!(source.rgba, pixels);
    }
}

#[test]
fn short_extension_followed_by_a_developer_payload_keeps_the_pixels() {
    // a 482-byte extension whose size field still says 495, then a developer payload whose
    // first bytes land where a full extension keeps its colour-correction offset, then the
    // directory that points back at that payload
    let mut file = header(2, 1, 1, 24, 0x20);
    file.extend_from_slice(&[10, 20, 30]);
    let extension = file.len() as u32;
    let mut area = vec![0; 482];
    area[..2].copy_from_slice(&495u16.to_le_bytes());
    file.extend_from_slice(&area);
    let payload = file.len() as u32;
    file.extend_from_slice(&18u32.to_le_bytes());
    file.extend_from_slice(&[0; 14]);
    let directory = file.len() as u32;
    file.extend_from_slice(&1u16.to_le_bytes());
    file.extend_from_slice(&0u16.to_le_bytes());
    file.extend_from_slice(&payload.to_le_bytes());
    file.extend_from_slice(&18u32.to_le_bytes());
    file.extend_from_slice(&extension.to_le_bytes());
    file.extend_from_slice(&directory.to_le_bytes());
    file.extend_from_slice(SIGNATURE);
    let source = dec(&file).expect("the image decodes");
    assert_eq!((source.width, source.height), (1, 1));
    assert_eq!(source.rgba, vec![30, 20, 10, 255]);
}

#[test]
fn a_footer_without_a_directory_does_not_read_the_header_as_one() {
    // no developer directory (offset 0). Read as one, the header's three-byte image ID
    // length is an entry count, and the third "entry" is the first words of a colour
    // correction table that sits right after a short pixel area; the extension points at
    // that table, so the image is incomplete and must stay so.
    let mut file = header(2, 2, 1, 24, 0x20);
    file[0] = 3;
    file.extend_from_slice(&[7, 8, 9]);
    file.extend_from_slice(&[0, 0, 255]);
    let table = file.len() as u32;
    let mut words = vec![0; 2048];
    words[..4].copy_from_slice(&2074u32.to_le_bytes());
    words[4..8].copy_from_slice(&1u32.to_le_bytes());
    file.extend_from_slice(&words);
    let extension = file.len() as u32;
    let mut area = vec![0; 495];
    area[..2].copy_from_slice(&495u16.to_le_bytes());
    area[482..486].copy_from_slice(&table.to_le_bytes());
    file.extend_from_slice(&area);
    file.extend_from_slice(&extension.to_le_bytes());
    file.extend_from_slice(&0u32.to_le_bytes());
    file.extend_from_slice(SIGNATURE);
    let decoded = dec(&file);
    assert!(decoded.is_err(), "the second pixel is missing");
}

#[test]
fn short_extension_followed_by_a_developer_directory_keeps_the_pixels() {
    // a complete 1x1 image, a 482-byte extension (its size says so) and right after it a
    // developer directory whose entry's length lands where a full extension keeps its
    // scan-line table offset; reading it as one would move the image end into the header
    let mut file = header(2, 1, 1, 24, 0x20);
    file.extend_from_slice(&[10, 20, 30]);
    let extension = file.len() as u32;
    let mut area = vec![0; 482];
    area[..2].copy_from_slice(&482u16.to_le_bytes());
    file.extend_from_slice(&area);
    let directory = file.len() as u32;
    let payload = directory + 2 + 10;
    file.extend_from_slice(&1u16.to_le_bytes());
    file.extend_from_slice(&0u16.to_le_bytes());
    file.extend_from_slice(&payload.to_le_bytes());
    file.extend_from_slice(&18u32.to_le_bytes());
    file.extend_from_slice(&[0; 18]);
    file.extend_from_slice(&extension.to_le_bytes());
    file.extend_from_slice(&directory.to_le_bytes());
    file.extend_from_slice(SIGNATURE);
    let source = dec(&file).expect("the image decodes");
    assert_eq!((source.width, source.height), (1, 1));
}

#[test]
fn importing_truncated_pixels_does_not_consume_tga_footer() {
    for (kind, data) in [(2, vec![0, 0, 255]), (10, vec![0x80, 0, 0, 255])] {
        for metadata in [None, Some(0usize), Some(1)] {
            let mut file = header(kind, 2, 1, 24, 0x20);
            file.extend_from_slice(&data); // only the first of two pixels
            let mut offsets = [0u8; 8];
            if let Some(field) = metadata {
                let offset = file.len() as u32;
                offsets[field * 4..field * 4 + 4].copy_from_slice(&offset.to_le_bytes());
                let mut area = vec![0; 495];
                if field == 0 {
                    area[..2].copy_from_slice(&495u16.to_le_bytes());
                }
                file.extend_from_slice(&area); // optional metadata is not pixel data
            }
            file.extend_from_slice(&offsets);
            file.extend_from_slice(SIGNATURE);
            let result = dec(&file);
            assert!(result.is_err(), "kind {kind}, metadata {metadata:?}: metadata is not a second pixel");
        }
    }
    for kind in [1, 9] {
        let mut file = header(kind, 1, 1, 8, 0x20);
        file[1] = 1;
        file[5] = 1;
        file[7] = 24;
        file.extend_from_slice(&[0, 0]); // missing third palette byte and the index
        file.extend_from_slice(&[0; 8]);
        file.extend_from_slice(SIGNATURE);
        let result = dec(&file);
        assert!(result.is_err(), "kind {kind}: the footer is not colour-map data");
    }
}

#[test]
fn complete_tga_footer_and_metadata_do_not_change_pixels() {
    for metadata in [None, Some(0usize), Some(1)] {
        let mut file = header(2, 1, 1, 24, 0x20);
        file.extend_from_slice(&[0, 0, 255]);
        let mut offsets = [0u8; 8];
        if let Some(field) = metadata {
            let offset = file.len() as u32;
            offsets[field * 4..field * 4 + 4].copy_from_slice(&offset.to_le_bytes());
            let mut area = vec![0; 495];
            if field == 0 {
                area[..2].copy_from_slice(&495u16.to_le_bytes());
            }
            file.extend_from_slice(&area);
        }
        file.extend_from_slice(&offsets);
        file.extend_from_slice(SIGNATURE);
        let source = dec(&file).unwrap();
        assert_eq!(source.rgba, [255, 0, 0, 255]);
    }
}

fn preceding_metadata(complete: bool, metadata: usize) -> Vec<u8> {
    assert!(metadata <= 3, "test metadata selector must be 0..=3");
    let mut file = header(2, if complete { 1 } else { 2 }, 1, 24, 0x20);
    file.extend_from_slice(&[0, 0, 255]);
    let payload_offset = file.len() as u32;
    let mut offsets = [0u8; 8];
    if metadata == 0 {
        file.extend_from_slice(&[0, 255, 0]);
        let directory_offset = file.len() as u32;
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&payload_offset.to_le_bytes());
        file.extend_from_slice(&3u32.to_le_bytes());
        offsets[4..].copy_from_slice(&directory_offset.to_le_bytes());
    } else {
        match metadata {
            1 => file.extend_from_slice(&[0; 2048]),
            2 => file.extend_from_slice(&[1, 1, 0, 255, 0]),
            3 => file.extend_from_slice(&18u32.to_le_bytes()),
            _ => {}
        }
        let extension_offset = file.len() as u32;
        let mut extension = [0; 495];
        extension[..2].copy_from_slice(&495u16.to_le_bytes());
        let field = [0, 482, 486, 490][metadata];
        extension[field..field + 4].copy_from_slice(&payload_offset.to_le_bytes());
        file.extend_from_slice(&extension);
        offsets[..4].copy_from_slice(&extension_offset.to_le_bytes());
    }
    file.extend_from_slice(&offsets);
    file.extend_from_slice(SIGNATURE);
    file
}

#[test]
fn preceding_metadata_cannot_supply_missing_pixels() {
    let mut accepted = Vec::new();
    for metadata in 0..4 {
        let result = dec(&preceding_metadata(false, metadata));
        if result.is_ok() {
            accepted.push(metadata);
        }
    }
    assert!(accepted.is_empty(), "metadata payloads supplied missing pixels: {accepted:?}");
}

#[test]
fn complete_pixels_before_metadata_keep_their_colour() {
    for metadata in 0..4 {
        let source = dec(&preceding_metadata(true, metadata)).unwrap();
        assert_eq!(source.rgba, [255, 0, 0, 255], "metadata {metadata}");
    }
    let mut file = preceding_metadata(true, 1);
    let footer = file.len() - SIGNATURE.len() - 8;
    let extension = u32::from_le_bytes(file[footer..footer + 4].try_into().unwrap()) as usize;
    file[extension..extension + 2].copy_from_slice(&496u16.to_le_bytes());
    file.insert(footer, 0);
    let source = dec(&file).unwrap();
    assert_eq!(source.rgba, [255, 0, 0, 255], "larger complete extensions preserve pixels");
}

#[test]
fn malformed_optional_metadata_keeps_complete_pixels_readable() {
    let accept = |file: Vec<u8>| {
        let source = dec(&file).unwrap();
        assert_eq!(source.rgba, [255, 0, 0, 255]);
    };
    for field in [0, 4] {
        let mut file = preceding_metadata(true, 0);
        let footer = file.len() - SIGNATURE.len() - 8;
        file[footer + field..footer + field + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        accept(file);
    }
    for metadata in 1..4 {
        let mut file = preceding_metadata(true, metadata);
        let footer = file.len() - SIGNATURE.len() - 8;
        let extension = u32::from_le_bytes(file[footer..footer + 4].try_into().unwrap()) as usize;
        let field = [0, 482, 486, 490][metadata];
        file[extension + field..extension + field + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        accept(file);
    }
    let mut file = preceding_metadata(true, 0);
    let footer = file.len() - SIGNATURE.len() - 8;
    let directory = u32::from_le_bytes(file[footer + 4..footer + 8].try_into().unwrap()) as usize;
    file.drain(directory + 2..footer);
    accept(file);
    let mut file = preceding_metadata(true, 0);
    let footer = file.len() - SIGNATURE.len() - 8;
    let directory = u32::from_le_bytes(file[footer + 4..footer + 8].try_into().unwrap()) as usize;
    file[directory + 8..directory + 12].copy_from_slice(&u32::MAX.to_le_bytes());
    accept(file);
    let mut file = preceding_metadata(true, 0);
    let footer = file.len() - SIGNATURE.len() - 8;
    let directory = u32::from_le_bytes(file[footer + 4..footer + 8].try_into().unwrap()) as usize;
    file[directory..directory + 2].copy_from_slice(&2u16.to_le_bytes());
    accept(file);
    // A zero-length entry cannot mark a payload boundary, even if its offset overlaps pixels.
    let mut file = preceding_metadata(true, 0);
    let footer = file.len() - SIGNATURE.len() - 8;
    let directory = u32::from_le_bytes(file[footer + 4..footer + 8].try_into().unwrap()) as usize;
    file[directory + 4..directory + 8].copy_from_slice(&19u32.to_le_bytes());
    file[directory + 8..directory + 12].copy_from_slice(&0u32.to_le_bytes());
    accept(file);
    let mut file = preceding_metadata(true, 2);
    file[21..23].copy_from_slice(&[255, 255]);
    accept(file);
    let mut file = preceding_metadata(true, 2);
    let footer = file.len() - SIGNATURE.len() - 8;
    let extension = u32::from_le_bytes(file[footer..footer + 4].try_into().unwrap()) as usize;
    file[extension..extension + 2].copy_from_slice(&494u16.to_le_bytes());
    accept(file);
    let mut file = preceding_metadata(true, 2);
    let footer = file.len() - SIGNATURE.len() - 8;
    file.remove(footer - 1); // extension is physically short, but its postage offset remains
    accept(file);
}

#[test]
fn malformed_metadata_offsets_inside_id_or_palette_keep_complete_pixels() {
    let mut file = header(1, 1, 1, 8, 0x20);
    file[0] = 4; // image ID length
    file[1] = 1; // colour map present
    file[5..7].copy_from_slice(&1u16.to_le_bytes());
    file[7] = 24; // one BGR palette entry
    file.extend_from_slice(&[7, 8, 9, 10]); // image ID
    file.extend_from_slice(&[0, 0, 255]); // red palette entry
    file.push(0); // palette index
    for (field, offset) in [(0, HEADER + 1), (4, HEADER + 4 + 1)] {
        let mut candidate = file.clone();
        let mut footer_offsets = [0u8; 8];
        footer_offsets[field..field + 4].copy_from_slice(&(offset as u32).to_le_bytes());
        candidate.extend_from_slice(&footer_offsets);
        candidate.extend_from_slice(SIGNATURE);
        let image = dec(&candidate).unwrap();
        assert_eq!(image.rgba, [255, 0, 0, 255], "footer field {field}");
    }
}

#[test]
fn malformed_optional_metadata_cannot_supply_missing_pixels() {
    let mut file = preceding_metadata(false, 0);
    let footer = file.len() - SIGNATURE.len() - 8;
    let directory = u32::from_le_bytes(file[footer + 4..footer + 8].try_into().unwrap()) as usize;
    file[directory + 8..directory + 12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(dec(&file).is_err());

    // The declared entry count exceeds the bytes before the footer, but the first entry's
    // payload still precedes the directory and cannot be used as a second pixel.
    let mut file = preceding_metadata(false, 0);
    let footer = file.len() - SIGNATURE.len() - 8;
    let directory = u32::from_le_bytes(file[footer + 4..footer + 8].try_into().unwrap()) as usize;
    file[directory..directory + 2].copy_from_slice(&2u16.to_le_bytes());
    assert!(dec(&file).is_err());

    let mut file = preceding_metadata(false, 2);
    let footer = file.len() - SIGNATURE.len() - 8;
    let extension = u32::from_le_bytes(file[footer..footer + 4].try_into().unwrap()) as usize;
    file[extension..extension + 2].copy_from_slice(&494u16.to_le_bytes());
    assert!(dec(&file).is_err());
    let mut file = preceding_metadata(false, 2);
    let footer = file.len() - SIGNATURE.len() - 8;
    file.remove(footer - 1);
    assert!(dec(&file).is_err());
}

#[test]
fn valid_developer_directory_with_overlapping_payload_rejects_pixels() {
    let mut file = preceding_metadata(true, 0);
    let footer = file.len() - SIGNATURE.len() - 8;
    let directory = u32::from_le_bytes(file[footer + 4..footer + 8].try_into().unwrap()) as usize;
    file[directory..directory + 2].copy_from_slice(&2u16.to_le_bytes());
    let mut extra = [0u8; 10];
    extra[..2].copy_from_slice(&2u16.to_le_bytes());
    extra[2..6].copy_from_slice(&(HEADER as u32).to_le_bytes());
    extra[6..10].copy_from_slice(&1u32.to_le_bytes());
    file.splice(footer..footer, extra);
    let moved_footer = file.len() - SIGNATURE.len() - 8;
    assert_eq!(u32::from_le_bytes(file[moved_footer + 4..moved_footer + 8].try_into().unwrap()), directory as u32);
    assert_eq!(u16::from_le_bytes(file[directory..directory + 2].try_into().unwrap()), 2);
    assert_eq!(&file[directory + 12..directory + 22], &extra);
    assert!(dec(&file).is_err());
}

#[test]
fn nonempty_developer_reference_inside_pixels_rejects_the_whole_image() {
    let mut file = header(2, 2, 1, 24, 0x20);
    file.extend_from_slice(&[0, 0, 255, 0, 255, 0]);
    let directory = file.len() as u32;
    file.extend_from_slice(&1u16.to_le_bytes());
    file.extend_from_slice(&1u16.to_le_bytes());
    file.extend_from_slice(&(HEADER as u32 + 3).to_le_bytes());
    file.extend_from_slice(&1u32.to_le_bytes());
    file.extend_from_slice(&0u32.to_le_bytes());
    file.extend_from_slice(&directory.to_le_bytes());
    file.extend_from_slice(SIGNATURE);
    assert!(dec(&file).is_err(), "an overlapping reference must not return a partial image");
}

#[test]
fn inflated_developer_count_scans_the_next_chunk_as_an_entry_and_rejects_its_pixel_payload() {
    // a directory claiming two entries over one real entry: the ten bytes after it, scanned as
    // the second entry, name a payload inside the pixels, which is refused like any other
    let mut file = header(2, 2, 1, 24, 0x20);
    file.extend_from_slice(&[0, 0, 255, 0, 255, 0]);
    let directory = file.len() as u32;
    file.extend_from_slice(&2u16.to_le_bytes());
    file.extend_from_slice(&1u16.to_le_bytes());
    file.extend_from_slice(&(directory + 22).to_le_bytes());
    file.extend_from_slice(&0u32.to_le_bytes());
    file.extend_from_slice(&2u16.to_le_bytes());
    file.extend_from_slice(&(HEADER as u32 + 3).to_le_bytes());
    file.extend_from_slice(&1u32.to_le_bytes());
    file.extend_from_slice(&0u32.to_le_bytes());
    file.extend_from_slice(&directory.to_le_bytes());
    file.extend_from_slice(SIGNATURE);
    assert!(dec(&file).is_err(), "a scanned entry's payload inside the pixels is refused");
}

#[test]
fn legacy_pixels_that_end_in_signature_at_or_after_the_header_are_read_as_a_footer() {
    // a footerless 36×1 grayscale image whose last 26 bytes happen to be eight zeros and the
    // signature: the putative trailer begins at byte 28, so it is taken for a footer and the
    // image, left with 10 pixel bytes, is rejected as truncated (the ambiguity the body states)
    let prefix = 18usize;
    let width = (prefix + SIGNATURE.len()) as u16;
    let mut file = header(3, width, 1, 8, 0x20);
    file.extend(vec![0; prefix]);
    file.extend_from_slice(SIGNATURE);
    assert!(dec(&file).is_err());
}

#[test]
fn mutated_tga_footer_files_never_panic() {
    let mut file = header(2, 2, 1, 24, 0x20);
    file.extend_from_slice(&[0, 0, 255, 0, 255, 0]);
    file.extend_from_slice(&[0; 8]);
    file.extend_from_slice(SIGNATURE);
    for end in 0..file.len() {
        assert!(std::panic::catch_unwind(|| dec(&file[..end])).is_ok(), "truncation at {end}");
    }
    for at in 0..file.len() {
        let mut mutated = file.clone();
        mutated[at] ^= 0xFF;
        assert!(std::panic::catch_unwind(|| dec(&mutated)).is_ok(), "mutation at {at}");
    }
    for metadata in 0..4 {
        let file = preceding_metadata(true, metadata);
        assert!(dec(&file).is_ok());
        let footer = file.len() - SIGNATURE.len() - 8;
        for end in 0..footer {
            let mut truncated = file[..end].to_vec();
            truncated.extend_from_slice(&file[footer..]);
            assert!(std::panic::catch_unwind(|| dec(&truncated)).is_ok(), "metadata {metadata}, footer-preserving truncation at {end}");
        }
        for at in 0..file.len() {
            let mut mutated = file.clone();
            mutated[at] ^= 0xFF;
            assert!(std::panic::catch_unwind(|| dec(&mutated)).is_ok(), "metadata {metadata}, mutation at {at}");
        }
    }
}
