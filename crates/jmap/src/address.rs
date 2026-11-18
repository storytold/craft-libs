//! Absolute `http` and `https` addresses (RFC 3986, RFC 9110 §4.2), reference resolution (RFC 3986 §5.2),
//! origins, and the transport rule that decides whether an address may carry credentials.
//!
//! An [`Address`] is always absolute, always `http` or `https`, always has a host, never has user information
//! or a fragment, and is kept in a normal form (RFC 3986 §6.2.2 and §6.2.3, RFC 9110 §4.2.3): lower-case scheme
//! and host, no default port, percent-escapes in upper case with escaped unreserved characters decoded, no
//! dot-segments, and `/` for an empty path.
//!
//! Its `Debug` form shows the origin only, so that a path or a query holding something private never reaches
//! a log by accident. [`Address::as_str`] gives the full form, for the request itself.

use std::fmt;

use crate::host::Host;
use crate::percent::{self, is_sub_delim, is_unreserved};

/// The longest address accepted, in octets.
pub const MAX_ADDRESS_LEN: usize = 8 * 1024;

/// An address scheme this crate can use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scheme {
    /// `http` (RFC 9110 §4.2.1); only usable towards a loopback host.
    Http,
    /// `https` (RFC 9110 §4.2.2).
    Https,
}

impl Scheme {
    /// The scheme's name, lower case.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }

    /// The TCP port used when an address names none (RFC 9110 §4.2.1 and §4.2.2).
    pub fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
        }
    }
}

/// Why a text is not a usable address.
///
/// No variant carries the text that was refused: it may hold a password or a token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AddressError {
    /// Nothing was given.
    Empty,
    /// Longer than [`MAX_ADDRESS_LEN`].
    TooLong,
    /// A space, a control character, a character outside ASCII, or a character no URI may contain.
    ForbiddenCharacter,
    /// A backslash, which browsers read as a slash and RFC 3986 does not allow.
    Backslash,
    /// No scheme where an absolute address was needed.
    MissingScheme,
    /// A scheme other than `http` or `https`.
    UnsupportedScheme,
    /// No `//` and authority after the scheme.
    MissingAuthority,
    /// The host is empty (RFC 9110 §4.2.1).
    EmptyHost,
    /// User information (`name@` or `name:password@`) before the host (RFC 9110 §4.2.4).
    UserInfo,
    /// The host is not a usable name.
    InvalidHost,
    /// The host is outside ASCII; it must be given in its `xn--` form.
    InternationalHost,
    /// A numeric host that is not a plain dotted IPv4 address (`127.1`, `0x7f.0.0.1`, …).
    AmbiguousNumericHost,
    /// A bracketed IPv6 literal that does not parse.
    InvalidIpv6,
    /// The port is not a number from 1 to 65535.
    InvalidPort,
    /// The path holds a character or an escape that RFC 3986 §3.3 does not allow.
    InvalidPath,
    /// The query holds a character or an escape that RFC 3986 §3.4 does not allow.
    InvalidQuery,
    /// The fragment holds a character or an escape that RFC 3986 §3.5 does not allow.
    InvalidFragment,
    /// Plain `http` to a host that is not loopback.
    InsecureTransport,
    /// An address on this computer or its local network, given by a host farther away (see
    /// [`HostScope`](crate::host::HostScope)).
    LocalFromRemote,
}

