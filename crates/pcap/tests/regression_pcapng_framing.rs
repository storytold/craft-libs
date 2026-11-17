use craft_anything_formats::pcap::summarize;
use craft_anything_testkit::pcap::{block, pcapng, sample_frames};

#[test]
fn mismatched_block_footers_stop_the_capture_with_a_warning() {
    let original = pcapng(&sample_frames());
    for footer in [24, 56] {
        let mut data = original.clone();
        data[footer..footer + 4].copy_from_slice(&12u32.to_le_bytes());
        let result = summarize(&data, &|| false).unwrap();
        assert!(!result.warnings.is_empty(), "footer at {footer}");
        assert_eq!(result.packets, 0);
    }
}

#[test]
fn fixed_block_headers_cannot_borrow_the_next_blocks_bytes() {
    let header = pcapng(&[]);
    for (kind, short_body) in [(1, vec![]), (2, vec![0; 16]), (6, vec![0; 16]), (3, vec![])] {
        let mut data = header[..28].to_vec();
        if kind != 1 {
            data.extend(&header[28..]);
        }
        data.extend(block(kind, &short_body));
        data.extend(&pcapng(&sample_frames())[28..]);
        let result = summarize(&data, &|| false).unwrap();
        assert!(!result.warnings.is_empty(), "kind {kind}");
        assert_eq!(result.packets, 0, "kind {kind}");
        assert_eq!(result.interfaces.len(), usize::from(kind != 1), "kind {kind}");
    }
}

#[test]
fn valid_blocks_and_unknown_blocks_still_work() {
    let original = pcapng(&sample_frames());
    let mut data = original[..60].to_vec();
    data.extend(block(0x0000_1234, b"ignored"));
    data.extend(&original[60..]);
    let result = summarize(&data, &|| false).unwrap();
    assert_eq!(result.packets, 7);
    assert!(result.warnings.is_empty());
    assert_eq!(result.interfaces.len(), 1);
}

#[test]
fn big_endian_block_footers_use_the_sections_byte_order() {
    let be_block = |kind: u32, body: &[u8]| {
        let size = (body.len() + 12) as u32;
        let mut out = kind.to_be_bytes().to_vec();
        out.extend(size.to_be_bytes());
        out.extend(body);
        out.extend(size.to_be_bytes());
        out
    };
    let mut section = 0x1a2b3c4du32.to_be_bytes().to_vec();
    section.extend([0, 1, 0, 0]);
    section.extend((-1i64).to_be_bytes());
    let mut data = be_block(0x0a0d0d0a, &section);
    data.extend(be_block(1, &[0, 1, 0, 0, 0, 0, 0, 0]));
    let result = summarize(&data, &|| false).unwrap();
    assert!(result.warnings.is_empty());
    assert_eq!(result.interfaces[0].link_type, 1);
    let end = data.len();
    data[end - 4..].copy_from_slice(&20u32.to_le_bytes());
    assert!(!summarize(&data, &|| false).unwrap().warnings.is_empty());
}
