//! Hostile input: every limit, entity expansion, truncation, wrong encodings, wrong nesting.
//! Nothing here may panic, and nothing may grow beyond a small multiple of its input.

use craft_xml::{Canonical, Doctype, El, Error, Limits, Options, UnknownEntity, parse, to_utf8};

const CANON: Canonical<'static> = Canonical::new(&[("w", "urn:w"), ("r", "urn:r")]);

/// Every way of reading, lenient and strict.
fn all_options() -> Vec<Options<'static>> {
    let full =
        Options { cdata: true, pis: true, comments: true, drop_blank_text: true, unknown_entity: UnknownEntity::Literal, ..Options::default() };
    let base = [Options::default(), Options::local(), Options::canonical(&CANON), full, Options { doctype: Doctype::Reject, ..Options::default() }];
    base.iter().flat_map(|o| [*o, o.strict()]).collect()
}

fn text_len(e: &El) -> usize {
    let own: usize = e.attrs.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>() + e.name.len() + e.text().len();
    own + e.descendants().map(|d| d.name.len() + d.text().len() + d.attrs.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>()).sum::<usize>()
}

#[test]
fn the_defaults_are_the_documented_ones() {
    assert_eq!(
        Limits::default(),
        Limits { max_input: 256 * 1024 * 1024, max_depth: 256, max_nodes: 2_000_000, max_attrs: 1024, max_attrs_total: 4_000_000 }
    );
    for e in [Error::TooLarge(1), Error::TooDeep(1), Error::TooManyNodes(1), Error::TooManyAttrs(1), Error::TooManyAttrsTotal(1)] {
        assert!(e.is_limit(), "{e}");
    }
    assert!(!Error::NoRoot.is_limit() && !Error::Doctype.is_limit());
}

#[test]
fn a_depth_bomb_is_refused_at_the_limit() {
    let limits = Limits { max_depth: 8, ..Limits::DEFAULT };
    let nested = |n: usize| format!("{}{}", "<a>".repeat(n), "</a>".repeat(n));
    for opts in all_options() {
        let opts = opts.with_limits(limits);
        assert!(opts.parse(nested(8).as_bytes()).is_ok());
        assert_eq!(opts.parse(nested(9).as_bytes()), Err(Error::TooDeep(8)));
        assert_eq!(opts.parse("<a>".repeat(9).as_bytes()), Err(Error::TooDeep(8)), "unclosed tags are depth too");
        assert_eq!(opts.parse_streaming(nested(9).as_bytes(), &["a"], |_| Ok::<(), Error>(())), Err(Error::TooDeep(8)));
    }
    // A megabyte of open tags against the default limit.
    assert_eq!(parse("<a>".repeat(350_000).as_bytes()), Err(Error::TooDeep(256)));
    // Text and deep traversals of the deepest tree the reader builds do not recurse.
    let deepest = parse(format!("{}x{}", "<a>".repeat(256), "</a>".repeat(256)).as_bytes()).unwrap();
    assert_eq!((deepest.deep_text().as_str(), deepest.descendants().count()), ("x", 255));
    assert_eq!(parse(deepest.to_xml().as_bytes()).unwrap(), deepest);
    assert_eq!(Options { drop_blank_text: true, ..Options::default() }.parse(deepest.to_xml_pretty(" ").as_bytes()).unwrap(), deepest);
}