impl fmt::Display for AddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "no address was entered; type the JMAP server's address, for example example.com",
            Self::TooLong => "the address is longer than 8 KiB (8192 characters), more than any server address needs; check that nothing else was pasted with it",
            Self::ForbiddenCharacter => "the address contains a space, a control character, a non-ASCII character or one of \" < > ^ ` { | }; remove it or percent-encode it",
            Self::Backslash => "the address contains a backslash (\\); addresses use forward slashes (/)",
            Self::MissingScheme => "the address does not say how to connect; start it with https://",
            Self::UnsupportedScheme => "the address uses a scheme other than https; enter an https:// address (http is only for this computer)",
            Self::MissingAuthority => "the scheme is not followed by // and a server name; write it as https://server.example/path",
            Self::EmptyHost => "the address has no server name after https://; add the server's name",
            Self::UserInfo => "the address contains a user name followed by @ before the server name; remove it and enter the user name in the sign-in fields",
            Self::InvalidHost => "the server name contains characters a DNS name cannot hold, or an empty or overlong label; check its spelling",
            Self::InternationalHost => "the server name has non-ASCII letters; enter its ASCII form, which starts with xn--",
            Self::AmbiguousNumericHost => "the server name looks numeric but is not a plain IPv4 address; write four numbers such as 192.0.2.1, without leading zeros, or use the server's name",
            Self::InvalidIpv6 => "the IPv6 address in brackets cannot be read; zone suffixes such as %eth0 and IPvFuture forms are not supported",
            Self::InvalidPort => "the port after the server name is not a number from 1 to 65535",
            Self::InvalidPath => "the path contains a character or a % sign that addresses do not allow there; percent-encode it",
            Self::InvalidQuery => "the part after ? contains a character or a % sign that addresses do not allow there; percent-encode it",
            Self::InvalidFragment => "the part after # contains a character or a % sign that addresses do not allow there",
            Self::InsecureTransport => "this address would send the password unencrypted over plain http to another computer; use https:// (plain http is only accepted for localhost, 127.0.0.0/8 and [::1])",
            Self::LocalFromRemote => "the server tried to send this program to this computer or its local network, which a remote server may not do; its configuration may be wrong or tampered with",
        })
    }
}

impl std::error::Error for AddressError {}

/// An absolute `http` or `https` address in normal form.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Address {
    text: String,
    scheme: Scheme,
    host: Host,
    port: Option<u16>,
    path_start: usize,
    query_start: Option<usize>,
}

/// The origin of an address: scheme, host and port (the tuple origin of the WHATWG URL Standard §4.7 for
/// `http` and `https`). Its `Display` form is `scheme://host` with `:port` only when it is not the default.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Origin {
    scheme: Scheme,
    host: Host,
    port: Option<u16>,
}

impl Origin {
    /// The scheme.
    pub fn scheme(&self) -> Scheme {
        self.scheme
    }

    /// The host.
    pub fn host(&self) -> &Host {
        &self.host
    }

    /// The port, the scheme's default when none was given.
    pub fn port(&self) -> u16 {
        self.port.unwrap_or_else(|| self.scheme.default_port())
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://{}", self.scheme.as_str(), self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// The parts of a URI reference (RFC 3986 Appendix B), the fragment already set aside.
struct Parts<'a> {
    scheme: Option<&'a str>,
    authority: Option<&'a str>,
    path: &'a str,
    query: Option<&'a str>,
}

fn refuse_unusable_characters(input: &str) -> Result<(), AddressError> {
    if input.is_empty() {
        return Err(AddressError::Empty);
    }
    if input.len() > MAX_ADDRESS_LEN {
        return Err(AddressError::TooLong);
    }
    for b in input.bytes() {
        if b == b'\\' {
            return Err(AddressError::Backslash);
        }
        if !(0x21..=0x7E).contains(&b) || matches!(b, b'"' | b'<' | b'>' | b'^' | b'`' | b'{' | b'|' | b'}') {
            return Err(AddressError::ForbiddenCharacter);
        }
    }
    Ok(())
}

fn looks_like_scheme(text: &str) -> bool {
    let mut bytes = text.bytes();
    matches!(bytes.next(), Some(b) if b.is_ascii_alphabetic()) && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
}

/// Splits a reference into its components (RFC 3986 Appendix B); the fragment is dropped, as RFC 9110 §7.1
/// leaves it out of every target.
fn reference_parts(reference: &str) -> Result<Parts<'_>, AddressError> {
    let (without_fragment, fragment) = match reference.split_once('#') {
        Some((head, fragment)) => (head, Some(fragment)),
        None => (reference, None),
    };
    if let Some(fragment) = fragment
        && !chars_allowed(fragment, true)
    {
        return Err(AddressError::InvalidFragment);
    }
    let (scheme, rest) = match without_fragment.find([':', '/', '?']) {
        Some(index) if without_fragment.get(index..).is_some_and(|tail| tail.starts_with(':')) && index > 0 => {
            let (scheme, rest) = without_fragment.split_at_checked(index).ok_or(AddressError::MissingScheme)?;
            (Some(scheme), rest.get(1..).unwrap_or(""))
        }
        _ => (None, without_fragment),
    };
    let (authority, rest) = match rest.strip_prefix("//") {
        Some(after) => {
            let end = after.find(['/', '?']).unwrap_or(after.len());
            let (authority, rest) = after.split_at_checked(end).ok_or(AddressError::MissingAuthority)?;
            (Some(authority), rest)
        }
        None => (None, rest),
    };
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (rest, None),
    };
    Ok(Parts { scheme, authority, path, query })
}

