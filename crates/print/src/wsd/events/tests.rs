//! Unit tests of the pieces; the listener and the subscription flows are tested end to end in
//! `tests/wsd_events.rs`.

use super::http::{Bucket, Request, ingest, token_of};
use super::*;
use crate::ManualClock;
use crate::wsd::adapter::default_transport;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

#[test]
fn the_comparison_looks_at_every_byte_and_the_length() {
    assert!(ct_eq("abc123", "abc123"));
    assert!(!ct_eq("abc123", "abc124"));
    assert!(!ct_eq("abc123", "abc12"));
    assert!(!ct_eq("", "a"));
    assert!(ct_eq("", ""));
}

#[test]
fn a_huge_lease_still_has_a_renewal_point() {
    use std::time::Duration;
    assert_eq!(renew_after(Duration::from_secs(600), 50), Duration::from_secs(300));
    assert_eq!(renew_after(Duration::from_secs(600), 500), Duration::from_secs(600), "more than all of it is all of it");
    // Duration::MAX times 50 does not fit; this must not panic and must stay below the lease.
    let r = renew_after(Duration::MAX, 50);
    assert!(r > Duration::from_secs(1 << 40) && r < Duration::MAX);
    assert_eq!(renew_after(Duration::MAX, 100), Duration::MAX / 100 * 100);
}

#[test]
fn tokens_are_long_hex_and_never_repeat() {
    let tokens: Vec<String> = (0..64).map(|_| new_token()).collect();
    for t in &tokens {
        assert_eq!(t.len(), 64);
        assert!(t.bytes().all(|b| b.is_ascii_hexdigit()));
    }
    let mut sorted = tokens.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), tokens.len());
}

#[test]
fn only_a_hex_token_under_n_or_e_is_a_path() {
    assert_eq!(token_of("/n/abcdef0123"), Some("abcdef0123"));
    assert_eq!(token_of("/e/ABCDEF"), Some("ABCDEF"));
    for bad in ["/", "/n/", "/x/abc", "/n/abc/", "/n/ab c", "/n/../e/ab", "/n/zz", "n/abc", "/n/abc?x=1", &format!("/n/{}", "a".repeat(129))] {
        assert_eq!(token_of(bad), None, "{bad}");
    }
}

#[test]
fn the_bucket_allows_a_burst_then_refills_with_time() {
    let mut bucket = Bucket::new(3, 2);
    let now = secs(10);
    assert!((0..3).all(|_| bucket.take(now)));
    assert!(!bucket.take(now));
    // Half a second is one token at two per second.
    assert!(bucket.take(now + Duration::from_millis(500)));
    assert!(!bucket.take(now + Duration::from_millis(500)));
    // A long pause refills to the burst and no further.
    let later = now + secs(1000);
    assert!((0..3).all(|_| bucket.take(later)));
    assert!(!bucket.take(later));
}

#[test]
fn the_resubscribe_backoff_doubles_to_its_cap() {
    let t = EventTuning::default();
    let waits: Vec<u64> = (1..=6).map(|n| backoff(n, t).as_secs()).collect();
    assert_eq!(waits, [60, 120, 240, 480, 600, 600]);
    assert_eq!(backoff(u32::MAX, t).as_secs(), 600);
}

#[test]
fn a_granted_lease_is_read_and_never_below_two_seconds() {
    let asked = secs(600);
    assert_eq!(granted_lease(None, asked), asked);
    assert_eq!(granted_lease(Some(Expiry::After(secs(90))), asked), secs(90));
    assert_eq!(granted_lease(Some(Expiry::After(Duration::ZERO)), asked), secs(2));
    // An absolute time in the past is no lease at all.
    assert_eq!(granted_lease(Some(Expiry::At(1)), asked), secs(2));
}

#[test]
fn the_advertised_address_follows_the_local_address_policy() {
    let bind = SocketAddr::from(([127, 0, 0, 1], 0));
    for no in ["http://8.8.8.8:80/", "http://example.com:80/", "https://127.0.0.1:80/", "ftp://127.0.0.1/"] {
        assert!(NotifyConfig::new(bind, no).is_err(), "{no}");
    }
    for ok in ["http://127.0.0.1:9000/", "http://10.0.0.5:9000", "http://192.168.1.2:80/x"] {
        assert!(NotifyConfig::new(bind, ok).is_ok(), "{ok}");
    }
}

