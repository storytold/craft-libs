//! Packet-capture fixtures: hand-built Ethernet/IPv4 frames (DNS, TCP handshake, TLS
//! ClientHello, HTTP request, ARP) written as classic pcap or pcapng.

const CLASSIC_US: u32 = 0xa1b2_c3d4;
const NG_SHB: u32 = 0x0a0d_0d0a;
const NG_BOM: u32 = 0x1a2b_3c4d;

pub fn eth(ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = vec![0xaa; 6];
    f.extend([0xbb; 6]);
    f.extend(ethertype.to_be_bytes());
    f.extend(payload);
    f
}

pub fn ipv4_pkt(proto: u8, src: [u8; 4], dst: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut p = vec![0x45, 0, 0, 0, 0, 1, 0, 0, 64, proto, 0, 0];
    p.extend(src);
    p.extend(dst);
    let total = (20 + payload.len()) as u16;
    p[2..4].copy_from_slice(&total.to_be_bytes());
    p.extend(payload);
    p
}

pub fn udp(sp: u16, dp: u16, payload: &[u8]) -> Vec<u8> {
    let mut u = sp.to_be_bytes().to_vec();
    u.extend(dp.to_be_bytes());
    u.extend(((8 + payload.len()) as u16).to_be_bytes());
    u.extend([0, 0]);
    u.extend(payload);
    u
}

pub fn tcp(sp: u16, dp: u16, flags: u8, payload: &[u8]) -> Vec<u8> {
    let mut t = sp.to_be_bytes().to_vec();
    t.extend(dp.to_be_bytes());
    t.extend(1000u32.to_be_bytes());
    t.extend(0u32.to_be_bytes());
    t.extend([0x50, flags, 0xff, 0xff, 0, 0, 0, 0]);
    t.extend(payload);
    t
}

/// DNS response for example.com A 93.184.216.34 (name compressed in the answer).
pub fn dns_response() -> Vec<u8> {
    let mut m = vec![0x12, 0x34, 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0];
    m.extend(b"\x07example\x03com\x00\x00\x01\x00\x01");
    m.extend([0xc0, 12, 0, 1, 0, 1, 0, 0, 0x0e, 0x10, 0, 4, 93, 184, 216, 34]);
    m
}

