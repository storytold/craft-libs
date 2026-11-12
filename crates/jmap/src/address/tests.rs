use super::*;

fn parsed(input: &str) -> String {
    Address::parse(input).unwrap().as_str().to_owned()
}

// RFC 3986 §3 and RFC 9110 §4.2.1–§4.2.2: the components of http and https addresses.
#[test]
fn parses_components() {
    let address = Address::parse("https://jmap.example.com:8443/api/v1?x=1&y=2").unwrap();
    assert_eq!(address.scheme(), Scheme::Https);
    assert_eq!(address.host(), &Host::Name("jmap.example.com".into()));
    assert_eq!(address.port(), 8443);
    assert_eq!(address.path(), "/api/v1");
    assert_eq!(address.query(), Some("x=1&y=2"));
    assert_eq!(address.as_str(), "https://jmap.example.com:8443/api/v1?x=1&y=2");
}

// RFC 3986 §6.2.2 and §6.2.3, RFC 9110 §4.2.3: the three equivalent forms normalise to one.
#[test]
fn normal_form() {
    assert_eq!(parsed("http://example.com:80/~smith/home.html"), "http://example.com/~smith/home.html");
    assert_eq!(parsed("http://EXAMPLE.com/%7Esmith/home.html"), "http://example.com/~smith/home.html");
    assert_eq!(parsed("http://EXAMPLE.com:/%7esmith/home.html"), "http://example.com/~smith/home.html");
    assert_eq!(parsed("HTTPS://Example.ORG"), "https://example.org/");
    assert_eq!(parsed("https://example.org:443/a/./b/../c"), "https://example.org/a/c");
    assert_eq!(parsed("https://example.org/a%2fb?q=%2f%41"), "https://example.org/a%2Fb?q=%2FA");
    assert_eq!(parsed("https://example.org/a/%2e%2E/b"), "https://example.org/b");
}

// RFC 9110 §7.1: the fragment is not part of the target.
#[test]
fn drops_fragment() {
    assert_eq!(parsed("https://example.com/a?b#c"), "https://example.com/a?b");
    assert_eq!(Address::parse("https://example.com/#a b"), Err(AddressError::ForbiddenCharacter));
    assert_eq!(Address::parse("https://example.com/#a#b"), Err(AddressError::InvalidFragment));
}

// Requirement 1.1: what is not a usable absolute http/https address is refused.
#[test]
fn refuses_unusable_addresses() {
    let cases: &[(&str, AddressError)] = &[
        ("", AddressError::Empty),
        ("https://exa mple.com/", AddressError::ForbiddenCharacter),
        ("https://example.com/\u{7f}", AddressError::ForbiddenCharacter),
        ("https://example.com/\n", AddressError::ForbiddenCharacter),
        ("https://example.com/é", AddressError::ForbiddenCharacter),
        ("https://example.com/{x}", AddressError::ForbiddenCharacter),
        ("https://example.com\\@example.org/", AddressError::Backslash),
        ("https:\\\\example.com/", AddressError::Backslash),
        ("//example.com/", AddressError::MissingScheme),
        ("example.com/path", AddressError::MissingScheme),
        ("ftp://example.com/", AddressError::UnsupportedScheme),
        ("javascript:alert(1)", AddressError::UnsupportedScheme),
        ("https:example.com", AddressError::MissingAuthority),
        ("https:///path", AddressError::EmptyHost),
        ("https://:443/", AddressError::EmptyHost),
        ("https://user@example.com/", AddressError::UserInfo),
        ("https://user:secret@example.com/", AddressError::UserInfo),
        ("https://[::1/", AddressError::InvalidIpv6),
        ("https://[::g]/", AddressError::InvalidIpv6),
        ("https://[::1]x/", AddressError::InvalidHost),
        ("https://example.com:65536/", AddressError::InvalidPort),
        ("https://example.com:0/", AddressError::InvalidPort),
        ("https://example.com:-1/", AddressError::InvalidPort),
        ("https://example.com:12a/", AddressError::InvalidPort),
        ("https://example.com:1:2/", AddressError::InvalidPort),
        ("https://example.com/%zz", AddressError::InvalidPath),
        ("https://example.com/[x]", AddressError::InvalidPath),
        ("https://example.com/?%", AddressError::InvalidQuery),
        ("https://127.1/", AddressError::AmbiguousNumericHost),
    ];
    for (input, expected) in cases {
        assert_eq!(Address::parse(input).as_ref().map(Address::as_str), Err(expected), "{input:?}");
    }
    let long = format!("https://example.com/{}", "a".repeat(MAX_ADDRESS_LEN));
    assert_eq!(Address::parse(&long), Err(AddressError::TooLong));
}

