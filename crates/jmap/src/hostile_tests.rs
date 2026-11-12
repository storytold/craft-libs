//! Hostile input on every reader (requirement 4.2, in the absence of a fuzzer in CI): deterministic
//! pseudo-random bytes and mutations of valid samples must never panic, and what is accepted must keep the
//! crate's promises (normal form is stable, the transport rule holds, excerpts are clean and short).

use crate::address::Address;
use crate::discovery::session_address;
use crate::redact::{EXCERPT_MAX_CHARS, Redactor};
use crate::response::ApiResponse;
use crate::retry::{RetryAfter, UnixTime};
use crate::session::Session;
use crate::session::tests::{EXAMPLE, session_address as example_session_address};
use crate::template::AddressTemplate;

/// xorshift64*: a small deterministic generator, good enough to stir bytes.
struct Stir(u64);

impl Stir {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const TOKENS: &[&str] = &[
    "%",
    "%2e",
    "%2E%2e",
    "%00",
    "%ff",
    "..",
    "/./",
    "/../",
    "//",
    "\\",
    "@",
    ":",
    "::",
    "[",
    "]",
    "[::1]",
    "?",
    "#",
    "{",
    "}",
    "{a}",
    "\u{0}",
    "\n",
    " ",
    "é",
    "\u{202e}",
    "http://",
    "https://",
    "localhost",
    "127.0.0.1",
    "0x7f",
    "65536",
    "\"",
    "[]",
    "{}",
    "null",
    "1e999",
    "-1",
    "[[[[",
    "]]]]",
    ",",
    "\"error\"",
    "\"methodResponses\"",
    "\"sessionState\"",
    "\"apiUrl\"",
];

fn mutate(stir: &mut Stir, seed: &[u8]) -> Vec<u8> {
    let mut data = seed.to_vec();
    for _ in 0..=stir.below(4) {
        match stir.below(6) {
            0 if !data.is_empty() => {
                let at = stir.below(data.len());
                data[at] = (stir.next() & 0xFF) as u8;
            }
            1 => {
                let at = stir.below(data.len() + 1);
                let token = TOKENS[stir.below(TOKENS.len())].as_bytes();
                data.splice(at..at, token.iter().copied());
            }
            2 if !data.is_empty() => {
                let at = stir.below(data.len());
                data.truncate(at);
            }
            3 if !data.is_empty() => {
                let start = stir.below(data.len());
                let end = (start + stir.below(16)).min(data.len());
                data.drain(start..end);
            }
            4 if !data.is_empty() => {
                let start = stir.below(data.len());
                let end = (start + stir.below(32)).min(data.len());
                let copy: Vec<u8> = data[start..end].to_vec();
                data.splice(start..start, copy);
            }
            _ => {
                let len = stir.below(24);
                data.extend((0..len).map(|_| (stir.next() & 0xFF) as u8));
            }
        }
    }
    data
}

fn inputs(seeds: &[&[u8]], rounds: usize, salt: u64) -> Vec<Vec<u8>> {
    let mut stir = Stir(0x9E37_79B9_7F4A_7C15 ^ salt);
    (0..rounds)
        .map(|_| {
            let seed = seeds[stir.below(seeds.len())];
            mutate(&mut stir, seed)
        })
        .collect()
}

const ADDRESS_SEEDS: &[&[u8]] = &[
    b"https://jmap.example.com/.well-known/jmap",
    b"http://[::1]:8080/a/b?c=d#e",
    b"https://EXAMPLE.org:443/%7e/./../x?y=%2f",
    b"http://127.0.0.1/",
    b"example.net/jmap",
    b"https://ex%61mple.com:00080//a//b",
];

// RFC 3986 §6.2.2: whatever parses re-parses to itself; diagnostics show the origin only.
#[test]
fn address_parser_survives_hostile_input() {
    let base = Address::parse("https://jmap.example.com/a/b/c").unwrap();
    for data in inputs(ADDRESS_SEEDS, 20_000, 1) {
        let text = String::from_utf8_lossy(&data);
        if let Ok(address) = Address::parse(&text) {
            assert_eq!(Address::parse(address.as_str()).as_ref(), Ok(&address), "{text:?}");
            assert!(address.as_str().bytes().all(|b| (0x21..=0x7E).contains(&b)));
            assert_eq!(format!("{address:?}"), format!("Address({})", address.origin()));
        }
        if let Ok(resolved) = base.resolve(&text) {
            assert_eq!(Address::parse(resolved.as_str()).as_ref(), Ok(&resolved), "{text:?}");
        }
    }
}

// Requirement 1.6 and 1.2: whatever the loose form accepts satisfies the transport rule.
#[test]
fn loose_session_address_survives_hostile_input() {
    for data in inputs(ADDRESS_SEEDS, 20_000, 2) {
        let text = String::from_utf8_lossy(&data);
        if let Ok(address) = session_address(&text) {
            assert!(address.uses_secure_transport(), "{text:?}");
        }
    }
}

// RFC 8620 §2: the session reader never panics on mutated sessions.
#[test]
fn session_parser_survives_hostile_input() {
    let address = example_session_address();
    let seeds: &[&[u8]] = &[EXAMPLE.as_bytes(), b"{\"capabilities\":{}}", b"{}"];
    let mut accepted = 0usize;
    for data in inputs(seeds, 5_000, 3) {
        if let Ok(session) = Session::parse(&data, &address) {
            accepted += 1;
            assert!(session.api_address().uses_secure_transport());
        }
    }
    assert!(accepted > 0);
}

// RFC 8620 §3.4: the response reader never panics on mutated responses.
#[test]
fn api_response_parser_survives_hostile_input() {
    let seeds: &[&[u8]] = &[
        br#"{"methodResponses":[["Email/get",{"list":[]},"c0"],["error",{"type":"serverFail","description":"x"},"c1"]],"sessionState":"s","createdIds":{"k":"M1"}}"#,
        br#"{"methodResponses":[],"sessionState":""}"#,
    ];
    let mut redactor = Redactor::new();
    redactor.add("secret-value");
    for data in inputs(seeds, 20_000, 4) {
        let _ = ApiResponse::parse(&data, &redactor);
    }
}

// Requirement 1.7: template reading and expansion never panic, and expansions keep the transport rule.
#[test]
fn templates_survive_hostile_input() {
    let base = example_session_address();
    let seeds: &[&[u8]] = &[b"https://jmap.example.com/download/{accountId}/{blobId}/{name}?accept={type}", b"/upload/{accountId}/"];
    let values = [("accountId", "A1"), ("blobId", "B1"), ("name", "n"), ("type", "t/t"), ("a", ".")];
    for data in inputs(seeds, 20_000, 5) {
        let text = String::from_utf8_lossy(&data);
        if let Ok(template) = AddressTemplate::parse(&text, &base)
            && let Ok(address) = template.expand(&values)
        {
            assert!(address.uses_secure_transport());
        }
    }
}

// Requirement 3.5 and RFC 9110 §10.2.3: excerpts and Retry-After never panic and keep their bounds.
#[test]
fn text_readers_survive_hostile_input() {
    let mut redactor = Redactor::new();
    redactor.add("secret-value");
    let seeds: &[&[u8]] = &[b"Mon, 27 Jul 2009 12:28:53 GMT", b"Saturday, 29-Oct-94 19:43:31 GMT", b"120", b"secret-value in text"];
    for data in inputs(seeds, 20_000, 6) {
        let text = String::from_utf8_lossy(&data);
        let excerpt = redactor.excerpt(&text);
        assert!(excerpt.as_str().chars().count() <= EXCERPT_MAX_CHARS + 1);
        assert!(!excerpt.as_str().chars().any(char::is_control));
        assert!(!excerpt.as_str().contains("secret-value"));
        if let Some(retry) = RetryAfter::parse(&text) {
            let _ = retry.delay_from(UnixTime::from_secs(1_248_697_733));
        }
    }
}
