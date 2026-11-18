use super::{BARE_NAMES, GREEK_LOWER, GREEK_UPPER, LATIN1, decode_entity, escape, escape_dq, named, reference, unescape, unescape_attr};

#[test]
fn escapes_the_five() {
    assert_eq!(escape(r#"<a href="x">Tom & 'Jerry'</a>"#), "&lt;a href=&quot;x&quot;&gt;Tom &amp; &#39;Jerry&#39;&lt;/a&gt;");
    assert_eq!(escape_dq(r#"<a href="x">Tom & 'Jerry'</a>"#), "&lt;a href=&quot;x&quot;&gt;Tom &amp; 'Jerry'&lt;/a&gt;");
    assert_eq!(escape("plain ünïcode"), "plain ünïcode");
    assert_eq!(escape(""), "");
}

#[test]
fn bare_names_are_sorted_and_known() {
    assert_eq!(BARE_NAMES.len(), 94);
    assert!(BARE_NAMES.windows(2).all(|w| w[0] < w[1]));
    assert!(BARE_NAMES.iter().all(|n| (2..=6).contains(&n.len()) && named(n).is_some()));
}

#[test]
fn every_latin_1_and_greek_name_is_its_code_point() {
    for (table, base) in [(&LATIN1[..], 0xA0), (&GREEK_UPPER[..], 0x391), (&GREEK_LOWER[..], 0x3B1)] {
        for (i, name) in table.iter().enumerate().filter(|(_, n)| !n.is_empty()) {
            assert_eq!(named(name), char::from_u32(base + i as u32), "{name}");
        }
    }
}

#[test]
fn named_references() {
    assert_eq!(unescape("&lt;b&gt; &amp; &quot;x&quot; &apos;y&apos;"), "<b> & \"x\" 'y'");
    assert_eq!(unescape("a&nbsp;b"), "a\u{a0}b");
    assert_eq!(unescape("&copy; &reg; &trade; &hellip; &mdash; &ndash;"), "© ® ™ … — –");
    assert_eq!(unescape("&lsquo;a&rsquo; &ldquo;b&rdquo; &euro;5"), "‘a’ “b” €5");
    assert_eq!(unescape("caf&eacute; na&iuml;ve &Uuml;ber &szlig; &yuml;"), "café naïve Über ß ÿ");
    assert_eq!(unescape("&alpha;&beta;&Omega;&sigmaf; &rarr; &le; &ne;"), "αβΩς → ≤ ≠");
    assert_eq!(unescape("&zwnj;&brvbar;&ETH;&thorn;&Alpha;&omicron;&cross;&check;&diams;"), "\u{200c}¦Ðþ\u{391}ο✗✓♦");
    assert_eq!(unescape("&frac13;&there4;&hArr;&loz;"), "⅓∴⇔◊");
}

#[test]
fn names_are_case_sensitive() {
    assert_eq!(unescape("&Eacute;&eacute;"), "Éé");
    assert_eq!(unescape("&EACUTE;"), "&EACUTE;");
    assert_eq!(unescape("&AMP;&LT;"), "&<");
}

#[test]
fn numeric_references() {
    assert_eq!(unescape("&#233;&#xE9;&#Xe9;"), "ééé");
    assert_eq!(unescape("&#128512;"), "😀");
    assert_eq!(unescape("&#x1F600;"), "😀");
    // Without the semicolon.
    assert_eq!(unescape("&#65 &#x42."), "A B.");
    // Windows-1252 for the C1 range.
    assert_eq!(unescape("&#147;hi&#148; &#128; &#x99;"), "“hi” € ™");
}

#[test]
fn bad_numbers_become_the_replacement_character() {
    assert_eq!(unescape("&#0;"), "\u{fffd}");
    assert_eq!(unescape("&#xD800;"), "\u{fffd}");
    assert_eq!(unescape("&#x110000;"), "\u{fffd}");
    assert_eq!(unescape("&#99999999999999999999999999;"), "\u{fffd}");
    assert_eq!(unescape("&#xFFFFFFFFFFFFFFFFFFFF;"), "\u{fffd}");
}

#[test]
fn invalid_references_are_left_alone() {
    for s in ["&", "&&", "& ", "&;", "&#;", "&#x;", "&#xZZ;", "&bogus;", "AT&T", "a & b", "x&y=z", "&#", "&#x", "&eacut", "ünï&", "&Alpha", "&hellip"]
    {
        assert_eq!(unescape(s), s, "{s}");
        assert_eq!(unescape_attr(s), s, "{s}");
    }
}

#[test]
fn legacy_names_work_without_a_semicolon() {
    assert_eq!(unescape("a &lt b &gt c"), "a < b > c");
    assert_eq!(unescape("x&nbsp"), "x\u{a0}");
    assert_eq!(unescape("&amp"), "&");
    assert_eq!(unescape("&copy 2024 caf&eacute!"), "© 2024 café!");
    // The longest name wins: `&notin;` is one reference; `&yenx;` is `&yen` and `x;`.
    assert_eq!(unescape("&notin; &yenx;"), "∉ ¥x;");
    // In text even inside a word, as browsers read it.
    assert_eq!(unescape("&amplify ?a=1&amp=2"), "&lify ?a=1&=2");
}

#[test]
fn in_an_attribute_a_bare_name_before_a_letter_digit_or_equals_is_text() {
    assert_eq!(unescape_attr("?a=1&amp=2&lt=3&copy=4&reg1"), "?a=1&amp=2&lt=3&copy=4&reg1");
    assert_eq!(unescape_attr("&amplify"), "&amplify");
    assert_eq!(unescape_attr("?a=1&amp;b=2"), "?a=1&b=2");
    assert_eq!(unescape_attr("a &lt b&copy"), "a < b©");
    // Numeric references need no `;` anywhere.
    assert_eq!(unescape_attr("&#65=1"), "A=1");
}

#[test]
fn an_entity_body_decodes_whole_or_not_at_all() {
    assert_eq!(decode_entity("amp"), Some('&'));
    assert_eq!(decode_entity("#38"), Some('&'));
    assert_eq!(decode_entity("#x26"), Some('&'));
    assert_eq!(decode_entity("#150"), Some('–'));
    for none in ["", "#", "#x", "#38x", "#3 8", "bogus", "amp;"] {
        assert_eq!(decode_entity(none), None, "{none}");
    }
    assert_eq!(reference("amp;rest", false), Some(('&', 4)));
    assert_eq!(reference("#x41;", true), Some(('A', 5)));
}

#[test]
fn a_long_run_of_ampersands_is_quick() {
    let s = "&".repeat(200_000);
    assert_eq!(unescape(&s), s);
    let s = "&#".repeat(100_000);
    assert_eq!(unescape(&s), s);
    let s = "&aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".repeat(20_000);
    assert_eq!(unescape(&s), s);
}

proptest::proptest! {
    #[test]
    fn unescape_undoes_escape(s in proptest::prelude::any::<String>()) {
        proptest::prop_assert_eq!(unescape(&escape(&s)), s.clone());
        proptest::prop_assert_eq!(unescape_attr(&escape_dq(&s)), s);
    }

    #[test]
    fn unescape_never_panics(s in "[&#xX0-9a-zA-Z;= é]{0,60}") {
        let _ = unescape(&s);
        let _ = unescape_attr(&s);
    }
}
