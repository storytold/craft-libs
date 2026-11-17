use craft_anything_formats::pcap::Capture;
use craft_anything_testkit::pcap::{block, pcapng};

fn capture(original: u32, snaplen: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = pcapng(&[]);
    out[40..44].copy_from_slice(&snaplen.to_le_bytes());
    let mut body = original.to_le_bytes().to_vec();
    body.extend(payload);
    out.extend(block(3, &body));
    out
}

#[test]
fn a_simple_packet_cannot_silently_shorten_its_declared_capture() {
    let data = capture(10, 0, &[1, 2, 3, 4]);
    let mut cap = Capture::new(&data).unwrap();
    assert!(cap.next().is_none());
    assert!(!cap.warnings.is_empty());
}

#[test]
fn a_simple_packet_cannot_have_unexplained_extra_payload() {
    let data = capture(1, 0, &[1; 8]);
    let mut cap = Capture::new(&data).unwrap();
    assert!(cap.next().is_none());
    assert!(!cap.warnings.is_empty());
}

#[test]
fn snaplen_truncation_and_alignment_padding_are_preserved() {
    for (original, snaplen) in [(3, 0), (10, 3)] {
        let data = capture(original, snaplen, &[1, 2, 3]);
        let mut cap = Capture::new(&data).unwrap();
        let packet = cap.next().unwrap();
        assert_eq!(packet.data, [1, 2, 3]);
        assert_eq!(packet.original_len, original);
        assert!(cap.next().is_none());
        assert!(cap.warnings.is_empty());
    }
}

#[test]
fn rejected_simple_packets_share_one_warning_and_preserve_later_frames() {
    let mut data = pcapng(&[]);
    let rejected = block(3, &1u32.to_le_bytes());
    for _ in 0..128 {
        data.extend(&rejected);
    }
    let mut valid = 3u32.to_le_bytes().to_vec();
    valid.extend([1, 2, 3]);
    data.extend(block(3, &valid));
    let mut cap = Capture::new(&data).unwrap();
    let frame = cap.next().unwrap();
    assert_eq!(frame.number, 1);
    assert_eq!(frame.data, [1, 2, 3]);
    assert!(cap.next().is_none());
    assert!(!cap.truncated);
    assert_eq!(cap.warnings.len(), 1);
    assert!(cap.warnings[0].contains("128 simple packet blocks"));
    assert!(cap.next().is_none());
    assert_eq!(cap.warnings.len(), 1);
}