#[test]
fn a_node_count_bomb_is_refused_at_the_limit() {
    let limits = Limits { max_nodes: 100, ..Limits::DEFAULT };
    for opts in all_options() {
        let opts = opts.with_limits(limits);
        // The root and 99 children are 100 nodes.
        assert!(opts.parse(format!("<r>{}</r>", "<x/>".repeat(99)).as_bytes()).is_ok());
        assert_eq!(opts.parse(format!("<r>{}</r>", "<x/>".repeat(100)).as_bytes()), Err(Error::TooManyNodes(100)));
        // Text between elements is nodes too.
        assert_eq!(opts.parse(format!("<r>{}</r>", "t<x/>".repeat(50)).as_bytes()), Err(Error::TooManyNodes(100)));
        assert_eq!(opts.parse(format!("<r>{}", "<x>".repeat(100)).as_bytes()).err().map(|e| e.is_limit() || opts.strict), Some(true));
    }
    // Kept comments, processing instructions and CDATA sections count.
    let keep = Options { cdata: true, pis: true, comments: true, ..Options::default() }.with_limits(limits);
    for filler in ["<!--c-->", "<?p d?>", "<![CDATA[x]]><x/>"] {
        assert_eq!(keep.parse(format!("<r>{}</r>", filler.repeat(100)).as_bytes()), Err(Error::TooManyNodes(100)), "{filler}");
        assert!(Options::default().with_limits(limits).parse(format!("<r>{}</r>", filler.repeat(40)).as_bytes()).is_ok(), "{filler}");
    }
    // Indentation that is dropped does not count against the tree that is kept.
    let indented = format!("<r>{}</r>", "\n  <x/>".repeat(99));
    assert!(
        Options { drop_blank_text: true, ..Options::default() }
            .with_limits(Limits { max_nodes: 200, ..Limits::DEFAULT })
            .parse(indented.as_bytes())
            .is_ok()
    );
    // The default limit, on a part of endless empty elements.
    let endless = format!("<r>{}</r>", "<x/>".repeat(2_000_000));
    assert_eq!(parse(endless.as_bytes()), Err(Error::TooManyNodes(2_000_000)));
}

#[test]
fn an_attribute_flood_is_refused_at_the_limit() {
    let limits = Limits { max_attrs: 16, ..Limits::DEFAULT };
    let tag = |n: usize, attr: &dyn Fn(usize) -> String| format!("<a{}/>", (0..n).map(attr).collect::<String>());
    let kinds: [&dyn Fn(usize) -> String; 4] =
        [&|i| format!(" a{i}='v'"), &|_| " a='v'".to_string(), &|i| format!(" xmlns:p{i}='urn:{i}'"), &|i| format!(" bad{i}")];
    for opts in all_options() {
        let opts = opts.with_limits(limits);
        for (k, kind) in kinds.iter().enumerate() {
            let _ = opts.parse(tag(16, kind).as_bytes());
            assert_eq!(opts.parse(tag(17, kind).as_bytes()).err().map(|e| e.is_limit() || (opts.strict && k == 3)), Some(true), "kind {k}");
            assert!(opts.first_start(tag(17, kind).as_bytes(), "a").is_none());
        }
        assert_eq!(opts.parse(tag(17, kinds[0]).as_bytes()), Err(Error::TooManyAttrs(16)));
    }
    assert_eq!(parse(tag(200_000, kinds[0]).as_bytes()), Err(Error::TooManyAttrs(1024)));
    // Namespace declarations spread over many levels are bounded as a whole where names are
    // resolved against them: 3 levels of 6 is over 16, though no tag has more than 6.
    let level = |d: usize| format!("<w:a{}>", (0..6).map(|i| format!(" xmlns:p{d}_{i}='urn:x'")).collect::<String>());
    let spread = |levels: usize| format!("<w:r xmlns:w='urn:w'>{}", (0..levels).map(level).collect::<String>());
    let canonical = Options::canonical(&CANON).with_limits(limits);
    assert_eq!(canonical.parse(spread(2).as_bytes()).map(|r| r.name), Ok("w:r".to_string()));
    assert_eq!(canonical.parse(spread(3).as_bytes()), Err(Error::TooManyAttrs(16)));
    assert!(Options::default().with_limits(limits).parse(spread(3).as_bytes()).is_ok(), "names as written resolve nothing");
}

#[test]
fn input_over_the_size_limit_is_refused_before_it_is_read() {
    let opts = Options::default().with_limits(Limits { max_input: 1000, ..Limits::DEFAULT });
    let doc = |len: usize| format!("<a>{}</a>", "x".repeat(len - 7));
    assert!(opts.parse(doc(1000).as_bytes()).is_ok());
    assert_eq!(opts.parse(doc(1001).as_bytes()), Err(Error::TooLarge(1000)));
    assert_eq!(opts.parse_streaming(doc(1001).as_bytes(), &[], |_| Ok::<(), Error>(())), Err(Error::TooLarge(1000)));
    assert!(opts.first_start(doc(1001).as_bytes(), "a").is_none());
}

