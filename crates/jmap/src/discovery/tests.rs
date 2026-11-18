use super::*;

fn fetched(input: &str) -> String {
    session_address(input).unwrap().as_str().to_owned()
}

// RFC 8620 §2.2: a bare host means https and the well-known path.
#[test]
fn bare_host_gets_well_known_path() {
    assert_eq!(fetched("example.com"), "https://example.com/.well-known/jmap");
    assert_eq!(fetched("  Example.COM \n"), "https://example.com/.well-known/jmap");
    assert_eq!(fetched("example.com/"), "https://example.com/.well-known/jmap");
    assert_eq!(fetched("example.com:8443"), "https://example.com:8443/.well-known/jmap");
    assert_eq!(fetched("https://example.com"), "https://example.com/.well-known/jmap");
    assert_eq!(fetched("[::1]:8080"), "https://[::1]:8080/.well-known/jmap");
}

// Requirement 1.6: a host with a path keeps the path; https is the default scheme.
#[test]
fn host_with_path_keeps_it() {
    assert_eq!(fetched("example.com/jmap/session"), "https://example.com/jmap/session");
    assert_eq!(fetched("jmap.example.org:8443/api/session?x=1"), "https://jmap.example.org:8443/api/session?x=1");
    assert_eq!(fetched("HTTPS://example.net/jmap/"), "https://example.net/jmap/");
    assert_eq!(fetched("example.com/?a=b"), "https://example.com/?a=b");
}

// Requirement 1.2 and 1.6: never a downgrade; http only as typed, and only to loopback.
#[test]
fn never_downgrades() {
    assert_eq!(fetched("localhost:8080"), "https://localhost:8080/.well-known/jmap");
    assert_eq!(fetched("http://localhost:8080"), "http://localhost:8080/.well-known/jmap");
    assert_eq!(fetched("http://127.0.0.1:8080/jmap"), "http://127.0.0.1:8080/jmap");
    assert_eq!(session_address("http://example.com"), Err(AddressError::InsecureTransport));
    assert_eq!(session_address("http://sub.localhost/"), Err(AddressError::InsecureTransport));
}

// Requirement 1.1 applied to typed input: what is not an address is refused.
#[test]
fn refuses_junk() {
    assert_eq!(session_address(""), Err(AddressError::Empty));
    assert_eq!(session_address("   "), Err(AddressError::Empty));
    assert_eq!(session_address("https:example.com"), Err(AddressError::MissingAuthority));
    assert_eq!(session_address("http:localhost"), Err(AddressError::MissingAuthority));
    assert_eq!(session_address("ftp://example.com"), Err(AddressError::UnsupportedScheme));
    assert_eq!(session_address("user@example.com"), Err(AddressError::UserInfo));
    assert_eq!(session_address("exa mple.com"), Err(AddressError::ForbiddenCharacter));
    assert_eq!(session_address("example.com\\jmap"), Err(AddressError::Backslash));
    assert_eq!(session_address("bücher.example.com"), Err(AddressError::ForbiddenCharacter));
    assert_eq!(
        session_address("example.com/path?u=http://example.org"),
        Ok(Address::parse("https://example.com/path?u=http://example.org").unwrap())
    );
}
