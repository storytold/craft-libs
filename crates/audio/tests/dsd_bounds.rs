//! Hostile container sizes must return errors, never wrap offsets or panic.
use craft_audio::dsd::{BitOrder, Error, RawAudio, encode_dsdiff, encode_dsf, read_metadata, read_raw};

#[test]
fn dsf_untrusted_sizes_are_checked() {
    let original = encode_dsf(2_822_400, &[&[0x69; 8]], 64, 8).unwrap();
    for offset in [32, 84] {
        // fmt and data chunk sizes
        for size in [u64::MAX, u64::MAX - 31, 1 << 32] {
            let mut bytes = original.clone();
            bytes[offset..offset + 8].copy_from_slice(&size.to_le_bytes());
            assert!(read_metadata(&bytes).is_err());
            assert!(read_raw(&bytes).is_err());
        }
    }
    let mut bytes = original;
    bytes[64..72].copy_from_slice(&u64::MAX.to_le_bytes());
    assert_eq!(read_raw(&bytes), Err(Error::MemoryLimitExceeded));
}

#[test]
fn dsdiff_untrusted_top_level_and_property_sizes_are_checked() {
    let original = encode_dsdiff(2_822_400, &[&[0xaa; 8]], 64).unwrap();
    let prop = original.windows(4).position(|w| w == b"PROP").unwrap();
    for offset in [4, 20, prop + 20] {
        // FRM8, FVER, nested FS
        for size in [u64::MAX, u64::MAX - 31, 1 << 32] {
            let mut bytes = original.clone();
            bytes[offset..offset + 8].copy_from_slice(&size.to_be_bytes());
            assert!(read_metadata(&bytes).is_err());
            assert!(read_raw(&bytes).is_err());
        }
    }
}

