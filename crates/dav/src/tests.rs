use proptest::prelude::*;

use super::*;

#[test]
fn namespaces_entities_cdata_and_failed_properties_stay_distinct() {
    let root = parse_multistatus(br#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:e="evil"><d:response><d:href>/a&amp;b&#x2f;c</d:href><d:propstat><d:prop><e:getetag>wrong</e:getetag></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat><d:propstat><d:prop><d:getetag>&quot;v1&quot;</d:getetag><c:calendar-data><![CDATA[BEGIN:VCALENDAR]]></c:calendar-data><c:comp name="VEVENT"/></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#).unwrap();
    let row = responses(&root).next().unwrap();
    assert_eq!(href(row).as_deref(), Some("/a&b/c"));
    let props: Vec<_> = properties(row).collect();
    assert_eq!(props.len(), 3);
    assert_eq!(props[0].name, "d:getetag");
    assert_eq!(props[0].text(), "\"v1\"");
    assert_eq!((props[1].name.as_str(), props[1].text().as_ref()), ("c:calendar-data", "BEGIN:VCALENDAR"));
    assert_eq!(props[2].attr("name"), Some("VEVENT"));
}

#[test]
fn any_prefix_and_the_default_namespace_read_the_same() {
    let a = parse_multistatus(b"<multistatus xmlns='DAV:' xmlns:C='urn:ietf:params:xml:ns:carddav'><response><href>/x</href><propstat><prop><C:address-data>v</C:address-data></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>").unwrap();
    let b = parse_multistatus(b"<D:multistatus xmlns:D='DAV:'><D:response><D:href>/x</D:href><D:propstat><D:prop><address-data xmlns='urn:ietf:params:xml:ns:carddav'>v</address-data></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>").unwrap();
    assert_eq!(a, b);
    assert_eq!(properties(responses(&a).next().unwrap()).next().map(|p| p.name.as_str()), Some("card:address-data"));
    // Other namespaces never pass for these.
    let other = parse_multistatus(b"<multistatus xmlns='DAV:'><response><x:href xmlns:x='urn:other'>/x</x:href></response></multistatus>").unwrap();
    assert_eq!(href(responses(&other).next().unwrap()), None);
}

#[test]
fn statuses_are_read_strictly() {
    assert!(is_ok_status("HTTP/1.1 200 OK") && is_ok_status(" HTTP/2 200"));
    for bad in ["", "200", "HTTP/1.1 2000", "HTTP/1.1 404 Not Found", "OK 200", "http/1.1 200"] {
        assert!(!is_ok_status(bad), "{bad}");
    }
}

#[test]
fn dtd_unknown_entities_namespaces_and_incomplete_trees_fail_closed() {
    for text in [
        "<!DOCTYPE x [<!ENTITY a SYSTEM 'file:///not-read'>]><multistatus xmlns='DAV:'/>",
        "<!DOCTYPE multistatus><multistatus xmlns='DAV:'/>",
        "<multistatus xmlns='DAV:'>&unknown;</multistatus>",
        "<d:multistatus/>",
        "<multistatus xmlns='DAV:'><response>",
        "<multistatus xmlns='DAV:'><response></multistatus>",
        "<multistatus xmlns='DAV:'/><multistatus xmlns='DAV:'/>",
        "not XML<multistatus xmlns='DAV:'/>",
        "\u{a0}<multistatus xmlns='DAV:'/>",
        "<multistatus xmlns='wrong'/>",
        "<multistatus/>",
        "<multistatus xmlns='DAV:'>\0</multistatus>",
        "<multistatus xmlns='DAV:'>&#0;</multistatus>",
        "<multistatus xmlns='DAV:' name='&#0;'/>",
        "",
    ] {
        assert!(parse_multistatus(text.as_bytes()).is_err(), "{text}");
    }
    assert_eq!(parse_multistatus(&[0xff]), Err(Error::Characters));
    assert_eq!(parse_multistatus(b"<propfind xmlns='DAV:'/>"), Err(Error::NotMultistatus));
    assert!(parse_multistatus(&vec![b' '; MAX_BYTES + 1]).is_err());
}

#[test]
fn depth_node_and_attribute_counts_are_bounded() {
    let deep = |levels: usize| format!("<multistatus xmlns='DAV:'>{}text{}</multistatus>", "<x>".repeat(levels), "</x>".repeat(levels));
    assert!(parse_multistatus(deep(MAX_DEPTH - 1).as_bytes()).is_ok());
    assert!(parse_multistatus(deep(MAX_DEPTH).as_bytes()).is_err());
    let many = format!("<multistatus xmlns='DAV:'>{}</multistatus>", "<response/>".repeat(MAX_NODES));
    assert!(parse_multistatus(many.as_bytes()).is_err());
    let attrs: String = (0..=MAX_ATTRS).map(|i| format!(" a{i}='x'")).collect();
    assert!(parse_multistatus(format!("<multistatus xmlns='DAV:'><response{attrs}/></multistatus>").as_bytes()).is_err());
}

proptest! {
    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = parse_multistatus(&bytes);
    }

    #[test]
    fn mutated_documents_never_panic(cut in 0usize..400, at in 0usize..400, byte in any::<u8>()) {
        let mut doc = br#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/a</d:href><d:propstat><d:prop><d:getetag>"1"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#.to_vec();
        if let Some(b) = doc.get_mut(at) {
            *b = byte;
        }
        doc.truncate(cut.max(1));
        if let Ok(root) = parse_multistatus(&doc) {
            for r in responses(&root) {
                let _ = (href(r), properties(r).count());
            }
        }
    }
}
