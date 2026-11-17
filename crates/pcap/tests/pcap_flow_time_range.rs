use craft_anything_formats::pcap::summarize;
use craft_anything_testkit::pcap::{block, eth, ipv4_pkt, pcap, pcapng, udp};

fn frame() -> Vec<u8> {
    eth(0x0800, &ipv4_pkt(17, [1; 4], [2; 4], &udp(100, 200, b"body")))
}

#[test]
fn out_of_order_flow_packets_keep_the_earliest_and_latest_timestamps() {
    let f = frame();
    let mut capture = pcap(&[f.clone(), f.clone(), f.clone()]);
    for (i, seconds) in [30u32, 10, 20].into_iter().enumerate() {
        let at = 24 + i * (16 + f.len());
        capture[at..at + 4].copy_from_slice(&seconds.to_le_bytes());
    }
    let summary = summarize(&capture, &|| false).unwrap();
    let flow = &summary.flows[0];
    assert_eq!(flow.first_ts_ns, summary.first_ts_ns);
    assert_eq!(flow.last_ts_ns, summary.last_ts_ns);
    assert_eq!(flow.first_frame, 1);
}

#[test]
fn a_timestamped_packet_fills_a_flow_started_by_a_simple_packet() {
    let f = frame();
    let headers = pcapng(&[]);
    let mut simple = (f.len() as u32).to_le_bytes().to_vec();
    simple.extend(&f);
    let mut capture = headers.clone();
    capture.extend(block(3, &simple));
    capture.extend(&pcapng(&[f])[headers.len()..]);
    let summary = summarize(&capture, &|| false).unwrap();
    assert!(summary.first_ts_ns.is_some());
    assert_eq!(summary.flows[0].first_ts_ns, summary.first_ts_ns);
    assert_eq!(summary.flows[0].last_ts_ns, summary.last_ts_ns);
}
