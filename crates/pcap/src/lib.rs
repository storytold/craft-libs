//! Saved packet captures: classic pcap (microsecond and nanosecond, either byte order) and pcapng
//! (section, interface, enhanced/simple/obsolete packet blocks). Frames are decoded from Ethernet
//! (with VLAN tags), raw IP, Linux cooked (SLL/SLL2) and BSD loopback links into IPv4/IPv6,
//! TCP/UDP/ICMP, and the application facts most people ask about: DNS questions and answers,
//! HTTP/1.x request and status lines with the Host header, and the TLS ClientHello server name
//! (SNI) and ALPN. Each frame is decoded on its own: no TCP reassembly, no decryption. Pure, no I/O,
//! every length checked.

use crate::FormatError;
use crate::bytes::Reader;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Most frames read from one capture.
pub const MAX_PACKETS: u64 = 5_000_000;
/// Most flows tracked; later flows are counted but get no id.
pub const MAX_FLOWS: usize = 100_000;
/// Most distinct values kept per inventory (endpoints, DNS names, hosts…).
pub const MAX_INVENTORY: usize = 10_000;

const CLASSIC_US: u32 = 0xa1b2_c3d4;
const CLASSIC_NS: u32 = 0xa1b2_3c4d;
const NG_SHB: u32 = 0x0a0d_0d0a;
const NG_BOM: u32 = 0x1a2b_3c4d;

/// One capture interface (a pcap file has exactly one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Interface {
    pub link_type: u32,
    pub link_name: &'static str,
    pub snaplen: u32,
    /// Timestamp units per second, as a power: (base 10 or 2, exponent).
    #[serde(skip)]
    tsresol: (u8, u8),
    /// Signed offset in seconds added to timestamps on this interface (`if_tsoffset`).
    #[serde(skip)]
    tsoffset: i64,
}

/// The plain name of a link-layer type.
pub fn link_name(link: u32) -> &'static str {
    match link {
        0 => "BSD loopback",
        1 => "Ethernet",
        12 | 14 | 101 => "raw IP",
        108 => "OpenBSD loopback",
        113 => "Linux cooked (SLL)",
        228 => "raw IPv4",
        229 => "raw IPv6",
        276 => "Linux cooked v2 (SLL2)",
        105 => "IEEE 802.11",
        127 => "802.11 radiotap",
        _ => "other",
    }
}

/// One captured frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame<'a> {
    /// 1-based frame number, as capture tools show it.
    pub number: u64,
    /// Nanoseconds since the Unix epoch, when representable.
    pub ts_ns: Option<u64>,
    pub interface: usize,
    pub link_type: u32,
    pub original_len: u32,
    pub data: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Classic { le: bool, nanos: bool },
    Ng { le: bool },
}

/// Walks the frames of a capture. Malformed data stops the walk with a warning, never a panic.
#[derive(Debug, Clone)]
pub struct Capture<'a> {
    data: &'a [u8],
    pos: usize,
    kind: Kind,
    /// Interfaces of the current pcapng section start here in `interfaces`.
    section_base: usize,
    pub format: &'static str,
    pub version: String,
    pub interfaces: Vec<Interface>,
    pub warnings: Vec<String>,
    /// The walk stopped at `MAX_PACKETS`.
    pub truncated: bool,
    count: u64,
}

fn rd_u16(r: &Reader<'_>, le: bool, off: usize) -> Option<u16> {
    if le { r.u16_le(off) } else { r.u16_be(off) }
}

fn rd_u32(r: &Reader<'_>, le: bool, off: usize) -> Option<u32> {
    if le { r.u32_le(off) } else { r.u32_be(off) }
}

fn rd_i64(r: &Reader<'_>, le: bool, off: usize) -> Option<i64> {
    if le { r.u64_le(off) } else { r.u64_be(off) }.map(|v| v as i64)
}

/// True when the bytes start like a pcap or pcapng file.
pub fn is_capture(data: &[u8]) -> bool {
    let r = Reader::new(data);
    matches!(r.u32_le(0), Some(CLASSIC_US | CLASSIC_NS | NG_SHB)) || matches!(r.u32_be(0), Some(CLASSIC_US | CLASSIC_NS))
}

impl<'a> Capture<'a> {
    /// Read the file header.
    pub fn new(data: &'a [u8]) -> Result<Self, FormatError> {
        let r = Reader::new(data);
        let magic_le = r.u32_le(0).ok_or_else(|| FormatError::Truncated("no capture header".into()))?;
        let magic_be = r.u32_be(0).unwrap_or(0);
        let mut cap = Capture {
            data,
            pos: 0,
            kind: Kind::Ng { le: true },
            section_base: 0,
            format: "pcapng",
            version: String::new(),
            interfaces: Vec::new(),
            warnings: Vec::new(),
            truncated: false,
            count: 0,
        };
        let classic = |le: bool, nanos: bool| -> Result<(Kind, String, Interface), FormatError> {
            let major = rd_u16(&r, le, 4).ok_or_else(|| FormatError::Truncated("pcap header".into()))?;
            let minor = rd_u16(&r, le, 6).unwrap_or(0);
            let snaplen = rd_u32(&r, le, 16).ok_or_else(|| FormatError::Truncated("pcap header".into()))?;
            // Link type: bits 0–15; reserved: 16–25 and 27; FCS-present: 26; FCS length in 16-bit words: 28–31.
            let link = rd_u32(&r, le, 20).ok_or_else(|| FormatError::Truncated("pcap header".into()))? & 0xffff;
            let res = if nanos { (10, 9) } else { (10, 6) };
            let iface = Interface { link_type: link, link_name: link_name(link), snaplen, tsresol: res, tsoffset: 0 };
            Ok((Kind::Classic { le, nanos }, format!("{major}.{minor}"), iface))
        };
        match (magic_le, magic_be) {
            (CLASSIC_US, _) | (_, CLASSIC_US) | (CLASSIC_NS, _) | (_, CLASSIC_NS) => {
                let le = magic_le == CLASSIC_US || magic_le == CLASSIC_NS;
                let nanos = (if le { magic_le } else { magic_be }) == CLASSIC_NS;
                let (kind, version, iface) = classic(le, nanos)?;
                cap.kind = kind;
                cap.format = "pcap";
                cap.version = version;
                cap.interfaces.push(iface);
                cap.pos = 24;
            }
            (NG_SHB, _) => {
                let le = match r.u32_le(8) {
                    Some(NG_BOM) => true,
                    _ if r.u32_be(8) == Some(NG_BOM) => false,
                    _ => return Err(FormatError::Malformed("pcapng section header without a byte-order mark".into())),
                };
                cap.kind = Kind::Ng { le };
                cap.version = format!("{}.{}", rd_u16(&r, le, 12).unwrap_or(0), rd_u16(&r, le, 14).unwrap_or(0));
            }
            _ => return Err(FormatError::Unsupported("not a pcap or pcapng capture".into())),
        }
        Ok(cap)
    }