pub fn dns_query() -> Vec<u8> {
    let mut m = vec![0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    m.extend(b"\x07example\x03com\x00\x00\x01\x00\x01");
    m
}

/// A TLS 1.3-style ClientHello naming `host` with ALPN h2 and http/1.1.
pub fn client_hello(host: &str) -> Vec<u8> {
    let mut ext = Vec::new();
    let mut sni = vec![0u8];
    sni.extend((host.len() as u16).to_be_bytes());
    sni.extend(host.as_bytes());
    let mut sni_list = (sni.len() as u16).to_be_bytes().to_vec();
    sni_list.extend(sni);
    ext.extend(0u16.to_be_bytes());
    ext.extend((sni_list.len() as u16).to_be_bytes());
    ext.extend(sni_list);
    let alpn_list = b"\x02h2\x08http/1.1";
    ext.extend(16u16.to_be_bytes());
    ext.extend(((alpn_list.len() + 2) as u16).to_be_bytes());
    ext.extend((alpn_list.len() as u16).to_be_bytes());
    ext.extend(alpn_list);
    ext.extend(43u16.to_be_bytes());
    ext.extend(5u16.to_be_bytes());
    ext.extend([4, 0x03, 0x04, 0x03, 0x03]);
    let mut body = vec![3, 3];
    body.extend([7u8; 32]);
    body.push(0); // session id
    body.extend([0, 2, 0x13, 0x01]); // one cipher suite
    body.extend([1, 0]); // compression
    body.extend((ext.len() as u16).to_be_bytes());
    body.extend(ext);
    let mut hs = vec![1];
    hs.extend(&(body.len() as u32).to_be_bytes()[1..]);
    hs.extend(body);
    let mut rec = vec![22, 3, 1];
    rec.extend((hs.len() as u16).to_be_bytes());
    rec.extend(hs);
    rec
}

/// Frames of a small conversation, as (link type 1) Ethernet frames.
pub fn sample_frames() -> Vec<Vec<u8>> {
    let c = [192, 168, 1, 10];
    let dns_srv = [8, 8, 8, 8];
    let web = [93, 184, 216, 34];
    vec![
        eth(0x0800, &ipv4_pkt(17, c, dns_srv, &udp(50000, 53, &dns_query()))),
        eth(0x0800, &ipv4_pkt(17, dns_srv, c, &udp(53, 50000, &dns_response()))),
        eth(0x0800, &ipv4_pkt(6, c, web, &tcp(50001, 443, 0x02, &[]))),
        eth(0x0800, &ipv4_pkt(6, web, c, &tcp(443, 50001, 0x12, &[]))),
        eth(0x0800, &ipv4_pkt(6, c, web, &tcp(50001, 443, 0x18, &client_hello("example.com")))),
        eth(
            0x0800,
            &ipv4_pkt(6, c, web, &tcp(50002, 80, 0x18, b"GET http://user:pw@example.com/a?b=1 HTTP/1.1\r\nHost: example.com\r\nUser-Agent: t\r\n\r\n")),
        ),
        eth(0x0806, &[0u8; 28]),
    ]
}

/// A classic little-endian microsecond pcap of `frames`.
pub fn pcap(frames: &[Vec<u8>]) -> Vec<u8> {
    let mut out = CLASSIC_US.to_le_bytes().to_vec();
    out.extend(2u16.to_le_bytes());
    out.extend(4u16.to_le_bytes());
    out.extend([0u8; 8]);
    out.extend(65535u32.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    for (i, f) in frames.iter().enumerate() {
        out.extend((1_700_000_000u32 + i as u32).to_le_bytes());
        out.extend(500u32.to_le_bytes());
        out.extend((f.len() as u32).to_le_bytes());
        out.extend((f.len() as u32).to_le_bytes());
        out.extend(f);
    }
    out
}

pub fn block(t: u32, body: &[u8]) -> Vec<u8> {
    let mut body = body.to_vec();
    while body.len() % 4 != 0 {
        body.push(0);
    }
    let len = (body.len() + 12) as u32;
    let mut b = t.to_le_bytes().to_vec();
    b.extend(len.to_le_bytes());
    b.extend(body);
    b.extend(len.to_le_bytes());
    b
}

/// A little-endian pcapng with one Ethernet interface at nanosecond resolution.
pub fn pcapng(frames: &[Vec<u8>]) -> Vec<u8> {
    let mut shb = NG_BOM.to_le_bytes().to_vec();
    shb.extend(1u16.to_le_bytes());
    shb.extend(0u16.to_le_bytes());
    shb.extend((-1i64).to_le_bytes());
    let mut out = block(NG_SHB, &shb);
    let mut idb = 1u16.to_le_bytes().to_vec();
    idb.extend([0, 0]);
    idb.extend(0u32.to_le_bytes());
    idb.extend([9, 0, 1, 0, 9, 0, 0, 0, 0, 0, 0, 0]); // if_tsresol = 9, end of options
    out.extend(block(1, &idb));
    for (i, f) in frames.iter().enumerate() {
        let ts: u64 = 1_700_000_000_000_000_000 + i as u64;
        let mut epb = 0u32.to_le_bytes().to_vec();
        epb.extend(((ts >> 32) as u32).to_le_bytes());
        epb.extend((ts as u32).to_le_bytes());
        epb.extend((f.len() as u32).to_le_bytes());
        epb.extend((f.len() as u32).to_le_bytes());
        epb.extend(f);
        out.extend(block(6, &epb));
    }
    out
}