#[test]
fn dsdiff_invalid_optional_disc_info_is_ignored_without_overflow() {
    let mut bytes = encode_dsdiff(2_822_400, &[&[0xaa; 8]], 64).unwrap();
    let expected = read_metadata(&bytes).unwrap();
    bytes.extend_from_slice(b"DIIN");
    bytes.extend_from_slice(&12u64.to_be_bytes());
    bytes.extend_from_slice(b"DITI");
    bytes.extend_from_slice(&u64::MAX.to_be_bytes());
    let size = (bytes.len() - 12) as u64;
    bytes[4..12].copy_from_slice(&size.to_be_bytes());
    // Optional malformed text is ignored on 64-bit; unrepresentable sizes may
    // be rejected on narrower targets, but must never wrap or panic.
    match read_metadata(&bytes) {
        Ok(actual) => assert_eq!(actual, expected),
        Err(Error::MemoryLimitExceeded) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn empty_raw_audio_rejects_invalid_decimation_factors() {
    let bytes = encode_dsf(2_822_400, &[&[]], 0, 8).unwrap();
    let raw = RawAudio { metadata: read_metadata(&bytes).unwrap(), bit_order: BitOrder::LsbFirst, channel_bytes: Vec::new() };
    for factor in [0, 1, 7, 9, u32::MAX] {
        assert!(matches!(raw.to_pcm_with_factor(factor), Err(Error::Unsupported(_))));
    }
    for factor in [8, 16, 32, 64] {
        assert_eq!(raw.to_pcm_with_factor(factor).unwrap().sample_rate, 2_822_400 / factor);
    }
}

#[test]
fn writers_reject_unbounded_output_before_allocating() {
    let channels: [&[u8]; 8] = [&[]; 8];
    for count in [u64::MAX, (craft_audio::dsd::MAX_ENCODE_BYTES as u64 + 1) * 8] {
        assert_eq!(encode_dsf(2_822_400, &channels, count, 8), Err(Error::MemoryLimitExceeded));
        assert_eq!(encode_dsdiff(2_822_400, &channels, count), Err(Error::MemoryLimitExceeded));
    }
    assert!(matches!(encode_dsf(2_822_400, &[&[]], 0, u32::MAX), Err(Error::InvalidData(_))));
}

#[test]
fn encoded_sizes_match_actual_bytes_for_odd_and_even_payloads() {
    for channels in 1..=8 {
        for len in 0..=9 {
            let data = vec![0x69; len];
            let planes = vec![data.as_slice(); channels];
            for block in [0, 1, 3, 8] {
                // DSF has channel types for 1 to 6 channels only.
                if channels > 6 {
                    assert!(matches!(encode_dsf(2_822_400, &planes, len as u64 * 8, block), Err(Error::Unsupported(_))));
                    continue;
                }
                let dsf = encode_dsf(2_822_400, &planes, len as u64 * 8, block).unwrap();
                assert_eq!(u64::from_le_bytes(dsf[12..20].try_into().unwrap()), dsf.len() as u64);
                assert_eq!(read_raw(&dsf).unwrap().channel_bytes, vec![data.clone(); channels]);
            }
            let dff = encode_dsdiff(2_822_400, &planes, len as u64 * 8).unwrap();
            assert_eq!(u64::from_be_bytes(dff[4..12].try_into().unwrap()) + 12, dff.len() as u64);
            assert_eq!(read_raw(&dff).unwrap().channel_bytes, vec![data; channels]);
        }
    }
}

#[test]
fn dsf_audio_stops_at_the_declared_data_chunk() {
    for channels in 1..=2 {
        let input = vec![&[0x33u8; 8][..]; channels];
        let mut bytes = encode_dsf(2_822_400, &input, 64, 8).unwrap();
        // Ask for an additional block without extending the data chunk.
        bytes[64..72].copy_from_slice(&128u64.to_le_bytes());
        let expected = read_raw(&bytes).unwrap();
        for c in &expected.channel_bytes {
            assert_eq!(&c[..8], &[0x33; 8]);
            assert_eq!(&c[8..], &[0x69; 8]);
        }
        let metadata_offset = bytes.len() as u64;
        bytes[20..28].copy_from_slice(&metadata_offset.to_le_bytes());
        bytes.extend_from_slice(b"ID3\x04\x00\x00\x00\x00\x00\x00");
        bytes.extend_from_slice(&[0xff; 32]);
        let file_size = bytes.len() as u64;
        bytes[12..20].copy_from_slice(&file_size.to_le_bytes());
        assert_eq!(read_raw(&bytes).unwrap().channel_bytes, expected.channel_bytes);
    }
}

#[test]
fn dsdiff_odd_payload_receives_pad_byte() {
    let input = [&[0xaau8; 3][..]];
    let bytes = encode_dsdiff(2_822_400, &input, 24).unwrap();
    let raw = read_raw(&bytes).unwrap();
    assert_eq!(raw.channel_bytes[0].len(), 3);
}

#[test]
fn dsf_declared_file_size_and_metadata_pointer_bounds() {
    let valid = encode_dsf(2_822_400, &[&[0x69; 8]], 64, 8).unwrap();

    // Undersized file size declaration (e.g. 28)
    let mut undersized = valid.clone();
    undersized[12..20].copy_from_slice(&28u64.to_le_bytes());
    assert!(read_metadata(&undersized).is_err());
    assert!(read_raw(&undersized).is_err());

    // Oversized file size declaration beyond buffer
    let mut oversized = valid.clone();
    let big_size = valid.len() as u64 + 100;
    oversized[12..20].copy_from_slice(&big_size.to_le_bytes());
    assert!(read_metadata(&oversized).is_err());
    assert!(read_raw(&oversized).is_err());

    // Metadata pointer outside the declared file bounds
    let mut outside_meta = valid.clone();
    let outside_offset = valid.len() as u64 + 10;
    outside_meta[20..28].copy_from_slice(&outside_offset.to_le_bytes());
    assert!(read_metadata(&outside_meta).is_err());
    assert!(read_raw(&outside_meta).is_err());

    // Metadata pointer overlapping data chunk
    let mut overlap_meta = valid.clone();
    overlap_meta[20..28].copy_from_slice(&32u64.to_le_bytes());
    assert!(read_metadata(&overlap_meta).is_err());
    assert!(read_raw(&overlap_meta).is_err());
}

#[test]
fn dsdiff_channel_count_and_duplicate_chunks_are_validated() {
    let base = encode_dsdiff(2_822_400, &[&[0xaa; 8]], 64).unwrap();

    // Locate PROP chunk payload and CHNL subchunk
    let prop_idx = base.windows(4).position(|w| w == b"PROP").unwrap();
    let chnl_idx = base.windows(4).position(|w| w == b"CHNL").unwrap();

    // 1. Oversized channel count (e.g. 65535 or 9)
    let mut bad_count = base.clone();
    bad_count[chnl_idx + 12..chnl_idx + 14].copy_from_slice(&65535u16.to_be_bytes());
    assert!(matches!(read_metadata(&bad_count), Err(Error::Unsupported(_))));
    assert!(matches!(read_raw(&bad_count), Err(Error::Unsupported(_))));

    let mut zero_count = base.clone();
    zero_count[chnl_idx + 12..chnl_idx + 14].copy_from_slice(&0u16.to_be_bytes());
    assert!(matches!(read_metadata(&zero_count), Err(Error::Unsupported(_))));

    // 2. Conflicting duplicate CHNL chunk in PROP
    let mut dup_chnl = base.clone();
    let chnl_chunk = dup_chnl[chnl_idx..chnl_idx + 18].to_vec();
    dup_chnl.splice(chnl_idx..chnl_idx, chnl_chunk);
    // Adjust PROP size and FRM8 size
    let prop_size = u64::from_be_bytes(dup_chnl[prop_idx + 4..prop_idx + 12].try_into().unwrap()) + 18;
    dup_chnl[prop_idx + 4..prop_idx + 12].copy_from_slice(&prop_size.to_be_bytes());
    let frm8_size = (dup_chnl.len() - 12) as u64;
    dup_chnl[4..12].copy_from_slice(&frm8_size.to_be_bytes());
    assert_eq!(read_metadata(&dup_chnl), Err(Error::InvalidHeader("duplicate CHNL chunk")));

    // 3. Duplicate FS chunk in PROP
    let fs_idx = base.windows(4).position(|w| w == b"FS  ").unwrap();
    let mut dup_fs = base.clone();
    let fs_chunk = dup_fs[fs_idx..fs_idx + 16].to_vec();
    dup_fs.splice(fs_idx..fs_idx, fs_chunk);
    let prop_size = u64::from_be_bytes(dup_fs[prop_idx + 4..prop_idx + 12].try_into().unwrap()) + 16;
    dup_fs[prop_idx + 4..prop_idx + 12].copy_from_slice(&prop_size.to_be_bytes());
    let frm8_size = (dup_fs.len() - 12) as u64;
    dup_fs[4..12].copy_from_slice(&frm8_size.to_be_bytes());
    assert_eq!(read_metadata(&dup_fs), Err(Error::InvalidHeader("duplicate FS chunk")));
}

#[test]
#[cfg(target_pointer_width = "32")]
fn dsdiff_32bit_padding_overflow_is_rejected() {
    let dummy = [&[][..]];
    let result = encode_dsdiff(2_822_400, &dummy, 34_359_738_360);
    assert_eq!(result, Err(Error::MemoryLimitExceeded));
}