    fn stop(&mut self, why: impl Into<String>) -> Option<Frame<'a>> {
        self.warnings.push(why.into());
        self.pos = self.data.len();
        None
    }

    fn ts(&self, iface: usize, units: u64) -> Option<u64> {
        let (base, exp) = self.interfaces.get(iface).map(|i| i.tsresol).unwrap_or((10, 6));
        let offset_s = self.interfaces.get(iface).map(|i| i.tsoffset).unwrap_or(0);
        let ns = if base == 10 {
            if exp <= 9 { u128::from(units) * 10u128.pow(u32::from(9 - exp)) } else { u128::from(units) / 10u128.checked_pow(u32::from(exp - 9))? }
        } else {
            (u128::from(units) * 1_000_000_000) >> exp.min(127)
        };
        let adjusted = i128::try_from(ns).ok()?.checked_add(i128::from(offset_s) * 1_000_000_000)?;
        u64::try_from(adjusted).ok()
    }

    fn frame(&mut self, iface: usize, ts_units: u64, data: &'a [u8], original_len: u32) -> Frame<'a> {
        self.count += 1;
        Frame {
            number: self.count,
            ts_ns: self.ts(iface, ts_units),
            interface: iface,
            link_type: self.interfaces.get(iface).map(|i| i.link_type).unwrap_or(u32::MAX),
            original_len,
            data,
        }
    }

    fn next_classic(&mut self, le: bool, nanos: bool) -> Option<Frame<'a>> {
        let r = Reader::new(self.data);
        let p = self.pos;
        if p >= self.data.len() {
            return None;
        }
        let (Some(sec), Some(frac), Some(incl), Some(orig)) = (rd_u32(&r, le, p), rd_u32(&r, le, p + 4), rd_u32(&r, le, p + 8), rd_u32(&r, le, p + 12)) else {
            return self.stop(format!("record header at offset {p} is cut short"));
        };
        let Some(body) = usize::try_from(incl).ok().and_then(|n| r.slice(p + 16, n)) else {
            return self.stop(format!("record at offset {p} claims {incl} bytes past the end of the file"));
        };
        self.pos = p + 16 + body.len();
        let units = u64::from(sec) * if nanos { 1_000_000_000 } else { 1_000_000 } + u64::from(frac);
        Some(self.frame(0, units, body, orig))
    }

    fn next_ng(&mut self, mut le: bool) -> Option<Frame<'a>> {
        loop {
            let r = Reader::new(self.data);
            let p = self.pos;
            if p >= self.data.len() {
                return None;
            }
            let Some(btype) = rd_u32(&r, le, p) else { return self.stop(format!("block header at offset {p} is cut short")) };
            if btype == NG_SHB {
                // A new section can switch byte order.
                le = match r.u32_le(p + 8) {
                    Some(NG_BOM) => true,
                    _ if r.u32_be(p + 8) == Some(NG_BOM) => false,
                    _ => return self.stop(format!("section header at offset {p} has no byte-order mark")),
                };
                self.kind = Kind::Ng { le };
                self.section_base = self.interfaces.len();
            }
            let Some(blen) = rd_u32(&r, le, p + 4).and_then(|v| usize::try_from(v).ok()) else {
                return self.stop(format!("block at offset {p} is cut short"));
            };
            if blen < 12 || blen % 4 != 0 || r.slice(p, blen).is_none() {
                return self.stop(format!("block at offset {p} has an impossible length {blen}"));
            }
            self.pos = p + blen;
            let body = p + 8;
            let body_end = p + blen - 4;
            match btype {
                1 => {
                    let link = u32::from(rd_u16(&r, le, body).unwrap_or(u16::MAX));
                    let snaplen = rd_u32(&r, le, body + 4).unwrap_or(0);
                    let mut tsresol = (10u8, 6u8);
                    let mut tsoffset = 0i64;
                    let mut o = body + 8;
                    while o + 4 <= body_end {
                        let (Some(code), Some(len)) = (rd_u16(&r, le, o), rd_u16(&r, le, o + 2)) else { break };
                        if code == 0 {
                            break;
                        }
                        if code == 9
                            && len == 1
                            && o + 5 <= body_end
                            && let Some(v) = r.u8(o + 4)
                        {
                            tsresol = if v & 0x80 == 0 { (10, v) } else { (2, v & 0x7f) };
                        } else if code == 14
                            && len == 8
                            && o + 12 <= body_end
                            && let Some(v) = rd_i64(&r, le, o + 4)
                        {
                            tsoffset = v;
                        }
                        o += 4 + usize::from(len).div_ceil(4) * 4;
                    }
                    self.interfaces.push(Interface { link_type: link, link_name: link_name(link), snaplen, tsresol, tsoffset });
                }
                6 | 2 => {
                    let (iface, hi, lo, cap, orig) = if btype == 6 {
                        (
                            rd_u32(&r, le, body).map(|v| v as usize),
                            rd_u32(&r, le, body + 4),
                            rd_u32(&r, le, body + 8),
                            rd_u32(&r, le, body + 12),
                            rd_u32(&r, le, body + 16),
                        )
                    } else {
                        (
                            rd_u16(&r, le, body).map(usize::from),
                            rd_u32(&r, le, body + 4),
                            rd_u32(&r, le, body + 8),
                            rd_u32(&r, le, body + 12),
                            rd_u32(&r, le, body + 16),
                        )
                    };
                    let (Some(iface), Some(hi), Some(lo), Some(cap), Some(orig)) = (iface, hi, lo, cap, orig) else { continue };
                    let Some(data) = usize::try_from(cap).ok().filter(|&c| body + 20 + c <= body_end).and_then(|c| r.slice(body + 20, c)) else {
                        self.warnings.push(format!("packet block at offset {p} claims more bytes than it holds; skipped"));
                        continue;
                    };
                    let iface = self.section_base + iface;
                    if iface >= self.interfaces.len() {
                        self.warnings.push(format!("packet block at offset {p} names an undeclared interface; skipped"));
                        continue;
                    }
                    return Some(self.frame(iface, (u64::from(hi) << 32) | u64::from(lo), data, orig));
                }
                3 => {
                    let Some(orig) = rd_u32(&r, le, body) else { continue };
                    let iface = self.section_base;
                    let snap = self.interfaces.get(iface).map(|i| i.snaplen).filter(|&s| s > 0).unwrap_or(u32::MAX);
                    let room = body_end.saturating_sub(body + 4);
                    let cap = (orig.min(snap) as usize).min(room);
                    let Some(data) = r.slice(body + 4, cap) else { continue };
                    if iface >= self.interfaces.len() {
                        continue;
                    }
                    // Simple packet blocks carry no timestamp.
                    self.count += 1;
                    return Some(Frame {
                        number: self.count,
                        ts_ns: None,
                        interface: iface,
                        link_type: self.interfaces.get(iface).map(|i| i.link_type).unwrap_or(u32::MAX),
                        original_len: orig,
                        data,
                    });
                }
                _ => {}
            }
        }
    }
}

impl<'a> Iterator for Capture<'a> {
    type Item = Frame<'a>;