/// The "billion laughs": entities declared in a DTD are never expanded, whatever the options.
#[test]
fn declared_entities_are_never_expanded() {
    let mut dtd = String::from("<?xml version=\"1.0\"?><!DOCTYPE lolz [<!ENTITY lol \"lol\">");
    for i in 1..10 {
        let prev = if i == 1 { "lol".to_string() } else { format!("lol{}", i - 1) };
        dtd.push_str(&format!("<!ENTITY lol{i} \"{}\">", format!("&{prev};").repeat(10)));
    }
    let bomb = format!("{dtd}]><lolz a=\"&lol9;\">&lol9;</lolz>");
    let quadratic = format!("<!DOCTYPE b [<!ENTITY a \"{}\">]><b>{}</b>", "a".repeat(50_000), "&a;".repeat(50_000));
    let external =
        "<!DOCTYPE x [<!ENTITY xxe SYSTEM \"file:///etc/passwd\"><!ENTITY % pe SYSTEM \"http://127.0.0.1/x.dtd\">%pe;]><x a=\"&xxe;\">&xxe;</x>";
    for doc in [bomb.as_str(), quadratic.as_str(), external] {
        for opts in all_options() {
            match opts.parse(doc.as_bytes()) {
                Ok(root) => {
                    assert!(text_len(&root) <= doc.len(), "the tree is not larger than the document");
                    let text = root.text().into_owned() + root.attr("a").unwrap_or("");
                    assert!(!text.contains("lol") || text.contains("&lol9;"), "{text}");
                    assert!(!text.contains("root:"), "an external entity is never fetched");
                    match opts.unknown_entity {
                        UnknownEntity::Drop => assert_eq!(text, ""),
                        UnknownEntity::Literal => assert!(text.starts_with('&') && text.ends_with(';'), "{text}"),
                        UnknownEntity::Error => panic!("an undefined entity was accepted"),
                    }
                }
                Err(Error::Doctype) => assert_eq!(opts.doctype, Doctype::Reject),
                Err(Error::UnknownEntity { .. }) => assert_eq!(opts.unknown_entity, UnknownEntity::Error),
                Err(e) => panic!("unexpected error {e}"),
            }
        }
    }
    let refuse = Options { doctype: Doctype::RejectInternalSubset, ..Options::default() };
    assert_eq!(refuse.parse(bomb.as_bytes()), Err(Error::Doctype));
    assert_eq!(refuse.parse(external.as_bytes()), Err(Error::Doctype));
    // Predefined entities and character references are the only expansion, and they shrink.
    let refs = format!("<a v=\"{0}\">{0}</a>", "&amp;&#x10FFFF;&#65;&lt;".repeat(10_000));
    let root = parse(refs.as_bytes()).unwrap();
    assert_eq!(root.text().chars().count(), 40_000);
    assert!(text_len(&root) < refs.len());
}

const DOCUMENTS: &[&str] = &[
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"urn:w\" xmlns:r=\"urn:r\"><w:body><w:p w:rsidR=\"00A1\" r:id='rId1'><w:r><w:t xml:space=\"preserve\"> a &amp; b &#x41;&#65; </w:t></w:r></w:p><w:sectPr/></w:body></w:document>\n",
    "<!DOCTYPE r [<!ENTITY e \"x\"><!ELEMENT r ANY>]>\n<r a=\"1\" b='2&quot;'><!-- comment --><?pi data?><![CDATA[c <d> ]]> e]]>&e;&bogus;<x/>text<y>\r\n</y></r><!-- after --><z/>",
    "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 1 1\">\n  <text x='1'>Héllo <tspan>日本語</tspan> wörld😀</text>\n  <use xlink:href=\"#a\"/>\n</svg>",
    "<sst><si><t>one</t></si><si><r><t>two</t></r><r><t xml:space='preserve'> three</t></r></si><si/></sst>",
];

