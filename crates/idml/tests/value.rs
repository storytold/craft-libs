//! Attribute value codecs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_idml::value::*;

#[test]
fn numbers() {
    assert_eq!(num(12.0), "12");
    assert_eq!(num(-0.0), "0");
    assert_eq!(num(0.1 + 0.2), "0.3");
    assert_eq!(num(1.0 / 3.0), "0.333333333");
    assert_eq!(num(-1e-12), "0");
    assert_eq!(num(f64::NAN), "0");
    assert_eq!(num(f64::INFINITY), "0");
    assert_eq!(num(123456.5), "123456.5");
    assert_eq!(point_str(1.5, -2.0), "1.5 -2");
    assert_eq!(nums(" 1 2.5\t-3 x "), [1.0, 2.5, -3.0]);
    assert_eq!(nums("1 NaN 3"), Vec::<f64>::new(), "one bad number drops the list, not just itself");
    assert_eq!(nums("1 inf"), Vec::<f64>::new());
}

#[test]
fn transforms_and_points() {
    assert_eq!(matrix("1 0 0 1 36 -72.5"), Some([1.0, 0.0, 0.0, 1.0, 36.0, -72.5]));
    assert_eq!(matrix("1 0 0 1 36"), None);
    assert_eq!(matrix("1 0 0 1 36 72 9"), None);
    assert_eq!(matrix("1 0 0 1 NaN 0"), None);
    assert_eq!(matrix_str(&[1.0, 0.0, 0.0, 1.0, 36.0, -72.5]), "1 0 0 1 36 -72.5");
    assert_eq!(point("3 4"), Some((3.0, 4.0)));
    assert_eq!(point("3 4 5"), Some((3.0, 4.0)));
    assert_eq!(point("3"), None);
    assert_eq!(point("3 inf"), None);
}

#[test]
fn tints() {
    assert_eq!(tint(-1.0), 1.0);
    assert_eq!(tint(50.0), 0.5);
    assert_eq!(tint(250.0), 1.0);
    assert_eq!(tint(f64::NAN), 1.0);
}

#[test]
fn ids_and_names() {
    assert_eq!(escape_id("English: USA 100%"), "English%3a USA 100%25");
    assert_eq!(unescape_id("English%3a USA 100%25"), "English: USA 100%");
    assert_eq!(unescape_id("A%3AB"), "A:B");
    assert_eq!(uid(255), "uff");
    let builtins = [("[None]", "$ID/[No character style]")];
    assert_eq!(style_name_out(&builtins, "[None]"), "$ID/[No character style]");
    assert_eq!(style_name_in(&builtins, "$ID/[No character style]"), "[None]");
    for name in ["Group/Sub/Name", "A:B", "back\\slash", "a\\:b/c", "end\\"] {
        let out = style_name_out(&builtins, name);
        assert!(!out.contains('/'), "{out}");
        assert_eq!(style_name_in(&builtins, &out), name, "{out}");
    }
    assert_eq!(style_name_out(&builtins, "Group/A:B"), "Group:A\\:B");
    assert_eq!(style_name_in(&builtins, "$ID/NormalParagraphStyle"), "NormalParagraphStyle");
    assert_eq!(style_self("CharacterStyle", &builtins, "Group/Em"), "CharacterStyle/Group%3aEm");
}

#[test]
fn font_families() {
    assert_eq!(font_family("Minion Pro").as_deref(), Some("Minion Pro"));
    assert_eq!(font_family("Minion Pro ").as_deref(), Some("Minion Pro "), "a trailing space is another font");
    assert_eq!(font_family("$ID/Arial").as_deref(), Some("Arial"));
    assert_eq!(font_family("Arial\tBold").as_deref(), Some("Arial"));
    assert_eq!(font_family("$ID/"), None);
    assert_eq!(font_family(" \n"), None);
}

#[test]
fn xmp_values() {
    let xmp = "<x:xmpmeta><rdf:Description xmp:ModifyDate=\"2026-02-03\"><xmp:CreateDate>2026-01-02</xmp:CreateDate></rdf:Description>";
    assert_eq!(xmp_value(xmp, "xmp:CreateDate"), Some("2026-01-02"));
    assert_eq!(xmp_value(xmp, "xmp:ModifyDate"), Some("2026-02-03"));
    assert_eq!(xmp_value(xmp, "xmp:MetadataDate"), None);
    assert_eq!(xmp_value("<xmp:CreateDate>cut", "xmp:CreateDate"), None);
    assert_eq!(xmp_value("xmp:CreateDate=\"cut", "xmp:CreateDate"), None);
}

#[test]
fn base64() {
    for len in [0usize, 1, 2, 3, 56, 57, 58, 200] {
        let data: Vec<u8> = (0..len).map(|i| (i * 37 % 251) as u8).collect();
        let enc = base64_encode(&data);
        assert!(enc.lines().all(|l| l.len() <= 76), "{enc}");
        assert_eq!(base64_decode(&enc), data, "{len}");
    }
    assert_eq!(base64_encode(b"hi"), "aGk=");
    assert_eq!(base64_encode(&[0u8; 58]).lines().count(), 2);
    assert_eq!(base64_decode(" aG\r\nk=\t*<>"), b"hi");
    assert_eq!(base64_decode("-_"), base64_decode("+/"));
}

#[test]
fn link_uris() {
    assert_eq!(percent_decode("a%20b%zz%4"), "a b%zz%4");
    assert_eq!(percent_decode("%ff"), "\u{FFFD}");
    assert_eq!(uri_to_path("file:/a%20b/c.png"), "/a b/c.png");
    assert_eq!(uri_to_path("file:///C:/x/y.png"), "C:/x/y.png");
    assert_eq!(uri_to_path("file://localhost/a/b"), "/a/b");
    assert_eq!(uri_to_path("file://host/share/x"), "//host/share/x");
    assert_eq!(uri_to_path("file://C:/x"), "C:/x");
    assert_eq!(uri_to_path("/plain/path"), "/plain/path");
    assert_eq!(path_to_uri("/a b/c#1 50%.png"), "file:/a%20b/c%231%2050%25.png");
    assert_eq!(path_to_uri("C:\\x\\y.png"), "file:C:/x/y.png");
    assert_eq!(path_to_uri("https://example.com/a b"), "https://example.com/a b");
    for path in ["/a b/c#1 50%.png", "C:/x/y z.png"] {
        assert_eq!(uri_to_path(&path_to_uri(path)), path);
    }
}