    fn next(&mut self) -> Option<Frame<'a>> {
        if self.count >= MAX_PACKETS {
            if !self.truncated && self.pos < self.data.len() {
                self.truncated = true;
                self.warnings.push(format!("stopped after {MAX_PACKETS} frames"));
            }
            return None;
        }
        match self.kind {
            Kind::Classic { le, nanos } => self.next_classic(le, nanos),
            Kind::Ng { le } => self.next_ng(le),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Decoding

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IpInfo {
    pub version: u8,
    pub src: IpAddr,
    pub dst: IpAddr,
    /// IP protocol number of the payload (6 TCP, 17 UDP, 1 ICMP, 58 ICMPv6…).
    pub protocol: u8,
    /// TTL or hop limit.
    pub ttl: u8,
    /// A non-first fragment: the transport header is in another frame.
    pub fragment: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Transport {
    /// tcp, udp, icmp, icmpv6 or other
    pub protocol: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dst_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tcp_flags: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icmp_type: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icmp_code: Option<u8>,
    pub payload_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DnsQuestion {
    pub name: String,
    pub qtype: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DnsAnswer {
    pub name: String,
    pub rtype: String,
    pub ttl: u32,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Dns {
    pub id: u16,
    pub response: bool,
    pub rcode: u8,
    pub questions: Vec<DnsQuestion>,
    pub answers: Vec<DnsAnswer>,
    /// The message was cut short or malformed part-way.
    pub incomplete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Http {
    /// request or response
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Request target with any user:password removed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TlsHello {
    pub record_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sni: Option<String>,
    pub alpn: Vec<String>,
    pub supported_versions: Vec<String>,
    /// The ClientHello continues in a later segment; extensions may be missing.
    pub partial: bool,
}

/// What one frame says.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Decoded {
    /// Protocol stack, outermost first: eth, vlan, sll, ipv4, ipv6, arp, tcp, udp, icmp, dns, http, tls…
    pub protocols: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<IpInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<Transport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns: Option<Dns>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http: Option<Http>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<TlsHello>,
    /// Where decoding stopped early, when it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Decoded {
    /// The protocol stack joined with `:`.
    pub fn stack(&self) -> String {
        self.protocols.join(":")
    }

    /// The direction-independent conversation this frame belongs to, and whether it travels a→b.
    pub fn flow_key(&self) -> Option<(FlowKey, bool)> {
        let ip = self.ip.as_ref()?;
        let t = self.transport.as_ref();
        let sp = t.and_then(|t| t.src_port).unwrap_or(0);
        let dp = t.and_then(|t| t.dst_port).unwrap_or(0);
        let (s, d) = ((ip.src, sp), (ip.dst, dp));
        let forward = s <= d;
        let (a, b) = if forward { (s, d) } else { (d, s) };
        Some((FlowKey { protocol: ip.protocol, a, b }, forward))
    }
}

/// A conversation: IP protocol and the two (address, port) ends in sorted order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FlowKey {
    pub protocol: u8,
    pub a: (IpAddr, u16),
    pub b: (IpAddr, u16),
}

/// Decode one frame from its link layer up.
pub fn decode(link_type: u32, data: &[u8]) -> Decoded {
    let mut d = Decoded::default();
    let r = Reader::new(data);
    let (ethertype, off) = match link_type {
        1 => {
            d.protocols.push("eth");
            let mut et = r.u16_be(12);
            let mut off = 14usize;
            for _ in 0..2 {
                if matches!(et, Some(0x8100 | 0x88a8)) {
                    d.protocols.push("vlan");
                    et = r.u16_be(off + 2);
                    off += 4;
                }
            }
            (et, off)
        }
        113 => {
            d.protocols.push("sll");
            (r.u16_be(14), 16)
        }
        276 => {
            d.protocols.push("sll2");
            (r.u16_be(0), 20)
        }
        0 | 108 => {
            d.protocols.push("loopback");
            // The family is in the capturing host's byte order (NULL) or big-endian (LOOP).
            let fam = if link_type == 108 { r.u32_be(0) } else { r.u32_le(0).filter(|&v| v < 256).or_else(|| r.u32_be(0)) };
            (
                fam.map(|f| {
                    if f == 2 {
                        0x0800
                    } else if matches!(f, 24 | 28 | 30) {
                        0x86dd
                    } else {
                        0
                    }
                }),
                4,
            )
        }
        12 | 14 | 101 | 228 | 229 => {
            d.protocols.push("raw");
            (r.u8(0).map(|b| if b >> 4 == 6 { 0x86dd } else { 0x0800 }), 0)
        }
        _ => {
            d.note = Some(format!("link type {link_type} ({}) is not decoded", link_name(link_type)));
            return d;
        }
    };
    let payload = data.get(off..).unwrap_or(&[]);
    match ethertype {
        Some(0x0800) => ipv4(payload, &mut d),
        Some(0x86dd) => ipv6(payload, &mut d),
        Some(0x0806) => d.protocols.push("arp"),
        Some(other) => d.note = Some(format!("ethertype {other:#06x} is not decoded")),
        None => d.note = Some("frame too short for its link header".into()),
    }
    d
}

fn ipv4(p: &[u8], d: &mut Decoded) {
    let r = Reader::new(p);
    let (Some(vihl), Some(total), Some(frag), Some(ttl), Some(proto)) = (r.u8(0), r.u16_be(2), r.u16_be(6), r.u8(8), r.u8(9)) else {
        d.note = Some("IPv4 header cut short".into());
        return;
    };
    let ihl = usize::from(vihl & 0x0f) * 4;
    let (Some(src), Some(dst)) = (r.slice(12, 4), r.slice(16, 4)) else { return };
    if vihl >> 4 != 4 || ihl < 20 {
        d.note = Some("not a valid IPv4 header".into());
        return;
    }
    d.protocols.push("ipv4");
    let ip4 = |b: &[u8]| {
        IpAddr::V4(Ipv4Addr::new(
            b.first().copied().unwrap_or(0),
            b.get(1).copied().unwrap_or(0),
            b.get(2).copied().unwrap_or(0),
            b.get(3).copied().unwrap_or(0),
        ))
    };
    let fragment = frag & 0x1fff != 0;
    d.ip = Some(IpInfo { version: 4, src: ip4(src), dst: ip4(dst), protocol: proto, ttl, fragment });
    let end = usize::from(total).clamp(ihl, p.len().max(ihl));
    let body = p.get(ihl..end).unwrap_or(&[]);
    if fragment {
        d.note = Some("IPv4 fragment (transport header is in the first fragment)".into());
        return;
    }
    transport(proto, body, d);
}

fn ipv6(p: &[u8], d: &mut Decoded) {
    let r = Reader::new(p);
    let (Some(plen), Some(mut next), Some(hops), Some(src), Some(dst)) = (r.u16_be(4), r.u8(6), r.u8(7), r.slice(8, 16), r.slice(24, 16)) else {
        d.note = Some("IPv6 header cut short".into());
        return;
    };
    d.protocols.push("ipv6");
    let ip6 = |b: &[u8]| IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(b).unwrap_or([0; 16])));
    let end = (40 + usize::from(plen)).min(p.len());
    let mut off = 40usize;
    let mut fragment = false;
    for _ in 0..8 {
        match next {
            0 | 43 | 60 => {
                let (Some(n), Some(l)) = (r.u8(off), r.u8(off + 1)) else { break };
                next = n;
                off += (usize::from(l) + 1) * 8;
            }
            44 => {
                let (Some(n), Some(fo)) = (r.u8(off), r.u16_be(off + 2)) else { break };
                next = n;
                fragment = fo & 0xfff8 != 0;
                off += 8;
            }
            51 => {
                let (Some(n), Some(l)) = (r.u8(off), r.u8(off + 1)) else { break };
                next = n;
                off += (usize::from(l) + 2) * 4;
            }
            _ => break,
        }
    }
    d.ip = Some(IpInfo { version: 6, src: ip6(src), dst: ip6(dst), protocol: next, ttl: hops, fragment });
    if fragment {
        d.note = Some("IPv6 fragment (transport header is in the first fragment)".into());
        return;
    }
    transport(next, p.get(off..end).unwrap_or(&[]), d);
}

fn tcp_flags(f: u8) -> String {
    let names = [(0x02, "SYN"), (0x10, "ACK"), (0x01, "FIN"), (0x04, "RST"), (0x08, "PSH"), (0x20, "URG")];
    names.iter().filter(|(b, _)| f & b != 0).map(|(_, n)| *n).collect::<Vec<_>>().join(",")
}

fn transport(proto: u8, p: &[u8], d: &mut Decoded) {
    let r = Reader::new(p);
    let empty = Transport {
        protocol: "other",
        src_port: None,
        dst_port: None,
        tcp_flags: None,
        seq: None,
        ack: None,
        window: None,
        icmp_type: None,
        icmp_code: None,
        payload_len: 0,
    };
    match proto {
        6 => {
            let (Some(sp), Some(dp)) = (r.u16_be(0), r.u16_be(2)) else {
                d.note = Some("TCP header cut short".into());
                return;
            };
            d.protocols.push("tcp");
            let (seq, ack, off, flags, win) = (r.u32_be(4), r.u32_be(8), r.u8(12), r.u8(13), r.u16_be(14));
            if win.is_none() {
                d.note = Some("TCP header cut short".into());
            }
            let payload = off.map_or(&[][..], |off| p.get(usize::from(off >> 4) * 4..).unwrap_or(&[]));
            d.transport = Some(Transport {
                protocol: "tcp",
                src_port: Some(sp),
                dst_port: Some(dp),
                tcp_flags: flags.map(tcp_flags),
                seq,
                ack,
                window: win,
                payload_len: payload.len(),
                ..empty
            });
            if (sp == 53 || dp == 53) && payload.len() > 2 {
                dns(payload.get(2..).unwrap_or(&[]), d, "dns");
            } else if !payload.is_empty() {
                tcp_app(payload, d);
            }
        }
        17 => {
            let (Some(sp), Some(dp)) = (r.u16_be(0), r.u16_be(2)) else {
                d.note = Some("UDP header cut short".into());
                return;
            };
            d.protocols.push("udp");
            let payload = p.get(8..).unwrap_or(&[]);
            d.transport = Some(Transport { protocol: "udp", src_port: Some(sp), dst_port: Some(dp), payload_len: payload.len(), ..empty });
            if sp == 53 || dp == 53 {
                dns(payload, d, "dns");
            } else if sp == 5353 || dp == 5353 {
                dns(payload, d, "mdns");
            }
        }
        1 | 58 => {
            let name = if proto == 1 { "icmp" } else { "icmpv6" };
            d.protocols.push(name);
            d.transport = Some(Transport { protocol: name, icmp_type: r.u8(0), icmp_code: r.u8(1), payload_len: p.len().saturating_sub(4), ..empty });
        }
        _ => d.transport = Some(Transport { protocol: "other", payload_len: p.len(), ..empty }),
    }
}

pub fn qtype_name(t: u16) -> String {
    match t {
        1 => "A".into(),
        2 => "NS".into(),
        5 => "CNAME".into(),
        6 => "SOA".into(),
        12 => "PTR".into(),
        15 => "MX".into(),
        16 => "TXT".into(),
        28 => "AAAA".into(),
        33 => "SRV".into(),
        64 => "SVCB".into(),
        65 => "HTTPS".into(),
        255 => "ANY".into(),
        n => format!("TYPE{n}"),
    }
}

/// A (possibly compressed) DNS name at `off`; returns the name and the offset after it.
fn dns_name(msg: &[u8], mut off: usize) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut after: Option<usize> = None;
    for _ in 0..128 {
        let len = *msg.get(off)?;
        match len {
            0 => return Some((if out.is_empty() { ".".into() } else { out }, after.unwrap_or(off + 1))),
            l if l & 0xc0 == 0xc0 => {
                let ptr = usize::from(u16::from_be_bytes([l & 0x3f, *msg.get(off + 1)?]));
                after.get_or_insert(off + 2);
                if ptr >= off {
                    return None; // only backward pointers, so no loops
                }
                off = ptr;
            }
            l if l < 64 => {
                let label = msg.get(off + 1..off + 1 + usize::from(l))?;
                if !out.is_empty() {
                    out.push('.');
                }
                out.extend(label.iter().map(|&c| if c.is_ascii_graphic() { char::from(c) } else { '?' }));
                if out.len() > 255 {
                    return None;
                }
                off += 1 + usize::from(l);
            }
            _ => return None,
        }
    }
    None
}

fn dns(msg: &[u8], d: &mut Decoded, label: &'static str) {
    let r = Reader::new(msg);
    let (Some(id), Some(flags), Some(qd), Some(an)) = (r.u16_be(0), r.u16_be(2), r.u16_be(4), r.u16_be(6)) else { return };
    // Sanity: opcode 0–6 and no absurd counts, so random payload on port 53 isn't misread.
    if (flags >> 11) & 0xf > 6 || qd > 64 || qd == 0 && an == 0 {
        return;
    }
    d.protocols.push(label);
    let mut out = Dns { id, response: flags & 0x8000 != 0, rcode: (flags & 0xf) as u8, questions: Vec::new(), answers: Vec::new(), incomplete: false };
    let mut off = 12usize;
    for _ in 0..qd {
        let Some((name, next)) = dns_name(msg, off) else {
            out.incomplete = true;
            break;
        };
        let Some(qt) = r.u16_be(next) else {
            out.incomplete = true;
            break;
        };
        out.questions.push(DnsQuestion { name, qtype: qtype_name(qt) });
        off = next + 4;
    }
    if !out.incomplete {
        for _ in 0..an.min(64) {
            let Some((name, next)) = dns_name(msg, off) else {
                out.incomplete = true;
                break;
            };
            let (Some(rt), Some(ttl), Some(rdlen)) = (r.u16_be(next), r.u32_be(next + 4), r.u16_be(next + 8)) else {
                out.incomplete = true;
                break;
            };
            let rd_off = next + 10;
            let Some(rd) = r.slice(rd_off, usize::from(rdlen)) else {
                out.incomplete = true;
                break;
            };
            let value = match (rt, rd.len()) {
                (1, 4) => Ipv4Addr::new(
                    rd.first().copied().unwrap_or(0),
                    rd.get(1).copied().unwrap_or(0),
                    rd.get(2).copied().unwrap_or(0),
                    rd.get(3).copied().unwrap_or(0),
                )
                .to_string(),
                (28, 16) => Ipv6Addr::from(<[u8; 16]>::try_from(rd).unwrap_or([0; 16])).to_string(),
                (2 | 5 | 12, _) => dns_name(msg, rd_off).map(|(n, _)| n).unwrap_or_default(),
                (15, _) => dns_name(msg, rd_off + 2).map(|(n, _)| n).unwrap_or_default(),
                (16, _) => {
                    // RFC 1035 §3.3.14: one TXT RDATA holds any number of
                    // length-prefixed character strings, and a zero-length
                    // string does not end the record. Consumers join them all
                    // (RFC 7208 §3.3), so keep every string instead of the
                    // first one and skip the rest of the RDATA.
                    let mut txt = String::new();
                    let mut pos = 0;
                    while let Some(&len) = rd.get(pos) {
                        let len = usize::from(len);
                        let end = (pos + 1).saturating_add(len).min(rd.len());
                        match rd.get(pos + 1..end) {
                            Some(part) => {
                                txt.push_str(&String::from_utf8_lossy(part));
                                pos = end;
                            }
                            None => break,
                        }
                    }
                    txt
                }
                (_, n) => format!("<{n} bytes>"),
            };
            out.answers.push(DnsAnswer { name, rtype: qtype_name(rt), ttl, value });
            off = rd_off + usize::from(rdlen);
        }
    }
    d.dns = Some(out);
}

const METHODS: &[&str] = &["GET", "POST", "PUT", "DELETE", "HEAD", "OPTIONS", "PATCH", "CONNECT", "TRACE"];

/// Remove `user:password@` from an absolute request target.
pub fn strip_userinfo(target: &str) -> String {
    if let Some((scheme, rest)) = target.split_once("://") {
        let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        if let Some(at) = rest.get(..auth_end).and_then(|a| a.rfind('@')) {
            return format!("{scheme}://{}", rest.get(at + 1..).unwrap_or(""));
        }
    }
    target.to_string()
}

fn tcp_app(p: &[u8], d: &mut Decoded) {
    let head = p.get(..p.len().min(16 << 10)).unwrap_or(p);
    let line_end = head.windows(2).position(|w| w == b"\r\n").unwrap_or(head.len().min(2048));
    let line = String::from_utf8_lossy(head.get(..line_end.min(2048)).unwrap_or(&[])).into_owned();
    if let Some(rest) = line.strip_prefix("HTTP/1.") {
        let status = rest.split(' ').nth(1).and_then(|s| s.parse().ok());
        d.protocols.push("http");
        d.http =
            Some(Http { kind: "response", method: None, target: None, status, version: format!("HTTP/1.{}", rest.chars().next().unwrap_or('?')), host: None });
        return;
    }
    let mut parts = line.splitn(3, ' ');
    if let (Some(m), Some(t), Some(v)) = (parts.next(), parts.next(), parts.next())
        && METHODS.contains(&m)
        && v.starts_with("HTTP/1.")
    {
        let lower = String::from_utf8_lossy(head).to_ascii_lowercase();
        let host = lower.find("\r\nhost:").and_then(|i| {
            let start = i + 7;
            let raw = String::from_utf8_lossy(head.get(start..)?).into_owned();
            Some(raw.split("\r\n").next().unwrap_or("").trim().to_string())
        });
        d.protocols.push("http");
        d.http = Some(Http { kind: "request", method: Some(m.to_string()), target: Some(strip_userinfo(t)), status: None, version: v.to_string(), host });
        return;
    }
    tls(p, d);
}

fn tls(p: &[u8], d: &mut Decoded) {
    let r = Reader::new(p);
    let (Some(ct), Some(major), Some(minor)) = (r.u8(0), r.u8(1), r.u8(2)) else { return };
    if !(20..=23).contains(&ct) || major != 3 || minor > 4 {
        return;
    }
    d.protocols.push("tls");
    if ct != 22 || r.u8(5) != Some(1) {
        return;
    }
    let ver = |v: u16| match v {
        0x0300 => "SSL 3.0".to_string(),
        0x0301 => "TLS 1.0".into(),
        0x0302 => "TLS 1.1".into(),
        0x0303 => "TLS 1.2".into(),
        0x0304 => "TLS 1.3".into(),
        v if v & 0x0f0f == 0x0a0a => "GREASE".into(),
        v => format!("{v:#06x}"),
    };
    let mut hello =
        TlsHello { record_version: ver(u16::from_be_bytes([major, minor])), sni: None, alpn: Vec::new(), supported_versions: Vec::new(), partial: false };
    // Record header (5) + handshake header (4) + client_version (2) + random (32).
    let mut off = 5 + 4 + 2 + 32;
    let skip8 = |off: usize| r.u8(off).map(|n| off + 1 + usize::from(n));
    let skip16 = |off: usize| r.u16_be(off).map(|n| off + 2 + usize::from(n));
    let Some(o) = skip8(off).and_then(skip16).and_then(skip8) else {
        hello.partial = true;
        d.tls = Some(hello);
        return;
    };
    off = o;
    let Some(ext_len) = r.u16_be(off) else {
        hello.partial = true;
        d.tls = Some(hello);
        return;
    };
    let end = off + 2 + usize::from(ext_len);
    hello.partial = end > p.len();
    let mut e = off + 2;
    while e + 4 <= end.min(p.len()) {
        let (Some(t), Some(l)) = (r.u16_be(e), r.u16_be(e + 2)) else { break };
        let body = e + 4;
        let Some(data) = r.slice(body, usize::from(l)) else {
            hello.partial = true;
            break;
        };
        let dr = Reader::new(data);
        match t {
            0 => {
                if dr.u8(2) == Some(0)
                    && let Some(n) = dr.u16_be(3).and_then(|n| dr.slice(5, usize::from(n)))
                {
                    hello.sni = Some(n.iter().map(|&c| if c.is_ascii_graphic() { char::from(c) } else { '?' }).collect());
                }
            }
            16 => {
                let mut a = 2usize;
                while let Some(n) = dr.u8(a) {
                    let Some(proto) = dr.slice(a + 1, usize::from(n)) else { break };
                    hello.alpn.push(String::from_utf8_lossy(proto).into_owned());
                    a += 1 + usize::from(n);
                    if hello.alpn.len() >= 16 {
                        break;
                    }
                }
            }
            43 => {
                let mut a = 1usize;
                while let Some(v) = dr.u16_be(a) {
                    let name = ver(v);
                    if name != "GREASE" {
                        hello.supported_versions.push(name);
                    }
                    a += 2;
                    if hello.supported_versions.len() >= 16 {
                        break;
                    }
                }
            }
            _ => {}
        }
        e = body + usize::from(l);
    }
    d.tls = Some(hello);
}

// ---------------------------------------------------------------------------------------------
// Flows and summaries

/// Assigns stable 1-based flow ids in order of first appearance.
#[derive(Debug, Default)]
pub struct FlowTable {
    ids: HashMap<FlowKey, usize>,
    /// Frames whose flow arrived after `MAX_FLOWS` and got no id.
    pub untracked: u64,
}

impl FlowTable {
    /// The id of `key`, creating it when there is room.
    pub fn id_for(&mut self, key: FlowKey) -> Option<usize> {
        if let Some(&id) = self.ids.get(&key) {
            return Some(id);
        }
        if self.ids.len() >= MAX_FLOWS {
            self.untracked += 1;
            return None;
        }
        let id = self.ids.len() + 1;
        self.ids.insert(key, id);
        Some(id)
    }
}

/// "addr:port", with brackets around IPv6 addresses; the port is omitted when zero.
pub fn endpoint((ip, port): (IpAddr, u16)) -> String {
    match (ip, port) {
        (IpAddr::V6(v6), 0) => v6.to_string(),
        (IpAddr::V6(v6), p) => format!("[{v6}]:{p}"),
        (v4, 0) => v4.to_string(),
        (v4, p) => format!("{v4}:{p}"),
    }
}

pub fn ip_protocol_name(p: u8) -> &'static str {
    match p {
        1 => "icmp",
        6 => "tcp",
        17 => "udp",
        58 => "icmpv6",
        _ => "other",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Flow {
    pub id: usize,
    pub protocol: &'static str,
    pub a: String,
    pub b: String,
    pub packets: u64,
    pub bytes: u64,
    pub packets_a_to_b: u64,
    pub packets_b_to_a: u64,
    pub first_frame: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_ts_ns: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_ts_ns: Option<u64>,
    /// Application protocols seen in it (dns, http, tls…).
    pub applications: BTreeSet<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Count {
    pub value: String,
    pub count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub format: &'static str,
    pub version: String,
    pub interfaces: Vec<Interface>,
    pub packets: u64,
    pub bytes: u64,
    pub first_ts_ns: Option<u64>,
    pub last_ts_ns: Option<u64>,
    /// Frames per protocol label.
    pub protocols: Vec<Count>,
    /// Frames per full protocol stack (eth:ipv4:udp:dns).
    pub stacks: Vec<Count>,
    /// Frames per IP address (source and destination each count once per frame).
    pub endpoints: Vec<Count>,
    pub flows: Vec<Flow>,
    pub flows_untracked_frames: u64,
    /// "name (TYPE)" asked in DNS queries.
    pub dns_queries: Vec<Count>,
    /// "name TYPE value" from DNS responses.
    pub dns_answers: Vec<Count>,
    /// "question A|AAAA address" from DNS responses: each address answered, keyed by the response's first
    /// question (so CNAME chains end at the name that was asked), as Wireshark's fields report them.
    pub dns_addresses: Vec<Count>,
    /// "METHOD host target" from HTTP/1.x requests.
    pub http_requests: Vec<Count>,
    /// Server names from TLS ClientHellos.
    pub tls_sni: Vec<Count>,
    pub tls_alpn: Vec<Count>,
    /// Frames whose network layer was not decoded (other link types, ARP, other ethertypes).
    pub undecoded_frames: u64,
    /// Inventories that hit `MAX_INVENTORY` distinct values.
    pub capped_inventories: Vec<&'static str>,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct Tally {
    map: BTreeMap<String, u64>,
    capped: bool,
}

impl Tally {
    fn add(&mut self, k: String) {
        if let Some(v) = self.map.get_mut(&k) {
            *v += 1;
        } else if self.map.len() < MAX_INVENTORY {
            self.map.insert(k, 1);
        } else {
            self.capped = true;
        }
    }

    fn done(self, name: &'static str, capped: &mut Vec<&'static str>) -> Vec<Count> {
        if self.capped {
            capped.push(name);
        }
        let mut v: Vec<Count> = self.map.into_iter().map(|(value, count)| Count { value, count }).collect();
        v.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
        v
    }
}

/// Read every frame once and summarize the capture. `cancelled` is polled every 4096 frames.
pub fn summarize(data: &[u8], cancelled: &dyn Fn() -> bool) -> Result<Summary, FormatError> {
    let mut cap = Capture::new(data)?;
    let mut s = Summary::default();
    let mut flows = FlowTable::default();
    let mut flow_rows: Vec<Flow> = Vec::new();
    let (mut protocols, mut stacks, mut endpoints) = (Tally::default(), Tally::default(), Tally::default());
    let (mut queries, mut answers, mut http, mut sni, mut alpn) = (Tally::default(), Tally::default(), Tally::default(), Tally::default(), Tally::default());
    let mut addresses = Tally::default();
    for f in cap.by_ref() {
        if f.number % 4096 == 0 && cancelled() {
            return Err(FormatError::Malformed("cancelled".into()));
        }
        s.packets += 1;
        s.bytes += u64::from(f.original_len);
        if let Some(t) = f.ts_ns {
            s.first_ts_ns = Some(s.first_ts_ns.map_or(t, |x| x.min(t)));
            s.last_ts_ns = Some(s.last_ts_ns.map_or(t, |x| x.max(t)));
        }
        let d = decode(f.link_type, f.data);
        for (i, p) in d.protocols.iter().enumerate() {
            if !d.protocols[..i].contains(p) {
                protocols.add((*p).to_string());
            }
        }
        stacks.add(d.stack());
        let Some(ip) = &d.ip else {
            s.undecoded_frames += 1;
            continue;
        };
        endpoints.add(ip.src.to_string());
        if ip.dst != ip.src {
            endpoints.add(ip.dst.to_string());
        }
        if let Some((key, forward)) = d.flow_key()
            && let Some(id) = flows.id_for(key)
        {
            if id > flow_rows.len() {
                flow_rows.push(Flow {
                    id,
                    protocol: ip_protocol_name(key.protocol),
                    a: endpoint(key.a),
                    b: endpoint(key.b),
                    packets: 0,
                    bytes: 0,
                    packets_a_to_b: 0,
                    packets_b_to_a: 0,
                    first_frame: f.number,
                    first_ts_ns: f.ts_ns,
                    last_ts_ns: f.ts_ns,
                    applications: BTreeSet::new(),
                });
            }
            if let Some(row) = flow_rows.get_mut(id - 1) {
                row.packets += 1;
                row.bytes += u64::from(f.original_len);
                if forward {
                    row.packets_a_to_b += 1;
                } else {
                    row.packets_b_to_a += 1;
                }
                if f.ts_ns.is_some() {
                    row.last_ts_ns = f.ts_ns;
                }
                for app in d.protocols.iter().skip_while(|p| !matches!(**p, "tcp" | "udp")).skip(1) {
                    row.applications.insert(app);
                }
            }
        }
        if let Some(dns) = &d.dns {
            if !dns.response {
                for q in &dns.questions {
                    queries.add(format!("{} ({})", q.name, q.qtype));
                }
            }
            for a in &dns.answers {
                answers.add(format!("{} {} {}", a.name, a.rtype, a.value));
            }
            if dns.response
                && let Some(q) = dns.questions.first()
            {
                for a in dns.answers.iter().filter(|a| matches!(a.rtype.as_str(), "A" | "AAAA") && a.value.parse::<IpAddr>().is_ok()) {
                    addresses.add(format!("{} {} {}", q.name, a.rtype, a.value));
                }
            }
        }
        if let Some(h) = &d.http
            && h.kind == "request"
        {
            http.add(format!("{} {} {}", h.method.as_deref().unwrap_or("?"), h.host.as_deref().unwrap_or("-"), h.target.as_deref().unwrap_or("")));
        }
        if let Some(t) = &d.tls {
            if let Some(n) = &t.sni {
                sni.add(n.clone());
            }
            for a in &t.alpn {
                alpn.add(a.clone());
            }
        }
    }
    let mut capped = Vec::new();
    s.format = cap.format;
    s.version = cap.version.clone();
    s.interfaces = cap.interfaces.clone();
    s.protocols = protocols.done("protocols", &mut capped);
    s.stacks = stacks.done("stacks", &mut capped);
    s.endpoints = endpoints.done("endpoints", &mut capped);
    s.dns_queries = queries.done("dns_queries", &mut capped);
    s.dns_answers = answers.done("dns_answers", &mut capped);
    s.dns_addresses = addresses.done("dns_addresses", &mut capped);
    s.http_requests = http.done("http_requests", &mut capped);
    s.tls_sni = sni.done("tls_sni", &mut capped);
    s.tls_alpn = alpn.done("tls_alpn", &mut capped);
    s.capped_inventories = capped;
    s.flows = flow_rows;
    s.flows_untracked_frames = flows.untracked;
    s.truncated = cap.truncated;
    s.warnings = cap.warnings;
    Ok(s)
}

/// `YYYY-MM-DDThh:mm:ss.nnnnnnnnnZ` for nanoseconds since the Unix epoch.
pub fn iso8601(ns: u64) -> String {
    let secs = ns / 1_000_000_000;
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil-from-days (proleptic Gregorian), valid for every u64 day count we can reach.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:09}Z", rem / 3600, rem % 3600 / 60, rem % 60, ns % 1_000_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_anything_testkit::pcap::{dns_query, eth, ipv4_pkt, pcap, pcapng, sample_frames, udp};

    #[test]
    fn tcp_cut_inside_header_keeps_ports() {
        // Ports sit in the first 4 bytes; a cut later in the header must not drop them (as UDP).
        let mut tcp = Vec::new();
        tcp.extend(41000u16.to_be_bytes());
        tcp.extend(9000u16.to_be_bytes());
        tcp.extend([0, 0, 0, 1, 0, 0, 0, 0, 0x50, 0x18, 4, 0, 0, 0, 0, 0]);
        let mut frame = eth(0x0800, &ipv4_pkt(6, [10, 0, 0, 1], [10, 0, 0, 2], &tcp));
        frame.truncate(14 + 20 + 11);
        let d = decode(1, &frame);
        let t = d.transport.unwrap();
        assert_eq!((t.src_port, t.dst_port), (Some(41000), Some(9000)));
        assert_eq!(t.seq, Some(1));
        assert_eq!(t.ack, None);
        assert_eq!(t.window, None);
        assert!(d.protocols.contains(&"tcp"));
    }

    /// One-answer DNS response for `example.com` with a TXT answer carrying `rdata`.
    fn txt_response(rdata: &[u8]) -> Vec<u8> {
        let mut m = vec![0x12, 0x34, 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0];
        m.extend(b"\x07example\x03com\x00\x00\x01\x00\x01");
        m.extend([0xc0, 12, 0, 16, 0, 1, 0, 0, 0x00, 0x3c]);
        m.extend((rdata.len() as u16).to_be_bytes());
        m.extend(rdata);
        m
    }

    #[test]
    fn dns_txt_answer_keeps_every_character_string() {
        // RFC 1035 §3.3.14: one TXT RDATA holds any number of length-prefixed
        // character strings. Only the first used to survive (issue #330).
        let rdata = [
            5, b'f', b'i', b'r', b's', b't', // "first"
            6, b's', b'e', b'c', b'o', b'n', b'd', // "second"
        ];
        let frame = eth(0x0800, &ipv4_pkt(17, [8, 8, 8, 8], [192, 168, 1, 10], &udp(53, 50000, &txt_response(&rdata))));
        let d = decode(1, &frame);
        let dns = d.dns.unwrap();
        assert_eq!(dns.answers.len(), 1);
        assert_eq!(dns.answers[0].rtype, "TXT");
        assert_eq!(dns.answers[0].value, "firstsecond");
        assert!(!dns.incomplete);
    }

    #[test]
    fn dns_txt_empty_first_string_keeps_later_content() {
        // A zero-length character string does not end the record.
        let rdata = [0, 6, b's', b'e', b'c', b'o', b'n', b'd'];
        let frame = eth(0x0800, &ipv4_pkt(17, [8, 8, 8, 8], [192, 168, 1, 10], &udp(53, 50000, &txt_response(&rdata))));
        let d = decode(1, &frame);
        let dns = d.dns.unwrap();
        assert_eq!(dns.answers.len(), 1);
        assert_eq!(dns.answers[0].value, "second");
        assert!(!dns.incomplete);
    }

    fn classic_link_word(link_word: u32, le: bool, nanos: bool, trailer: usize) {
        let link = link_word & 0xffff;
        let mut frame = sample_frames()[1].clone();
        if link == 101 {
            frame.drain(..14);
        }
        frame.extend(vec![0xab; trailer]);
        let word = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let short = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let mut data = word(if nanos { CLASSIC_NS } else { CLASSIC_US }).to_vec();
        data.extend(short(2));
        data.extend(short(4));
        data.extend([0; 8]);
        data.extend(word(65535));
        data.extend(word(link_word));
        data.extend(word(1_700_000_000));
        data.extend(word(500));
        data.extend(word(frame.len() as u32));
        data.extend(word(frame.len() as u32));
        data.extend(&frame);

        let mut cap = Capture::new(&data).unwrap();
        assert_eq!(cap.interfaces[0].link_type, link);
        assert_eq!(cap.interfaces[0].link_name, if link == 1 { "Ethernet" } else { "raw IP" });
        let packet = cap.next().unwrap();
        assert_eq!(packet.link_type, link);
        assert_eq!(packet.data, frame);
        assert_eq!(packet.ts_ns, Some(1_700_000_000_000_000_000 + if nanos { 500 } else { 500_000 }));
        let d = decode(packet.link_type, packet.data);
        assert_eq!(d.stack(), if link == 1 { "eth:ipv4:udp:dns" } else { "raw:ipv4:udp:dns" });
        let ip = d.ip.unwrap();
        assert_eq!(ip.src.to_string(), "8.8.8.8");
        assert_eq!(ip.dst.to_string(), "192.168.1.10");
        let dns = d.dns.unwrap();
        assert!(dns.response);
        assert_eq!(dns.answers[0].value, "93.184.216.34");
        assert!(cap.next().is_none());

        let s = summarize(&data, &|| false).unwrap();
        assert_eq!(s.packets, 1);
        assert_eq!(s.undecoded_frames, 0);
        assert_eq!(s.dns_answers[0].value, "example.com A 93.184.216.34");
        assert_eq!(s.flows[0].protocol, "udp");
        assert!(s.warnings.is_empty());
    }

    #[test]
    fn fcs_bits_do_not_change_the_ethernet_link_type() {
        classic_link_word(0x24000001, true, false, 4);
    }

    #[test]
    fn fcs_bits_are_masked_in_big_endian_files() {
        classic_link_word(0x24000001, false, false, 4);
    }

    #[test]
    fn fcs_bits_are_masked_in_nanosecond_files() {
        classic_link_word(0x24000001, true, true, 4);
    }

    #[test]
    fn fcs_bits_are_masked_in_big_endian_nanosecond_files() {
        classic_link_word(0x24000001, false, true, 4);
        classic_link_word(1, false, true, 0);
    }

    #[test]
    fn fcs_bits_do_not_change_big_endian_nanosecond_raw_ip() {
        classic_link_word(0x24000065, false, true, 4);
        classic_link_word(101, false, true, 0);
    }

    #[test]
    fn a_one_word_fcs_length_is_masked() {
        classic_link_word(0x14000001, true, false, 2);
    }

    #[test]
    fn fcs_bits_do_not_change_the_raw_ip_link_type() {
        classic_link_word(0x24000065, true, false, 4);
    }

    #[test]
    fn ethernet_without_fcs_bits_still_decodes() {
        classic_link_word(1, true, false, 0);
    }

    #[test]
    fn raw_ip_without_fcs_bits_still_decodes() {
        classic_link_word(101, true, false, 0);
    }

    #[test]
    fn decodes_dns_tls_http() {
        let frames = sample_frames();
        let d = decode(1, &frames[1]);
        assert_eq!(d.stack(), "eth:ipv4:udp:dns");
        let dns = d.dns.unwrap();
        assert!(dns.response);
        assert_eq!(dns.questions[0].name, "example.com");
        assert_eq!(dns.answers[0].value, "93.184.216.34");
        assert_eq!(dns.answers[0].ttl, 3600);

        let d = decode(1, &frames[4]);
        assert_eq!(d.stack(), "eth:ipv4:tcp:tls");
        let t = d.tls.unwrap();
        assert_eq!(t.sni.as_deref(), Some("example.com"));
        assert_eq!(t.alpn, vec!["h2", "http/1.1"]);
        assert_eq!(t.supported_versions, vec!["TLS 1.3", "TLS 1.2"]);
        assert!(!t.partial);

        let d = decode(1, &frames[5]);
        let h = d.http.unwrap();
        assert_eq!(h.method.as_deref(), Some("GET"));
        assert_eq!(h.host.as_deref(), Some("example.com"));
        assert_eq!(h.target.as_deref(), Some("http://example.com/a?b=1"));
        assert_eq!(decode(1, &frames[3]).transport.unwrap().tcp_flags.as_deref(), Some("SYN,ACK"));
        assert_eq!(decode(1, &frames[6]).stack(), "eth:arp");
    }

    #[test]
    fn summarizes_pcap_and_pcapng_alike() {
        let frames = sample_frames();
        for data in [pcap(&frames), pcapng(&frames)] {
            let s = summarize(&data, &|| false).unwrap();
            assert_eq!(s.packets, 7);
            assert_eq!(s.flows.len(), 3, "{:?}", s.flows);
            assert_eq!(s.flows[0].protocol, "udp");
            assert_eq!(s.flows[0].packets, 2);
            assert_eq!((s.flows[0].packets_a_to_b, s.flows[0].packets_b_to_a), (1, 1));
            assert!(s.flows[1].applications.contains("tls"));
            assert_eq!(s.dns_queries[0].value, "example.com (A)");
            assert_eq!(s.dns_answers[0].value, "example.com A 93.184.216.34");
            assert_eq!(s.tls_sni[0].value, "example.com");
            assert_eq!(s.http_requests[0].value, "GET example.com http://example.com/a?b=1");
            assert_eq!(s.undecoded_frames, 1);
            assert!(s.warnings.is_empty(), "{:?}", s.warnings);
            assert_eq!(s.interfaces[0].link_type, 1);
            assert!(s.first_ts_ns.unwrap() >= 1_700_000_000_000_000_000);
        }
        assert_eq!(summarize(&pcap(&frames), &|| false).unwrap().format, "pcap");
        assert_eq!(iso8601(1_700_000_000_000_000_500), "2023-11-14T22:13:20.000000500Z");
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000000000Z");
    }

    #[test]
    fn qinq_frames_count_vlan_once_per_frame() {
        let mut payload = vec![0x00, 0x0a, 0x81, 0x00, 0x00, 0x14, 0x08, 0x00];
        payload.extend(ipv4_pkt(17, [10, 0, 0, 1], [10, 0, 0, 2], &udp(40000, 9999, b"ping")));
        let frame = eth(0x88a8, &payload);
        let s = summarize(&pcap(&[frame.clone(), frame.clone(), frame]), &|| false).unwrap();
        let count = |name: &str| s.protocols.iter().find(|c| c.value == name).map(|c| c.count);
        assert_eq!(s.stacks[0].value, "eth:vlan:vlan:ipv4:udp");
        assert_eq!(count("vlan"), Some(3));
        assert_eq!(count("eth"), Some(3));
        assert_eq!(count("ipv4"), Some(3));
    }

    #[test]
    fn pcapng_interface_tsoffset_adjusts_timestamps() {
        let base = pcapng(&sample_frames());
        for offset_s in [3600i64, -50i64] {
            let mut data = base[..52].to_vec();
            data.extend(14u16.to_le_bytes());
            data.extend(8u16.to_le_bytes());
            data.extend(offset_s.to_le_bytes());
            data.extend(&base[52..]);
            data[32..36].copy_from_slice(&44u32.to_le_bytes());
            data[68..72].copy_from_slice(&44u32.to_le_bytes());
            let expected0 = (1_700_000_000_000_000_000i128 + i128::from(offset_s) * 1_000_000_000) as u64;
            let frames: Vec<_> = Capture::new(&data).unwrap().collect();
            assert_eq!(frames[0].ts_ns, Some(expected0));
            let s = summarize(&data, &|| false).unwrap();
            assert_eq!((s.first_ts_ns, s.last_ts_ns), (Some(expected0), Some(expected0 + 6)));
            assert_eq!((s.flows[0].first_ts_ns, s.flows[0].last_ts_ns), (Some(expected0), Some(expected0 + 1)));
        }
    }

    #[test]
    fn ipv6_sll_and_raw_links() {
        let mut v6 = vec![0x60, 0, 0, 0];
        let payload = udp(1000, 53, &dns_query());
        v6.extend((payload.len() as u16).to_be_bytes());
        v6.extend([17, 64]);
        v6.extend([0x20, 0x01, 0x0d, 0xb8].iter().chain([0u8; 11].iter()).chain([1u8].iter()));
        v6.extend([0x20, 0x01, 0x0d, 0xb8].iter().chain([0u8; 11].iter()).chain([2u8].iter()));
        v6.extend(&payload);
        let d = decode(101, &v6);
        assert_eq!(d.stack(), "raw:ipv6:udp:dns");
        assert_eq!(d.ip.unwrap().src.to_string(), "2001:db8::1");
        let mut sll = vec![0u8; 14];
        sll.extend(0x86ddu16.to_be_bytes());
        sll.extend(&v6);
        assert_eq!(decode(113, &sll).stack(), "sll:ipv6:udp:dns");
        assert!(decode(105, &v6).note.is_some());
    }

    #[test]
    fn hostile_input_never_panics() {
        assert!(Capture::new(b"").is_err());
        assert!(Capture::new(b"not a capture at all").is_err());
        let data = pcapng(&sample_frames());
        for cut in [0, 10, 28, 40, 100, data.len() / 2, data.len() - 3] {
            let _ = summarize(data.get(..cut).unwrap(), &|| false);
        }
        let mut bad = pcap(&sample_frames());
        bad[32..36].copy_from_slice(&[0xff; 4]); // incl_len of the first record far past the end
        let s = summarize(&bad, &|| false).unwrap();
        assert_eq!(s.packets, 0);
        assert!(!s.warnings.is_empty());
        for seed in 0..200u64 {
            let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let noise: Vec<u8> = (0..300)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    x as u8
                })
                .collect();
            for link in [0, 1, 101, 113, 276, 228] {
                let _ = decode(link, &noise);
            }
            let mut f = sample_frames();
            f.push(noise);
            let _ = summarize(&pcap(&f), &|| false);
        }
        // DNS compression loops are refused.
        assert!(dns_name(&[0xc0, 0x00], 0).is_none());
    }
}
