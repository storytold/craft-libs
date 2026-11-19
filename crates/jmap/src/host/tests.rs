use super::*;

fn shown(input: &str) -> String {
    Host::parse(input).unwrap().to_string()
}

// RFC 3986 §3.2.2 and §6.2.2.1: registered names are case-insensitive and normalised to lower case.
#[test]
fn names_are_lower_cased() {
    assert_eq!(shown("Example.COM"), "example.com");
    assert_eq!(shown("mail.example.org."), "mail.example.org.");
    assert_eq!(shown("under_score.example.net"), "under_score.example.net");
}

// RFC 3986 §3.2.2 (pct-encoded in reg-name) and WHATWG URL §3.5: the host is percent-decoded first.
#[test]
fn names_are_percent_decoded() {
    assert_eq!(shown("ex%61mple.com"), "example.com");
    assert_eq!(Host::parse("ex%20ample.com"), Err(AddressError::InvalidHost));
    assert_eq!(Host::parse("ex%2"), Err(AddressError::InvalidHost));
}

// WHATWG URL §3.2 (forbidden domain code points) and §3.3: what DNS cannot carry is refused, and a name
// outside ASCII needs IDNA processing, which this crate does not do.
#[test]
fn refuses_bad_names() {
    for bad in ["exa mple.com", "exa*mple.com", "example..com", ".example.com", "a%2fb"] {
        assert!(Host::parse(bad).is_err(), "{bad}");
    }
    assert_eq!(Host::parse("bücher.example.com"), Err(AddressError::InternationalHost));
    assert_eq!(Host::parse("b%C3%BCcher.example.com"), Err(AddressError::InternationalHost));
    let long_label = format!("{}.example.com", "a".repeat(64));
    assert_eq!(Host::parse(&long_label), Err(AddressError::InvalidHost));
}

// RFC 3986 §3.2.2 IPv4address, and WHATWG URL §3.5 "ends in a number": only the plain dotted form passes.
#[test]
fn ipv4_only_in_plain_form() {
    assert_eq!(Host::parse("127.0.0.1").unwrap(), Host::Ipv4(Ipv4Addr::LOCALHOST));
    assert_eq!(shown("192.0.2.255"), "192.0.2.255");
    for ambiguous in ["127.1", "0x7f.0.0.1", "2130706433", "127.0.0.01", "127.0.0.1.", "256.0.0.1", "1.2.3.4.5"] {
        assert_eq!(Host::parse(ambiguous), Err(AddressError::AmbiguousNumericHost), "{ambiguous}");
    }
    assert!(Host::parse("example.123").is_err());
    assert_eq!(shown("v4.example.com"), "v4.example.com");
}

// RFC 3986 §3.2.2 IP-literal read by std::net, WHATWG URL §3.5 (no zones) and §3.6 (hex output, RFC 5952 §4).
#[test]
fn ipv6_forms() {
    assert_eq!(shown("[::1]"), "[::1]");
    assert_eq!(shown("[0:0:0:0:0:0:0:1]"), "[::1]");
    assert_eq!(shown("[2001:DB8::0:1]"), "[2001:db8::1]");
    assert_eq!(shown("[2001:db8:0:0:1:0:0:1]"), "[2001:db8::1:0:0:1]");
    assert_eq!(shown("[::ffff:192.0.2.1]"), "[::ffff:c000:201]");
    assert_eq!(shown("[::]"), "[::]");
    assert_eq!(shown("[2001:db8:3:4:5:6:7:8]"), "[2001:db8:3:4:5:6:7:8]");
    assert_eq!(shown("[1:2:3:4:5:6:192.0.2.1]"), "[1:2:3:4:5:6:c000:201]");
    assert_eq!(shown("[2001:db8:0:1:1:1:1:1]"), "[2001:db8:0:1:1:1:1:1]");
    assert_eq!(shown("[1:0:0:2:0:0:0:3]"), "[1:0:0:2::3]");
    for bad in [
        "[]",
        "[:::1]",
        "[1::2::3]",
        "[12345::]",
        "[1:2:3:4:5:6:7]",
        "[1:2:3:4:5:6:7:8:9]",
        "[::1%25eth0]",
        "[v1.abc]",
        "[::1",
        "[1:2:3:4:5:6:7::8:9]",
        "[::1.2.3]",
        "[1.2.3.4::]",
        "[::01.2.3.4]",
        "[::1.2.3.4:5]",
        "[1.2.3.4]",
        "[fe80::1%eth0]",
        "[V1.abc]",
        "[::ffff:1.2.3.4.]",
    ] {
        assert_eq!(Host::parse(bad), Err(AddressError::InvalidIpv6), "{bad}");
    }
}

// RFC 6890 (special-purpose registries), RFC 1918, RFC 6598, RFC 4193, RFC 4291 §2.5.5–§2.5.6, RFC 6052
// §2.1, RFC 6761 §6.3 (localhost names): how close to this computer a host is.
#[test]
fn scopes() {
    let scope = |text: &str| Host::parse(text).unwrap().scope();
    for host in [
        "localhost",
        "localhost.",
        "x.localhost",
        "x.localhost.",
        "127.0.0.1",
        "0.0.0.0",
        "[::1]",
        "[::]",
        "[::ffff:127.0.0.1]",
        "[::ffff:7f00:1]",
        "[::127.0.0.1]",
        "[64:ff9b::127.0.0.1]",
        "[::ffff:0.0.0.0]",
    ] {
        assert_eq!(scope(host), HostScope::ThisComputer, "{host}");
    }
    for host in [
        "10.1.2.3",
        "172.16.0.1",
        "172.31.255.255",
        "192.168.1.1",
        "169.254.0.1",
        "100.64.0.1",
        "[fe80::1]",
        "[fd00::1]",
        "[fc00::1]",
        "[::ffff:192.168.1.1]",
    ] {
        assert_eq!(scope(host), HostScope::PrivateNetwork, "{host}");
    }
    for host in ["example.com", "192.0.2.1", "172.32.0.1", "100.128.0.1", "[2001:db8::1]", "localhost.example.com"] {
        assert_eq!(scope(host), HostScope::Public, "{host}");
    }
}

// Requirement 1.2: the loopback hosts are `localhost` exactly, 127.0.0.0/8 and ::1.
#[test]
fn loopback_hosts() {
    for host in ["localhost", "LOCALHOST", "127.0.0.1", "127.255.0.9", "[::1]", "[0::1]"] {
        assert!(Host::parse(host).unwrap().is_loopback(), "{host}");
    }
    for host in ["sub.localhost", "localhost.", "localhost.example.com", "192.0.2.1", "[::ffff:127.0.0.1]", "[2001:db8::2]"] {
        assert!(!Host::parse(host).unwrap().is_loopback(), "{host}");
    }
}