// RFC 3986 §3.2.3: an empty port means the default one; leading zeros are still the same number.
#[test]
fn ports() {
    assert_eq!(parsed("https://example.com:/"), "https://example.com/");
    assert_eq!(parsed("https://example.com:00443/"), "https://example.com/");
    assert_eq!(parsed("http://localhost:8080"), "http://localhost:8080/");
    assert_eq!(Address::parse("https://example.com:65535/").unwrap().port(), 65535);
    assert_eq!(parsed("https://[::1]:8443/x"), "https://[::1]:8443/x");
}

// Requirement 1.2: https always; http only to localhost, 127.0.0.0/8 or [::1].
#[test]
fn transport_rule() {
    for ok in ["https://example.com/", "http://localhost/", "http://LocalHost:8080/", "http://127.0.0.1/", "http://127.9.8.7:1/", "http://[::1]/"] {
        assert!(Address::parse_secure(ok).is_ok(), "{ok}");
    }
    for refused in [
        "http://example.com/",
        "http://sub.localhost/",
        "http://localhost./",
        "http://192.0.2.1/",
        "http://[2001:db8::2]/",
        "http://[::ffff:127.0.0.1]/",
    ] {
        assert_eq!(Address::parse_secure(refused), Err(AddressError::InsecureTransport), "{refused}");
    }
}

// Requirement 1.2 hardened, RFC 9110 §4.2.1 and RFC 8620 §8.3: a remote host cannot hand out a loopback address.
#[test]
fn links_from_remote_to_loopback_are_refused() {
    let remote = Address::parse("https://jmap.example.com/").unwrap();
    let local = Address::parse("http://127.0.0.1:8080/").unwrap();
    let other = Address::parse("https://api.example.net/").unwrap();
    let plain = Address::parse("http://example.org/").unwrap();
    assert_eq!(remote.check_link(&local), Err(AddressError::LocalFromRemote));
    for sneaky in [
        "https://localhost./",
        "https://x.localhost/",
        "https://0.0.0.0/",
        "https://[::]/",
        "https://[::ffff:7f00:1]/",
        "https://192.168.1.10/",
        "https://[fe80::1]/",
    ] {
        let target = Address::parse(sneaky).unwrap();
        assert_eq!(remote.check_link(&target), Err(AddressError::LocalFromRemote), "{sneaky}");
    }
    let lan = Address::parse("https://192.168.1.10/jmap").unwrap();
    assert_eq!(lan.check_link(&Address::parse("https://192.168.1.11/api").unwrap()), Ok(()));
    assert_eq!(lan.check_link(&Address::parse("https://localhost/api").unwrap()), Err(AddressError::LocalFromRemote));
    assert_eq!(remote.check_link(&plain), Err(AddressError::InsecureTransport));
    assert_eq!(remote.check_link(&other), Ok(()));
    assert_eq!(local.check_link(&local), Ok(()));
    assert_eq!(local.check_link(&remote), Ok(()));
}

// RFC 3986 §5.4.1: the normal examples, hosts moved under example domains (the non-http "g:h" is
// refused, as this crate only knows http and https).
#[test]
fn resolves_normal_examples() {
    let base = Address::parse("http://a.example.com/b/c/d;p?q").unwrap();
    let cases = [
        ("g", "http://a.example.com/b/c/g"),
        ("./g", "http://a.example.com/b/c/g"),
        ("g/", "http://a.example.com/b/c/g/"),
        ("/g", "http://a.example.com/g"),
        ("//g.example.org", "http://g.example.org/"),
        ("?y", "http://a.example.com/b/c/d;p?y"),
        ("g?y", "http://a.example.com/b/c/g?y"),
        ("#s", "http://a.example.com/b/c/d;p?q"),
        ("g#s", "http://a.example.com/b/c/g"),
        ("g?y#s", "http://a.example.com/b/c/g?y"),
        (";x", "http://a.example.com/b/c/;x"),
        ("g;x", "http://a.example.com/b/c/g;x"),
        ("g;x?y#s", "http://a.example.com/b/c/g;x?y"),
        ("", "http://a.example.com/b/c/d;p?q"),
        (".", "http://a.example.com/b/c/"),
        ("./", "http://a.example.com/b/c/"),
        ("..", "http://a.example.com/b/"),
        ("../", "http://a.example.com/b/"),
        ("../g", "http://a.example.com/b/g"),
        ("../..", "http://a.example.com/"),
        ("../../", "http://a.example.com/"),
        ("../../g", "http://a.example.com/g"),
    ];
    for (reference, expected) in cases {
        assert_eq!(base.resolve(reference).unwrap().as_str(), expected, "{reference:?}");
    }
    assert_eq!(base.resolve("g:h"), Err(AddressError::UnsupportedScheme));
}