/// Every prefix of a few documents, read every way: an error or a tree, never a panic. A strict
/// read of a cut document is an error unless only what follows the root was cut.
#[test]
fn input_truncated_at_every_byte_never_panics() {
    for doc in DOCUMENTS {
        let bytes = doc.as_bytes();
        let whole_strict_ok = Options::default().strict().parse(bytes).is_ok();
        for cut in 0..=bytes.len() {
            let part = &bytes[..cut];
            for opts in all_options() {
                let tree = opts.parse(part);
                let _ = opts.parse_streaming(part, &["si", "w:p", "x", "y"], |_| Ok::<(), Error>(()));
                let _ = opts.first_start(part, "t");
                let mut events = opts.events(part);
                while let Ok(Some(_)) = events.next() {}
                if let Ok(root) = &tree {
                    assert!(text_len(root) <= part.len() * 3 + 16);
                    let _ = (root.to_xml(), root.to_xml_pretty("  "), root.deep_text());
                }
            }
            let strict = Options::default().strict().parse(part);
            let root_ended = doc.rfind("</").is_some_and(|i| doc[i..].find('>').is_some_and(|j| cut > i + j));
            if whole_strict_ok && !root_ended {
                assert!(strict.is_err(), "a strict read accepted {:?}", String::from_utf8_lossy(part));
            }
        }
    }
}

/// The same documents with bytes overwritten, deleted and doubled: a cheap deterministic fuzz.
#[test]
fn damaged_input_never_panics() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let junk = b"<>&;'\"/=?![]-\x00\xff\xfe\xc3\xed\xa0\x80 :x#";
    for doc in DOCUMENTS {
        for _ in 0..600 {
            let mut bytes = doc.as_bytes().to_vec();
            for _ in 0..(next() % 4 + 1) {
                let at = (next() % bytes.len() as u64) as usize;
                match next() % 3 {
                    0 => bytes[at] = junk[(next() % junk.len() as u64) as usize],
                    1 => {
                        bytes.remove(at);
                    }
                    _ => bytes.insert(at, bytes[at]),
                }
            }
            for opts in all_options() {
                if let Ok(root) = opts.parse(&bytes) {
                    assert!(text_len(&root) <= bytes.len() * 3 + 16);
                    // Whatever was written reads back. (A damaged name that is no XML name is
                    // not written at all: the reader does not check names, the writer does.)
                    let written = root.to_xml();
                    if !written.is_empty() {
                        let again = Options { names: craft_xml::Names::AsWritten, strict: false, ns_decls: true, ..opts }.parse(written.as_bytes());
                        assert!(again.is_ok(), "{written:?}");
                        assert_eq!(again.map(|e| e.to_xml()), Ok(written));
                    }
                }
                let _ = opts.parse_streaming(&bytes, &["si", "w:p"], |_| Ok::<(), Error>(()));
                let _ = opts.first_start(&bytes, "t");
            }
        }
    }
}

#[test]
fn invalid_utf8_is_replaced_or_refused() {
    let cases: [&[u8]; 6] = [
        b"<a>\xff</a>",
        b"<a b='\xc3\x28'/>",
        b"<a><![CDATA[\xed\xa0\x80]]></a>",
        b"<\xff\xfe/>",
        b"<a \xf0\x9f='1'/>",
        b"<a><?p\xff \xfe?><!--\xff--></a>",
    ];
    let keep = Options { cdata: true, pis: true, comments: true, ..Options::default() };
    for bytes in cases {
        for opts in [Options::default(), Options::local(), Options::canonical(&CANON), keep] {
            let lenient = opts.parse(bytes).unwrap();
            assert!(std::str::from_utf8(lenient.to_xml().as_bytes()).is_ok());
            assert!(matches!(opts.strict().parse(bytes), Err(Error::Encoding(_) | Error::Syntax { .. })), "{:?}", String::from_utf8_lossy(bytes));
        }
        assert!(format!("{:?}", keep.parse(bytes).unwrap()).contains('\u{FFFD}'), "each bad byte is U+FFFD where it is kept");
    }
    assert_eq!(parse(b"<a>\xff</a>").unwrap().text(), "\u{FFFD}");
    assert_eq!(Options::default().strict().parse(b"<a>ok\xff</a>"), Err(Error::Encoding(5)));
}