/// Whether every character of a path (`allow_question` false) or a query/fragment (`allow_question` true) is
/// allowed by RFC 3986 §3.3–§3.5, escapes included.
fn chars_allowed(text: &str, allow_question: bool) -> bool {
    let mut bytes = text.bytes();
    while let Some(b) = bytes.next() {
        let ok = match b {
            b'%' => bytes.next().and_then(percent::hex_value).is_some() && bytes.next().and_then(percent::hex_value).is_some(),
            b':' | b'@' | b'/' => true,
            b'?' => allow_question,
            _ => is_unreserved(b) || is_sub_delim(b),
        };
        if !ok {
            return false;
        }
    }
    true
}

fn port_number(text: &str, scheme: Scheme) -> Result<Option<u16>, AddressError> {
    if text.is_empty() {
        return Ok(None);
    }
    let mut value: u32 = 0;
    for b in text.bytes() {
        let digit = match b {
            b'0'..=b'9' => u32::from(b.saturating_sub(b'0')),
            _ => return Err(AddressError::InvalidPort),
        };
        value = value.checked_mul(10).and_then(|v| v.checked_add(digit)).filter(|v| *v <= u32::from(u16::MAX)).ok_or(AddressError::InvalidPort)?;
    }
    let port = u16::try_from(value).map_err(|_| AddressError::InvalidPort)?;
    if port == 0 {
        return Err(AddressError::InvalidPort);
    }
    Ok(if port == scheme.default_port() { None } else { Some(port) })
}

/// Separates an authority's host text (brackets kept for IPv6) from its port text, empty when absent.
fn separate_port(authority: &str) -> Result<(&str, &str), AddressError> {
    if authority.starts_with('[') {
        let close = authority.find(']').ok_or(AddressError::InvalidIpv6)?;
        let end = close.checked_add(1).ok_or(AddressError::InvalidIpv6)?;
        let (host, rest) = authority.split_at_checked(end).ok_or(AddressError::InvalidIpv6)?;
        if rest.is_empty() {
            return Ok((host, ""));
        }
        let port = rest.strip_prefix(':').ok_or(AddressError::InvalidHost)?;
        return Ok((host, port));
    }
    if authority.contains(['[', ']']) {
        return Err(AddressError::InvalidHost);
    }
    Ok(authority.split_once(':').unwrap_or((authority, "")))
}

/// Removes the `.` and `..` segments of a path (RFC 3986 §5.2.4).
pub(crate) fn collapse_dot_segments(path: &str) -> String {
    let mut input = path;
    let mut output = String::with_capacity(path.len());
    while !input.is_empty() {
        if let Some(rest) = input.strip_prefix("../") {
            input = rest;
        } else if let Some(rest) = input.strip_prefix("./") {
            input = rest;
        } else if input.starts_with("/./") {
            input = input.get(2..).unwrap_or("");
        } else if input == "/." {
            input = "/";
        } else if input.starts_with("/../") {
            input = input.get(3..).unwrap_or("");
            drop_last_segment(&mut output);
        } else if input == "/.." {
            input = "/";
            drop_last_segment(&mut output);
        } else if input == "." || input == ".." {
            input = "";
        } else {
            let end = input.get(1..).and_then(|after_first| after_first.find('/')).and_then(|index| index.checked_add(1)).unwrap_or(input.len());
            let (segment, rest) = input.split_at_checked(end).unwrap_or((input, ""));
            output.push_str(segment);
            input = rest;
        }
    }
    output
}

