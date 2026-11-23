//! Hosts of `http` and `https` addresses: RFC 3986 §3.2.2, read with the WHATWG URL Standard's host rules
//! (§3.5) where RFC 3986 leaves room.
//!
//! What is accepted is deliberately the part on which both documents and DNS agree:
//!
//! * a registered name, percent-decoded, in ASCII, lower-cased, made of letters, digits, `-`, `_` and dots,
//!   with labels of 1 to 63 octets and at most 253 octets in all (one final dot allowed);
//! * an IPv4 address written exactly as `std::net::Ipv4Addr` writes it: four decimal numbers without leading
//!   zeros (RFC 3986 `IPv4address`);
//! * an IPv6 address in brackets, read by `std::net::Ipv6Addr` (RFC 3986 `IPv6address`), without zone
//!   identifiers, without `IPvFuture`, and with an embedded IPv4 address only as its last 32 bits in plain
//!   dotted form.
//!
//! A name whose last label is numeric (the WHATWG "ends in a number" test) but which is not a plain dotted
//! IPv4 address, such as `127.1`, `0x7f.0.0.1` or `2130706433`, is refused: the WHATWG parser reads those
//! as IPv4 addresses while RFC 3986 reads them as names, and RFC 3986 §7.4 warns about exactly this.

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

use crate::address::AddressError;
use crate::percent;

/// The longest registered name, in octets, without a final dot (the DNS limit; RFC 3986 §3.2.2 notes that
/// a registered name meant for DNS follows DNS syntax).
const MAX_NAME_LEN: usize = 253;
/// The longest label of a registered name, in octets.
const MAX_LABEL_LEN: usize = 63;
/// The longest text accepted between the brackets of an IPv6 literal: eight groups of four digits and seven
/// colons are 39 characters; a trailing dotted IPv4 address adds at most 6 more.
const MAX_IPV6_TEXT_LEN: usize = 45;

/// How close to this computer a host is, as far as its text tells (no name is resolved).
///
/// The order matters: [`HostScope::Public`] < [`HostScope::PrivateNetwork`] < [`HostScope::ThisComputer`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostScope {
    /// Any registered name other than the `localhost` family, and every other IP address.
    Public,
    /// A private, shared or link-local network: IPv4 `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`,
    /// `100.64.0.0/10` (RFC 6598), `169.254.0.0/16`; IPv6 `fc00::/7`, `fe80::/10`, `fec0::/10`.
    PrivateNetwork,
    /// This computer: `localhost` and names under it (with or without a final dot), `127.0.0.0/8`,
    /// `0.0.0.0/8`, `::1`, `::`, and those IPv4 addresses mapped or embedded in IPv6 (`::ffff:0:0/96`,
    /// `::/96`, `64:ff9b::/96`).
    ThisComputer,
}

fn ipv4_scope(address: Ipv4Addr) -> HostScope {
    let [first, second, _, _] = address.octets();
    if address.is_loopback() || first == 0 {
        HostScope::ThisComputer
    } else if address.is_private() || address.is_link_local() || (first == 100 && (64..=127).contains(&second)) {
        HostScope::PrivateNetwork
    } else {
        HostScope::Public
    }
}

fn ipv6_scope(address: Ipv6Addr) -> HostScope {
    // `to_ipv4` covers ::, ::1, the IPv4-compatible ::/96 and the IPv4-mapped ::ffff:0:0/96 forms; :: and ::1
    // come out as 0.0.0.0 and 0.0.0.1, both on this computer.
    if let Some(embedded) = address.to_ipv4() {
        return ipv4_scope(embedded);
    }
    let [s0, s1, s2, s3, s4, s5, s6, s7] = address.segments();
    if [s0, s1, s2, s3, s4, s5] == [0x0064, 0xFF9B, 0, 0, 0, 0] {
        // The NAT64 well-known prefix (RFC 6052 §2.1) carries an IPv4 address in its last 32 bits.
        let [a, b] = s6.to_be_bytes();
        let [c, d] = s7.to_be_bytes();
        return ipv4_scope(Ipv4Addr::new(a, b, c, d));
    }
    // Unique local fc00::/7, link-local fe80::/10 and the deprecated site-local fec0::/10.
    if address.is_unique_local() || address.is_unicast_link_local() || s0 & 0xFFC0 == 0xFEC0 { HostScope::PrivateNetwork } else { HostScope::Public }
}

/// The host of an address.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Host {
    /// A registered name, lower-case ASCII (`example.com`).
    Name(String),
    /// An IPv4 address.
    Ipv4(Ipv4Addr),
    /// An IPv6 address.
    Ipv6(Ipv6Addr),
}