#[test]
fn the_advertised_address_is_the_bind_address_or_a_private_one() {
    let lan = SocketAddr::from(([192, 168, 1, 5], 9000));
    // The same address, or a private (NAT) address the app names on purpose.
    for ok in ["http://192.168.1.5:9000/", "http://192.168.1.5:9100/", "http://10.1.2.3:9000/", "http://172.16.0.9:80/"] {
        assert!(NotifyConfig::new(lan, ok).is_ok(), "{ok}");
    }
    // A loopback or link-local address that is not the listener's own cannot be the listener.
    for no in ["http://127.0.0.1:9000/", "http://169.254.3.4:9000/", "http://[::1]:9000/", "http://[fe80::1]:9000/"] {
        let e = NotifyConfig::new(lan, no).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidRequest, "{no}");
        assert!(e.message().contains("neither the listener's address"), "{no}: {e}");
    }
    // Loopback and link-local binds advertised as themselves.
    assert!(NotifyConfig::new("127.0.0.1:0".parse().unwrap(), "http://127.0.0.1:1/").is_ok());
    assert!(NotifyConfig::new("[::1]:0".parse().unwrap(), "http://[::1]:1/").is_ok());
    assert!(NotifyConfig::new("[fe80::1]:0".parse().unwrap(), "http://[fe80::1]:1/").is_ok());
    assert!(NotifyConfig::new("169.254.3.4:0".parse().unwrap(), "http://169.254.3.4:1/").is_ok());
    assert!(NotifyConfig::new("[fd00::5]:0".parse().unwrap(), "http://[fd00::5]:1/").is_ok());
}

#[test]
fn a_wildcard_or_public_bind_is_refused_by_name() {
    for bind in ["0.0.0.0:0", "0.0.0.0:9000", "[::]:0", "[::]:9000", "[::ffff:0.0.0.0]:9000"] {
        let e = NotifyConfig::new(bind.parse().unwrap(), "http://127.0.0.1:1/").unwrap_err();
        assert_eq!((e.kind(), e.retry_safety()), (ErrorKind::InvalidRequest, crate::RetrySafety::Safe), "{bind}");
        assert!(e.message().contains("wildcard") && e.message().contains("local interface address"), "{bind}: {e}");
        // The text form goes through the same check.
        assert!(NotifyConfig::parse(bind, "http://127.0.0.1:1/").is_err(), "{bind}");
    }
    for bind in ["8.8.8.8:9000", "203.0.113.9:80", "[2001:db8::1]:9000", "224.0.0.251:5353", "255.255.255.255:9"] {
        let e = NotifyConfig::new(bind.parse().unwrap(), "http://127.0.0.1:1/").unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidRequest, "{bind}");
        assert!(e.message().contains("not a loopback, link-local or private address"), "{bind}: {e}");
    }
    // A concrete address is accepted, as is a v4-mapped one that names a local address.
    for bind in ["127.0.0.1:0", "10.0.0.5:9000", "[::ffff:192.168.1.5]:9000"] {
        let ip = bind.parse::<SocketAddr>().unwrap().ip().to_canonical();
        let advertise = format!("http://{ip}:1/");
        assert!(NotifyConfig::new(bind.parse().unwrap(), &advertise).is_ok(), "{bind}");
    }
}

#[test]
fn a_config_whose_public_fields_were_changed_is_checked_again_before_the_socket_opens() {
    let mut config = NotifyConfig::loopback().unwrap();
    config.bind = SocketAddr::from(([0, 0, 0, 0], 0));
    let clock: Arc<dyn Clock> = Arc::new(ManualClock::new());
    let e = Events::start(config, clock, default_transport(), NetworkLimits::default()).map(|_| ()).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidRequest);
    assert!(e.message().contains("wildcard"), "{e}");
}

fn events() -> Events {
    let clock: Arc<dyn Clock> = Arc::new(ManualClock::new());
    let config = NotifyConfig::loopback().unwrap();
    Events::start(config, clock, default_transport(), NetworkLimits::default()).unwrap()
}

#[test]
fn nothing_listens_until_started_and_the_port_follows_the_system_choice() {
    let events = events();
    assert!(events.local_addr().ip().is_loopback());
    assert_ne!(events.local_addr().port(), 0);
    assert_eq!(events.advertise_url(), format!("http://127.0.0.1:{}", events.local_addr().port()));
    events.shutdown();
    events.shutdown();
    assert!(std::net::TcpStream::connect(events.local_addr()).is_err());
}

#[test]
fn a_request_from_another_address_than_the_tokens_printer_is_forged() {
    let events = events();
    let printer = PrinterUrl::parse("http://127.0.0.1:5357/print").unwrap();
    let _rx = events.register(&printer, 1).unwrap();
    let token = events.token_for(&printer).unwrap();
    let request =
        Request { method: "POST".into(), path: format!("/n/{token}"), content_type: Some("application/soap+xml".into()), body: b"<x/>".to_vec() };
    // From the printer's address the body is merely malformed ...
    assert_eq!(ingest(&events.shared, "127.0.0.1".parse().unwrap(), &request), 400);
    // ... from any other address the token does not work at all.
    assert_eq!(ingest(&events.shared, "127.0.0.2".parse().unwrap(), &request), 404);
    assert_eq!(events.stats().forged, 1);
    assert_eq!(events.stats().malformed, 1);
}

#[test]
fn the_subscription_table_is_bounded() {
    let events = events();
    let tuning = events.shared.tuning;
    let mut kept = Vec::new();
    for i in 0..tuning.max_subscriptions {
        let printer = PrinterUrl::parse(&format!("http://127.0.0.1:{}/print", 6000 + i)).unwrap();
        kept.push(events.register(&printer, 1).unwrap());
    }
    let extra = PrinterUrl::parse("http://127.0.0.1:7999/print").unwrap();
    assert!(events.register(&extra, 1).is_none());
}