fn drop_last_segment(output: &mut String) {
    match output.rfind('/') {
        Some(index) => output.truncate(index),
        None => output.clear(),
    }
}

/// Merges a relative path with the path of a base that has an authority (RFC 3986 §5.2.3).
fn base_directory_plus(base_path: &str, reference_path: &str) -> String {
    let directory = match base_path.rfind('/') {
        Some(index) => base_path.get(..=index).unwrap_or("/"),
        None => "/",
    };
    let mut merged = String::with_capacity(directory.len().saturating_add(reference_path.len()));
    merged.push_str(directory);
    merged.push_str(reference_path);
    merged
}

impl Address {
    /// Parses an absolute `http` or `https` address.
    ///
    /// This checks the syntax only; [`Address::parse_secure`] also applies the transport rule. A fragment is
    /// accepted and dropped (RFC 9110 §7.1).
    pub fn parse(input: &str) -> Result<Self, AddressError> {
        refuse_unusable_characters(input)?;
        let parts = reference_parts(input)?;
        let scheme_text = parts.scheme.ok_or(AddressError::MissingScheme)?;
        if !looks_like_scheme(scheme_text) {
            return Err(AddressError::MissingScheme);
        }
        let scheme = match scheme_text.to_ascii_lowercase().as_str() {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            _ => return Err(AddressError::UnsupportedScheme),
        };
        let authority = parts.authority.ok_or(AddressError::MissingAuthority)?;
        if authority.contains('@') {
            return Err(AddressError::UserInfo);
        }
        let (host_text, port_text) = separate_port(authority)?;
        let host = Host::parse(host_text)?;
        let port = port_number(port_text, scheme)?;
        if !chars_allowed(parts.path, false) {
            return Err(AddressError::InvalidPath);
        }
        let path = percent::normalise_escapes(parts.path).ok_or(AddressError::InvalidPath)?;
        let mut path = collapse_dot_segments(&path);
        if path.is_empty() {
            path.push('/');
        }
        let query = match parts.query {
            Some(query) => {
                if !chars_allowed(query, true) {
                    return Err(AddressError::InvalidQuery);
                }
                Some(percent::normalise_escapes(query).ok_or(AddressError::InvalidQuery)?)
            }
            None => None,
        };
        Self::assemble(scheme, host, port, &path, query.as_deref())
    }

    /// Parses an absolute address and applies the transport rule ([`Address::require_secure_transport`]).
    pub fn parse_secure(input: &str) -> Result<Self, AddressError> {
        let address = Self::parse(input)?;
        address.require_secure_transport()?;
        Ok(address)
    }

    fn assemble(scheme: Scheme, host: Host, port: Option<u16>, path: &str, query: Option<&str>) -> Result<Self, AddressError> {
        let origin = Origin { scheme, host, port };
        let mut text = origin.to_string();
        let path_start = text.len();
        text.push_str(path);
        let query_start = match query {
            Some(query) => {
                let start = text.len();
                text.push('?');
                text.push_str(query);
                Some(start)
            }
            None => None,
        };
        if text.len() > MAX_ADDRESS_LEN {
            return Err(AddressError::TooLong);
        }
        let Origin { scheme, host, port } = origin;
        Ok(Self { text, scheme, host, port, path_start, query_start })
    }