impl Host {
    /// Reads the host part of an authority (RFC 3986 §3.2.2), brackets included for IPv6.
    pub(crate) fn parse(input: &str) -> Result<Self, AddressError> {
        if input.is_empty() {
            return Err(AddressError::EmptyHost);
        }
        if let Some(inner) = input.strip_prefix('[') {
            let inner = inner.strip_suffix(']').ok_or(AddressError::InvalidIpv6)?;
            return ipv6_between_brackets(inner).map(Self::Ipv6);
        }
        let decoded = percent::decode_bytes(input).map_err(|_| AddressError::InvalidHost)?;
        if decoded.iter().any(|b| !b.is_ascii()) {
            return Err(AddressError::InternationalHost);
        }
        let name = String::from_utf8(decoded).map_err(|_| AddressError::InvalidHost)?.to_ascii_lowercase();
        if name.is_empty() {
            return Err(AddressError::EmptyHost);
        }
        if !name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')) {
            return Err(AddressError::InvalidHost);
        }
        if last_label_is_numeric(&name) {
            return plain_dotted_ipv4(&name).map(Self::Ipv4).ok_or(AddressError::AmbiguousNumericHost);
        }
        check_dns_lengths(&name)?;
        Ok(Self::Name(name))
    }

    /// Whether this is a loopback host in the sense of the transport rule: the name `localhost` exactly, an
    /// IPv4 address in `127.0.0.0/8`, or the IPv6 address `::1`.
    ///
    /// `sub.localhost`, `localhost.` and IPv4-mapped IPv6 addresses are not loopback here.
    pub fn is_loopback(&self) -> bool {
        match self {
            Self::Name(name) => name == "localhost",
            Self::Ipv4(address) => address.is_loopback(),
            Self::Ipv6(address) => address.is_loopback(),
        }
    }

    /// How close to this computer the host is ([`HostScope`]). Broader than [`Host::is_loopback`]: it serves
    /// to stop a remote server from sending the program towards this computer or its network, not to allow
    /// plain `http`.
    pub fn scope(&self) -> HostScope {
        match self {
            Self::Name(name) => {
                let name = name.strip_suffix('.').unwrap_or(name);
                if name == "localhost" || name.ends_with(".localhost") { HostScope::ThisComputer } else { HostScope::Public }
            }
            Self::Ipv4(address) => ipv4_scope(*address),
            Self::Ipv6(address) => ipv6_scope(*address),
        }
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(name) => f.write_str(name),
            Self::Ipv4(address) => write!(f, "{address}"),
            Self::Ipv6(address) => {
                f.write_str("[")?;
                write_ipv6_in_hex(f, address)?;
                f.write_str("]")
            }
        }
    }
}

impl fmt::Debug for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Whether a name would be read as a number by the WHATWG host parser ("ends in a number", §3.5): its last
/// label, a final dot aside, is all decimal digits or `0x` followed by hex digits.
fn last_label_is_numeric(name: &str) -> bool {
    let trimmed = name.strip_suffix('.').unwrap_or(name);
    let last = trimmed.rsplit('.').next().unwrap_or("");
    if last.is_empty() {
        return false;
    }
    if last.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    match last.strip_prefix("0x").or_else(|| last.strip_prefix("0X")) {
        Some(hex) => hex.bytes().all(|b| b.is_ascii_hexdigit()),
        None => false,
    }
}

/// An IPv4 host, accepted only when the text is exactly the address's own dotted form: `std::net` reads it,
/// and writing it back must give the same text. That refuses leading zeros, a final dot, and every shortened,
/// octal or hexadecimal spelling (RFC 3986 §3.2.2 `dec-octet`, §7.4).
fn plain_dotted_ipv4(text: &str) -> Option<Ipv4Addr> {
    let address: Ipv4Addr = text.parse().ok()?;
    (address.to_string() == text).then_some(address)
}

/// An IPv6 literal, the text between the brackets, read by `std::net` once the forms this crate refuses are
/// set aside: zone identifiers (RFC 6874's `%25…`, which the WHATWG parser refuses too), `IPvFuture`
/// (`v1.…`), and an embedded IPv4 part that is not the last 32 bits in plain dotted form.
fn ipv6_between_brackets(text: &str) -> Result<Ipv6Addr, AddressError> {
    if text.is_empty() || text.len() > MAX_IPV6_TEXT_LEN || text.contains('%') || text.starts_with(['v', 'V']) {
        return Err(AddressError::InvalidIpv6);
    }
    if text.contains('.') {
        let (head, dotted) = text.rsplit_once(':').ok_or(AddressError::InvalidIpv6)?;
        if head.contains('.') || plain_dotted_ipv4(dotted).is_none() {
            return Err(AddressError::InvalidIpv6);
        }
    }
    text.parse::<Ipv6Addr>().map_err(|_| AddressError::InvalidIpv6)
}

fn check_dns_lengths(name: &str) -> Result<(), AddressError> {
    let body = name.strip_suffix('.').unwrap_or(name);
    if body.is_empty() || body.len() > MAX_NAME_LEN {
        return Err(AddressError::InvalidHost);
    }
    if body.split('.').any(|label| label.is_empty() || label.len() > MAX_LABEL_LEN) {
        return Err(AddressError::InvalidHost);
    }
    Ok(())
}

/// Writes an IPv6 address in the hexadecimal form of RFC 5952 §4, as the WHATWG IPv6 serializer (§3.6) does:
/// lower-case hex without leading zeros, the first longest run of two or more zero groups written as `::`,
/// and no dotted IPv4 tail (`std`'s `Display` writes IPv4-mapped addresses with one).
fn write_ipv6_in_hex(f: &mut fmt::Formatter<'_>, address: &Ipv6Addr) -> fmt::Result {
    let groups = address.segments();
    let mut best: Option<(usize, usize)> = None;
    let mut run_start: Option<usize> = None;
    for (index, group) in groups.iter().enumerate() {
        if *group == 0 {
            let start = *run_start.get_or_insert(index);
            let len = index.saturating_sub(start).saturating_add(1);
            if len >= 2 && best.is_none_or(|(_, best_len)| len > best_len) {
                best = Some((start, len));
            }
        } else {
            run_start = None;
        }
    }
    let mut index = 0usize;
    let mut need_colon = false;
    while let Some(group) = groups.get(index) {
        if let Some((start, len)) = best
            && index == start
        {
            f.write_str("::")?;
            index = index.saturating_add(len);
            need_colon = false;
            continue;
        }
        if need_colon {
            f.write_str(":")?;
        }
        write!(f, "{group:x}")?;
        need_colon = true;
        index = index.saturating_add(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
