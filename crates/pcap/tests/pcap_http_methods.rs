use craft_anything_formats::pcap::decode;
use craft_anything_testkit::pcap::{ipv4_pkt, tcp};

/// WebDAV and extension methods retain the same HTTP evidence as core methods.
#[test]
fn extension_methods_are_http_requests() {
    for method in ["PROPFIND", "MKCOL", "LOCK", "REPORT", "M-SEARCH", "custom", "X!#$%&'*+-.^_`|~012"] {
        let request = format!("{method} /collection/ HTTP/1.1\r\nHost: dav.example\r\n\r\n");
        let decoded = decode(101, &ipv4_pkt(6, [1; 4], [2; 4], &tcp(100, 80, 0x18, request.as_bytes())));
        let http = decoded.http.unwrap_or_else(|| panic!("missing HTTP: {method}"));
        assert_eq!(http.method.as_deref(), Some(method));
        assert_eq!(http.target.as_deref(), Some("/collection/"));
        assert_eq!(http.host.as_deref(), Some("dav.example"));
    }
}

/// A method still needs a nonempty ASCII token and a complete valid start line.
#[test]
fn invalid_extension_method_lines_are_not_http() {
    for request in [
        " / HTTP/1.1\r\n",
        "BAD:METHOD / HTTP/1.1\r\n",
        "BAD/METHOD / HTTP/1.1\r\n",
        "MÉTHOD / HTTP/1.1\r\n",
        "BAD\tMETHOD / HTTP/1.1\r\n",
        "PROPFIND / HTTP/1.1",
        "PROPFIND / HTTP/no\r\n",
    ] {
        let decoded = decode(101, &ipv4_pkt(6, [1; 4], [2; 4], &tcp(100, 80, 0x18, request.as_bytes())));
        assert!(decoded.http.is_none(), "{request:?}");
    }
}