    /// Resolves a reference, relative or absolute, against this address (RFC 3986 §5.2.2, strict).
    ///
    /// The result is checked like [`Address::parse`]; the transport rule is not applied.
    pub fn resolve(&self, reference: &str) -> Result<Self, AddressError> {
        if reference.is_empty() {
            return Ok(self.clone());
        }
        refuse_unusable_characters(reference)?;
        let parts = reference_parts(reference)?;
        if parts.scheme.is_some() {
            return Self::parse(reference);
        }
        let without_fragment = reference.split_once('#').map_or(reference, |(head, _)| head);
        if parts.authority.is_some() {
            return Self::parse(&format!("{}:{}", self.scheme.as_str(), without_fragment));
        }
        let (path, query) = if parts.path.is_empty() {
            (self.path().to_owned(), parts.query.or(self.query()))
        } else if parts.path.starts_with('/') {
            (parts.path.to_owned(), parts.query)
        } else {
            (base_directory_plus(self.path(), parts.path), parts.query)
        };
        // Dot-segment removal (§5.2.2, §5.2.4) runs in `parse`, after escapes are normalised (§6.2.2.2), so that
        // `%2e` segments are treated the same way whether the reference was relative or absolute.
        let mut target = self.origin().to_string();
        target.push_str(&path);
        if let Some(query) = query {
            target.push('?');
            target.push_str(query);
        }
        Self::parse(&target)
    }

    /// The full address, for the request line. Do not log it: use [`Address::origin`] for diagnostics.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The scheme.
    pub fn scheme(&self) -> Scheme {
        self.scheme
    }

    /// The host.
    pub fn host(&self) -> &Host {
        &self.host
    }

    /// The port, the scheme's default when the address names none.
    pub fn port(&self) -> u16 {
        self.port.unwrap_or_else(|| self.scheme.default_port())
    }

    /// The path, starting with `/`.
    pub fn path(&self) -> &str {
        let end = self.query_start.unwrap_or(self.text.len());
        self.text.get(self.path_start..end).unwrap_or("/")
    }

    /// The query without its `?`, if there is one.
    pub fn query(&self) -> Option<&str> {
        let start = self.query_start?.checked_add(1)?;
        self.text.get(start..)
    }

    /// The origin: scheme, host and port.
    pub fn origin(&self) -> Origin {
        Origin { scheme: self.scheme, host: self.host.clone(), port: self.port }
    }

    /// Whether both addresses have the same origin.
    pub fn same_origin(&self, other: &Self) -> bool {
        self.scheme == other.scheme && self.host == other.host && self.port == other.port
    }

    /// Whether this address may carry credentials: `https`, or `http` to a loopback host
    /// (see [`Host::is_loopback`]).
    pub fn uses_secure_transport(&self) -> bool {
        self.scheme == Scheme::Https || self.host.is_loopback()
    }

    /// The transport rule as a `Result`.
    pub fn require_secure_transport(&self) -> Result<(), AddressError> {
        if self.uses_secure_transport() { Ok(()) } else { Err(AddressError::InsecureTransport) }
    }

    /// Checks an address that `self` handed out (a session's endpoint, a template's expansion, a redirect's
    /// target): it must satisfy the transport rule, and it may not be closer to this computer than `self`
    /// ([`Host::scope`]), so that a remote server cannot point the program, credentials and all, at a service
    /// on this computer or its local network. Only the address text is judged: a name that resolves to such an
    /// address can only be caught by the program performing the request.
    pub fn check_link(&self, target: &Self) -> Result<(), AddressError> {
        target.require_secure_transport()?;
        if target.host.scope() > self.host.scope() {
            return Err(AddressError::LocalFromRemote);
        }
        Ok(())
    }

    /// The same origin with another path (and no query).
    pub(crate) fn with_path(&self, path: &str) -> Result<Self, AddressError> {
        let mut text = self.origin().to_string();
        text.push_str(path);
        Self::parse(&text)
    }
}

impl fmt::Debug for Address {
    /// Shows the origin only (requirement 1.8): a path or query may hold something private.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Address({})", self.origin())
    }
}

#[cfg(test)]
mod tests;