#[test]
fn invalid_utf16_never_panics() {
    let good: Vec<u16> = "<a k='v'>t</a>".encode_utf16().collect();
    let le = |units: &[u16]| [0xFF, 0xFE].into_iter().chain(units.iter().flat_map(|u| u.to_le_bytes())).collect::<Vec<u8>>();
    let be = |units: &[u16]| [0xFE, 0xFF].into_iter().chain(units.iter().flat_map(|u| u.to_be_bytes())).collect::<Vec<u8>>();
    assert_eq!(parse(&le(&good)).unwrap().attr("k"), Some("v"));
    assert_eq!(parse(&be(&good)).unwrap().text(), "t");
    for at in 0..good.len() {
        for bad in [0xD800u16, 0xDC00, 0xFFFF, 0x0000] {
            let mut units = good.clone();
            units[at] = bad;
            for bytes in [le(&units), be(&units)] {
                for cut in [bytes.len(), bytes.len() - 1, bytes.len() / 2] {
                    for opts in all_options() {
                        let _ = opts.parse(&bytes[..cut]);
                    }
                }
            }
        }
    }
    // An unpaired surrogate is U+FFFD; a trailing odd byte is dropped.
    assert_eq!(parse(&le(&[0x3C, 0x61, 0x3E, 0xD800, 0x3C, 0x2F, 0x61, 0x3E])).unwrap().text(), "\u{FFFD}");
    assert_eq!(to_utf8(&[0xFE, 0xFF, 0x00, 0x41, 0x00]).as_ref(), b"A");
    for empty in [&[0xFF, 0xFE][..], &[0xFE, 0xFF], &[0xEF, 0xBB, 0xBF], &[0xFF], &[]] {
        assert_eq!(parse(empty), Err(Error::NoRoot));
    }
}

#[test]
fn mismatched_tags_are_repaired_or_refused() {
    let cases = ["<a><b></a>", "<a></b>", "<a><b></c></a>", "</a><a/>", "<a></a></a>", "<a><b></a></b>", "<a></a><b></b>", "<a><b>", "<a/><a/>"];
    for xml in cases {
        for opts in all_options() {
            let got = opts.parse(xml.as_bytes());
            if opts.strict {
                assert!(matches!(got, Err(Error::Syntax { .. })), "{xml}: {got:?}");
            } else {
                assert_eq!(got.map(|e| e.local().to_string()), Ok("a".to_string()), "{xml}");
            }
        }
    }
    // A lenient end tag closes the innermost open element, whatever its name.
    let root = parse(b"<a><b>x</c>y</a>").unwrap();
    assert_eq!(root.to_xml(), "<a><b>x</b>y</a>");
    // Nothing after the root element is read by a lenient parse, however broken.
    assert_eq!(parse(b"<a/><<<\xff&&&").unwrap().name, "a");
}

#[test]
fn markup_that_is_not_xml_is_an_error() {
    for bytes in [
        &b""[..],
        b" ",
        b"text",
        b"<",
        b"<a",
        b"<a b='1",
        b"<!-- open",
        b"<![CDATA[x",
        b"<?pi",
        b"<!DOCTYPE a [",
        b"\x00\x01\x02",
        b"<?xml version='1.0'?>",
    ] {
        for opts in all_options() {
            assert!(opts.parse(bytes).is_err(), "{:?}", String::from_utf8_lossy(bytes));
            assert!(opts.first_start(bytes, "a").is_none() || bytes.starts_with(b"<a "));
        }
    }
}

/// Attributes are the one thing a node count does not see: 1,025 elements of 1,024 attributes
/// each are 1,025 nodes and 1,049,600 `(String, String)` pairs, about 50 MB of headers from
/// 5 MB of input.
#[test]
fn attributes_are_bounded_over_the_whole_tree() {
    let el = format!("<c{}/>", " a=\"\"".repeat(1024));
    let doc = format!("<r>{}</r>", el.repeat(1025));
    let limits = Limits { max_attrs_total: 1024 * 1024, ..Limits::DEFAULT };
    let opts = Options::default().with_limits(limits);
    let err = opts.parse(doc.as_bytes()).unwrap_err();
    assert_eq!(err, Error::TooManyAttrsTotal(1024 * 1024));
    assert!(err.is_limit());
    // exactly at the limit is accepted
    let fits = format!("<r>{}</r>", el.repeat(1024));
    assert_eq!(opts.parse(fits.as_bytes()).map(|r| r.children.len()), Ok(1024));
    // the default stops a document that is nothing but attributes
    let hostile = format!("<r>{}</r>", el.repeat(4000));
    assert_eq!(parse(hostile.as_bytes()), Err(Error::TooManyAttrsTotal(4_000_000)));
    // streamed elements take their attributes with them: any number of rows, as long as the
    // ones held at once fit
    let mut rows = 0;
    let root = opts.parse_streaming(doc.as_bytes(), &["c"], |c| {
        rows += c.attrs.len();
        Ok::<(), Error>(())
    });
    assert_eq!((rows, root.map(|r| r.children.len())), (1025 * 1024, Ok(0)));
    let tight = Options::default().with_limits(Limits { max_attrs_total: 1023, ..Limits::DEFAULT });
    assert_eq!(tight.parse_streaming(doc.as_bytes(), &["c"], |_| Ok::<(), Error>(())), Err(Error::TooManyAttrsTotal(1023)));
}