// RFC 3986 §5.4.2: the abnormal examples (hosts under example domains), strict for "http:g".
#[test]
fn resolves_abnormal_examples() {
    let base = Address::parse("http://a.example.com/b/c/d;p?q").unwrap();
    let cases = [
        ("../../../g", "http://a.example.com/g"),
        ("../../../../g", "http://a.example.com/g"),
        ("/./g", "http://a.example.com/g"),
        ("/../g", "http://a.example.com/g"),
        ("g.", "http://a.example.com/b/c/g."),
        (".g", "http://a.example.com/b/c/.g"),
        ("g..", "http://a.example.com/b/c/g.."),
        ("..g", "http://a.example.com/b/c/..g"),
        ("./../g", "http://a.example.com/b/g"),
        ("./g/.", "http://a.example.com/b/c/g/"),
        ("g/./h", "http://a.example.com/b/c/g/h"),
        ("g/../h", "http://a.example.com/b/c/h"),
        ("g;x=1/./y", "http://a.example.com/b/c/g;x=1/y"),
        ("g;x=1/../y", "http://a.example.com/b/c/y"),
        ("g?y/./x", "http://a.example.com/b/c/g?y/./x"),
        ("g?y/../x", "http://a.example.com/b/c/g?y/../x"),
        ("g#s/./x", "http://a.example.com/b/c/g"),
        ("g#s/../x", "http://a.example.com/b/c/g"),
    ];
    for (reference, expected) in cases {
        assert_eq!(base.resolve(reference).unwrap().as_str(), expected, "{reference:?}");
    }
    assert_eq!(base.resolve("http:g"), Err(AddressError::MissingAuthority));
    // RFC 3986 §6.2.2.2: an escaped dot is a dot, whether the reference is relative or absolute.
    assert_eq!(base.resolve("/a/%2e%2e/../b"), Address::parse("http://a.example.com/a/%2e%2e/../b"));
    assert_eq!(base.resolve("/a/%2e%2e/../b").unwrap().path(), "/b");
}

// RFC 3986 §5.2.4: the two worked examples of dot-segment removal.
#[test]
fn removes_dot_segments() {
    assert_eq!(collapse_dot_segments("/a/b/c/./../../g"), "/a/g");
    assert_eq!(collapse_dot_segments("mid/content=5/../6"), "mid/6");
    assert_eq!(collapse_dot_segments("/.."), "/");
    assert_eq!(collapse_dot_segments("//x/../y"), "//y");
}

// Requirement 1.4: the origin elides the default port; the full form keeps everything else.
#[test]
fn origin_and_full_form() {
    let address = Address::parse("https://example.com:443/session?token=abc").unwrap();
    assert_eq!(address.origin().to_string(), "https://example.com");
    assert_eq!(address.origin().port(), 443);
    let address = Address::parse("http://[::1]:8080/x").unwrap();
    assert_eq!(address.origin().to_string(), "http://[::1]:8080");
    assert!(address.same_origin(&Address::parse("http://[0::1]:8080/y").unwrap()));
    assert!(!address.same_origin(&Address::parse("http://[::1]:8081/x").unwrap()));
}

// Requirement 1.8: diagnostics never show more than the origin.
#[test]
fn debug_shows_origin_only() {
    let address = Address::parse("https://example.com/private/path?token=s3cr3t").unwrap();
    let shown = format!("{address:?}");
    assert_eq!(shown, "Address(https://example.com)");
    assert!(!shown.contains("s3cr3t") && !shown.contains("private"));
}

// RFC 3986 §6.2.2: normalisation is idempotent, so parsing the full form gives the same address.
#[test]
fn reparse_is_stable() {
    for input in ["https://EXAMPLE.com:443/a/../b/%7e?Q=%2f", "http://[0:0::1]:80//x/./y", "https://example.org"] {
        let first = Address::parse(input).unwrap();
        let second = Address::parse(first.as_str()).unwrap();
        assert_eq!(first, second);
    }
}