/// An internal subset is found wherever it hides, and looking for it is linear in the input.
#[test]
fn an_internal_subset_is_refused_wherever_it_hides() {
    let opts = Options { doctype: Doctype::RejectInternalSubset, ..Options::default() };
    assert!(opts.parse(b"<!DOCTYPE xmeml><xmeml/>").is_ok());
    assert!(opts.parse(b"<!DOCTYPE xmeml SYSTEM \"a>b\"><xmeml/>").is_ok());
    // the first marker sits in a comment; the real declaration follows
    let hidden = "<!-- <!DOCTYPE x> --><!DOCTYPE xmeml [<!ENTITY a \"aaaa\">]><xmeml>&a;</xmeml>";
    assert_eq!(opts.parse(hidden.as_bytes()), Err(Error::Doctype));
    // a `>` inside the system literal does not end the declaration early
    let quoted = "<!DOCTYPE xmeml SYSTEM \"a>b\" [<!ENTITY a \"aaaa\">]><xmeml>&a;</xmeml>";
    assert_eq!(opts.parse(quoted.as_bytes()), Err(Error::Doctype));
    assert_eq!(opts.parse(b"<?pi <!DOCTYPE a> ?><!DOCTYPE x [<!ENTITY a \"b\">]><x/>"), Err(Error::Doctype));
    // many markers in one comment, and many that never close
    let marker = "<!DOCTYPE ";
    let in_comment = format!("<!--{}--><x/>", marker.repeat(100_000));
    let unclosed = format!("<x>{}", marker.repeat(100_000));
    let start = std::time::Instant::now();
    assert!(opts.parse(in_comment.as_bytes()).is_ok());
    let _ = opts.parse(unclosed.as_bytes());
    assert!(start.elapsed() < std::time::Duration::from_secs(5), "{:?}", start.elapsed());
}

/// A reader of many parts keeps one budget: each tree is charged at its node count and the next
/// part is read with what is left, so it stops before its tree is built.
#[test]
fn a_node_count_charges_a_budget_shared_by_several_parts() {
    let part = format!("<r>{}</r>", "<a>t</a>".repeat(100));
    let mut left = 450_usize;
    let mut read = 0;
    for _ in 0..4 {
        let opts = Options::default().with_limits(Limits { max_nodes: left, ..Limits::DEFAULT });
        match opts.parse(part.as_bytes()) {
            Ok(root) => {
                // the root, 100 elements and 100 text nodes
                assert_eq!(root.node_count(), 201);
                left -= root.node_count();
                read += 1;
            }
            Err(e) => assert_eq!(e, Error::TooManyNodes(48)),
        }
    }
    assert_eq!((read, left), (2, 48));
    // exactly what the limit counts: a tree of n nodes reads with max_nodes n, not n - 1
    let exact = Options::default().with_limits(Limits { max_nodes: 201, ..Limits::DEFAULT });
    assert!(exact.parse(part.as_bytes()).is_ok());
    let short = Options::default().with_limits(Limits { max_nodes: 200, ..Limits::DEFAULT });
    assert_eq!(short.parse(part.as_bytes()), Err(Error::TooManyNodes(200)));
}

/// A part that cannot be read to its end still gives what came before the failure.
#[test]
fn a_failed_streaming_read_returns_what_it_had() {
    let opts = Options::local().with_limits(Limits { max_nodes: 1000, ..Limits::DEFAULT });
    let sheet = |row: &str| format!("<worksheet><cols><col min='1'/></cols><sheetData><row r='1'/>{row}</sheetData><mergeCells/></worksheet>");
    // one row over the node limit
    let huge = sheet(&format!("<row r='2'>{}</row>", "<c/>".repeat(1001)));
    let mut rows = Vec::new();
    let (root, err) = opts.parse_streaming_partial(huge.as_bytes(), &["row"], |r| {
        rows.push(r.attr_u32("r"));
        Ok::<(), Error>(())
    });
    assert_eq!(err, Some(Error::TooManyNodes(1000)));
    let root = root.unwrap();
    assert!(root.child("cols").is_some(), "what was complete before the error is kept");
    assert!(root.child("mergeCells").is_none(), "nothing after it is");
    assert_eq!(rows, [Some(1)], "the row that failed is not handed out");
    assert_eq!(
        root.child("sheetData").and_then(|d| d.child("row")).and_then(|r| r.attr_u32("r")),
        Some(2),
        "it is in the tree, as far as it was read"
    );
    // the same input through parse_streaming is the error alone
    assert_eq!(opts.parse_streaming(huge.as_bytes(), &["row"], |_| Ok::<(), Error>(())), Err(Error::TooManyNodes(1000)));
    // an error from the callback
    let (root, err) = opts.parse_streaming_partial(sheet("").as_bytes(), &["row"], |_| Err(Error::NoRoot));
    assert_eq!((root.map(|r| r.children.len()), err), (Some(2), Some(Error::NoRoot)));
    // markup the tokenizer cannot read
    let (root, err) = opts.parse_streaming_partial(b"<worksheet><cols/><sheetData><row b='1", &["row"], |_| Ok::<(), Error>(()));
    assert!(matches!(err, Some(Error::Syntax { .. })), "{err:?}");
    assert!(root.is_some_and(|r| r.child("cols").is_some()));
    // a whole document, and nothing at all
    let (root, err) = opts.parse_streaming_partial(sheet("").as_bytes(), &["row"], |_| Ok::<(), Error>(()));
    assert_eq!((root.map(|r| r.children.len()), err), (Some(3), None));
    assert_eq!(opts.parse_streaming_partial(b"", &[], |_| Ok::<(), Error>(())), (None, Some(Error::NoRoot)));
    let tiny = Options::default().with_limits(Limits { max_input: 2, ..Limits::DEFAULT });
    assert_eq!(tiny.parse_streaming_partial(b"<a/>", &[], |_| Ok::<(), Error>(())), (None, Some(Error::TooLarge(2))));
}

/// The pull reader builds no tree, so a strict one has to refuse by itself what the tree reader
/// refuses: a second root element and character data outside the root. A reader that stopped
/// believing a damaged file would otherwise take its first element for the whole document.
#[test]
fn a_strict_pull_reader_refuses_what_is_outside_the_root() {
    fn read(opts: Options<'_>, doc: &[u8]) -> Result<usize, Error> {
        let mut events = opts.events(doc);
        let mut n = 0;
        while events.next()?.is_some() {
            n += 1;
        }
        Ok(n)
    }
    let strict = Options::default().strict();
    for (doc, msg) in [
        (&b"<a/><b/>"[..], "more than one root element"),
        (b"<a></a><b>", "more than one root element"),
        (b"<a/>text", "text outside the root element"),
        (b"text<a/>", "text outside the root element"),
        (b"<a/><![CDATA[x]]>", "text outside the root element"),
        (b"<a/>&amp;", "text outside the root element"),
    ] {
        let shown = String::from_utf8_lossy(doc);
        assert!(matches!(read(strict, doc), Err(Error::Syntax { msg: m, .. }) if m == msg), "{shown}");
        assert_eq!(strict.parse(doc).map(|_| ()).map_err(|e| matches!(e, Error::Syntax { .. })), Err(true), "{shown}");
        assert!(read(Options::default(), doc).is_ok(), "a lenient read takes {shown}");
    }
    // white space, comments and instructions may stand outside the root
    assert!(read(strict, b"<?xml version='1.0'?>\r\n <!-- c -->\t<a>x</a>\n<?pi?> ").is_ok());
}
