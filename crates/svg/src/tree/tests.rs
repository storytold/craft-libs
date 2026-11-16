#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use super::*;

const DEPTH: usize = 256;

fn svg(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10">{body}</svg>"#)
}

/// `n` groups in one another around a rectangle.
fn groups(n: usize) -> String {
    format!("{}<rect width='1' height='1'/>{}", "<g>".repeat(n), "</g>".repeat(n))
}

fn deep(svg: &str) -> bool {
    height(&roxmltree::Document::parse(svg).unwrap(), DEPTH).unwrap().is_none()
}

fn options() -> usvg::Options<'static> {
    usvg::Options { image_href_resolver: image_resolver(Limits::default()), ..Default::default() }
}

/// A limit small enough for usvg to convert on a test thread's stack.
fn shallow() -> Limits {
    Limits { depth: 24, ..Limits::default() }
}

fn try_parse(svg: &str, limits: &Limits) -> Result<usvg::Tree, Error> {
    parse(svg.as_bytes(), limits, &options())
}

/// Every image in the tree, as whether it is an SVG document, nested ones after their parent.
fn images(group: &usvg::Group, out: &mut Vec<bool>) {
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => images(g, out),
            usvg::Node::Image(image) => match image.kind() {
                usvg::ImageKind::SVG(tree) => {
                    out.push(true);
                    images(tree.root(), out);
                }
                _ => out.push(false),
            },
            _ => {}
        }
    }
}

/// `<image>` showing the SVG text `inner` as a data URL.
fn embed(inner: &str) -> String {
    let encoded: String = inner.bytes().map(|b| if b.is_ascii_alphanumeric() { char::from(b).to_string() } else { format!("%{b:02X}") }).collect();
    format!(r#"<image width="5" height="5" xlink:href="data:image/svg+xml,{encoded}"/>"#)
}

#[test]
fn an_svg_and_an_svgz_parse_to_the_same_tree() {
    let text = svg("<g><rect width='4' height='2' fill='red'/></g>");
    let tree = try_parse(&text, &shallow()).unwrap();
    assert_eq!((tree.size().width(), tree.size().height()), (10.0, 10.0));
    assert_eq!(tree.root().children().len(), 1);
    let packed = parse(&svgz::compress(text.as_bytes()), &shallow(), &options()).unwrap();
    assert_eq!(packed.root().children().len(), 1);
    assert_eq!(super::text(text.as_bytes(), &shallow()).unwrap(), text);
    assert_eq!(parse_text(&text, &shallow(), &options()).unwrap().root().children().len(), 1);
}

#[test]
fn every_limit_refuses_the_file_past_it() {
    let text = svg(&groups(4));
    // Bytes, plain and unpacked.
    let small = Limits { bytes: text.len() - 1, ..shallow() };
    assert_eq!(try_parse(&text, &small).err(), Some(Error::Text(svgz::Error::TooLarge(text.len() - 1))));
    assert_eq!(parse(&svgz::compress(text.as_bytes()), &small, &options()).err(), Some(Error::Text(svgz::Error::TooLarge(text.len() - 1))));
    assert!(matches!(parse_text(&text, &small, &options()), Err(Error::Text(_))));
    assert!(try_parse(&text, &Limits { bytes: text.len(), ..shallow() }).is_ok());
    // Depth in the text: the root, four groups and the rectangle are six levels.
    assert!(try_parse(&text, &Limits { depth: 6, ..shallow() }).is_ok());
    assert_eq!(try_parse(&text, &Limits { depth: 5, ..shallow() }).err(), Some(Error::TooDeep(5)));
    // Depth by reference: no element is nested, each <use> holds the next.
    let chain: String = (0..20).map(|i| format!("<g id='g{i}'><use xlink:href='#g{}'/></g>", i + 1)).collect();
    let uses = svg(&format!("<defs>{chain}<rect id='g20' width='1' height='1'/></defs><use xlink:href='#g0'/>"));
    assert!(!guard::deeper_than(&uses, 24));
    let referenced_depth = Limits { tree_depth: 24, ..shallow() };
    assert_eq!(try_parse(&uses, &referenced_depth).err(), Some(Error::TooDeep(24)));
    // Attributes on one element.
    let wide = svg(&format!("<g{}/>", (0..40).map(|i| format!(" a{i}='1'")).collect::<String>()));
    assert!(try_parse(&wide, &Limits { attributes: 40, ..shallow() }).is_ok());
    assert_eq!(try_parse(&wide, &Limits { attributes: 39, ..shallow() }).err(), Some(Error::TooWide(39)));
    assert_eq!(try_parse(&wide, &Limits { attribute_work: 1000, ..shallow() }).err(), Some(Error::TooWide(4096)));
    // Entities.
    let bomb = format!("<!DOCTYPE svg [<!ENTITY a \"{}\">]>{}", "x".repeat(1000), svg(&format!("<title>{}</title>", "&a;".repeat(10))));
    assert_eq!(try_parse(&bomb, &Limits { entity_bytes: 9_999, ..shallow() }).err(), Some(Error::Entities));
    assert!(try_parse(&bomb, &Limits { entity_bytes: 10_000, ..shallow() }).is_ok());
    // XML nodes.
    assert!(matches!(try_parse(&svg(&"<g/>".repeat(100)), &Limits { xml_nodes: 50, ..shallow() }), Err(Error::Xml(_))));
    // Not XML, and XML that is not an SVG.
    assert!(matches!(try_parse("<svg", &shallow()), Err(Error::Xml(_))));
    assert!(matches!(try_parse("<html/>", &shallow()), Err(Error::Parse(_))));
    assert!(matches!(parse(&[0xfe, 0xff, 0xd8], &shallow(), &options()), Err(Error::Text(svgz::Error::Encoding))));
}

#[test]
fn an_entity_holding_markup_is_refused_only_when_asked() {
    let text = format!("<!DOCTYPE svg [<!ENTITY a \"<rect width='1' height='1'/>\">]>{}", svg("<g>&a;</g>"));
    assert_eq!(try_parse(&text, &Limits { refuse_entity_markup: true, ..shallow() }).err(), Some(Error::EntityMarkup));
    let tree = try_parse(&text, &shallow()).unwrap();
    assert_eq!(tree.root().children().len(), 1, "the entity's rectangle is drawn");
    // A namespace entity, as drawing apps write, is not markup.
    let names = "<!DOCTYPE svg [<!ENTITY ns \"http://www.w3.org/2000/svg\">]><svg xmlns=\"&ns;\" width=\"1\" height=\"1\"/>";
    assert!(try_parse(names, &Limits { refuse_entity_markup: true, ..shallow() }).is_ok());
}

#[test]
fn elements_nest_up_to_the_cap() {
    let (at, past) = (svg(&groups(DEPTH - 2)), svg(&groups(DEPTH - 1)));
    assert!(!deep(&at));
    assert!(deep(&past));
    let ten = svg(&groups(8));
    let xml = roxmltree::Document::parse(&ten).unwrap();
    assert_eq!((height(&xml, 10), height(&xml, 9)), (Ok(Some(10)), Ok(None)));
    assert_eq!(height(&roxmltree::Document::parse("<svg/>").unwrap(), 1), Ok(Some(1)));
}

#[test]
fn a_chain_of_uses_is_as_deep_as_what_it_copies() {
    let chain = |n: usize| {
        let defs: String = (0..n).map(|i| format!("<g id='g{i}'><use xlink:href='#g{}'/></g>", i + 1)).collect();
        svg(&format!("<defs>{defs}<rect id='g{n}' width='1' height='1'/></defs><use href='#g0'/>"))
    };
    assert!(!deep(&chain(10)));
    assert!(deep(&chain(DEPTH / 2)), "two elements a link");
    assert!(deep(&chain(5000)));
}

#[test]
fn chains_of_clips_masks_and_patterns_count_by_attribute_style_and_style_sheet() {
    let by_attribute = |n: usize| -> String {
        (0..n).map(|i| format!("<clipPath id='c{i}' clip-path='url(#c{})'><rect width='1' height='1'/></clipPath>", i + 1)).collect()
    };
    let by_style = |n: usize| -> String {
        (0..n).map(|i| format!("<mask id='c{i}' style='mask: url( \"#c{}\" )'><rect width='1' height='1'/></mask>", i + 1)).collect()
    };
    let by_sheet = |n: usize| -> String {
        let rules: String = (0..n).map(|i| format!(".p{i} {{ fill: url(#c{}) }}", i + 1)).collect();
        let patterns: String =
            (0..n).map(|i| format!("<pattern id='c{i}' width='1' height='1'><g class='p{i}'><rect width='1' height='1'/></g></pattern>")).collect();
        format!("<style>{rules}</style>{patterns}")
    };
    let by_marker =
        |n: usize| -> String { (0..n).map(|i| format!("<marker id='c{i}'><path d='M0 0L1 1' marker-end='url(#c{})'/></marker>", i + 1)).collect() };
    for chain in [&by_attribute as &dyn Fn(usize) -> String, &by_style, &by_sheet, &by_marker] {
        let file = |n| {
            svg(&format!(
                "<defs>{}</defs><rect width='5' height='5' fill='url(#c0)' clip-path='url(#c0)' mask='url(#c0)' marker-start='url(#c0)'/>",
                chain(n)
            ))
        };
        assert!(!deep(&file(5)));
        assert!(deep(&file(DEPTH)));
    }
}

#[test]
fn many_paint_servers_used_side_by_side_are_not_a_chain() {
    // As drawing apps write gradients: a class each, used by objects that aren't inside one another.
    let rules: String = (0..300).map(|i| format!(".c{i}{{fill:url(#g{i})}}")).collect();
    let defs: String = (0..300).map(|i| format!("<linearGradient id='g{i}'><stop class='c{i}' offset='0'/></linearGradient>")).collect();
    let art: String = (0..300).map(|i| format!("<g class='c{i}'><rect class='c{i}' width='1' height='1'/></g>")).collect();
    assert!(!deep(&svg(&format!("<style>{rules}</style><defs>{defs}</defs>{art}"))));
}

#[test]
fn a_reference_follows_the_first_element_of_its_id() {
    // A chain of `<use>`s as deep as the cap, each link's id given again later to an empty group:
    // usvg follows the first, so the chain is deep.
    let chain: String = (1..DEPTH).map(|i| format!("<g id='a{i}'><use href='#a{}'/></g>", i - 1)).collect();
    let shallow: String = (1..DEPTH).map(|i| format!("<g id='a{i}'/>")).collect();
    let top = format!("<use href='#a{}'/>", DEPTH - 1);
    assert!(deep(&svg(&format!("<defs><g id='a0'><rect width='1' height='1'/></g>{chain}{shallow}</defs>{top}"))));
}

#[test]
fn a_loop_of_references_is_not_deep() {
    assert!(!deep(&svg("<g id='a'><use href='#b'/></g><g id='b'><use href='#a'/></g><clipPath id='c' clip-path='url(#c)'/>")));
}

#[test]
fn an_inherited_paint_is_converted_under_everything_between() {
    // Neither the pattern nor the art is too deep; the pattern under the art's last object is.
    let half = DEPTH / 2;
    let pattern = format!("<pattern id='p' width='1' height='1'>{}</pattern>", groups(half));
    assert!(!deep(&svg(&format!("<defs>{pattern}</defs><g fill='url(#p)'>{}</g>", groups(half / 2)))));
    assert!(deep(&svg(&format!("<defs>{pattern}</defs><g fill='url(#p)'>{}</g>", groups(half)))));
}

#[test]
fn a_style_sheet_that_names_elements_and_cannot_be_read_is_refused() {
    let sheet = |css: &str, media: &str| {
        svg(&format!("<style{media}>{css}</style><pattern id='p' width='1' height='1'/><rect class='a' width='1' height='1'/>"))
    };
    let check = |text: &str| height(&roxmltree::Document::parse(text).unwrap(), DEPTH);
    // The root, the rectangle, and under it the pattern its fill names.
    assert_eq!(check(&sheet(".a{fill:url(#p)}", "")), Ok(Some(3)));
    // An at-rule block, a sheet for another medium and a broken rule: read here they would add
    // nothing, read by usvg they might.
    assert!(matches!(check(&sheet("@media print{.a{fill:url(#p)}}", "")), Err(Error::Stylesheet(_))));
    assert!(matches!(check(&sheet(".a{fill:url(#p)}", " media='print'")), Err(Error::Stylesheet(_))));
    assert!(matches!(check(&sheet(".a{fill:url(#p)", "")), Err(Error::Stylesheet(_))));
    assert!(matches!(try_parse(&sheet("@media print{.a{fill:url(#p)}}", ""), &shallow()), Err(Error::Stylesheet(_))));
    // The same sheets naming no element add no nesting and are usvg's to read.
    assert_eq!(check(&sheet("@media print{.a{fill:red}}", "")), Ok(Some(2)));
    assert_eq!(check(&sheet(".a{fill:red", "")), Ok(Some(2)));
    assert!(try_parse(&sheet(".a{fill:red}", " media='print'"), &shallow()).is_ok());
}

#[test]
fn embedded_svgs_are_read_within_their_budgets() {
    let inner = svg("<rect width='1' height='1'/>");
    let found = |text: &str, limits: Limits| {
        let options = usvg::Options { image_href_resolver: image_resolver(limits), ..Default::default() };
        let mut out = vec![];
        images(parse(text.as_bytes(), &limits, &options).unwrap().root(), &mut out);
        out
    };
    let one = svg(&embed(&inner));
    assert_eq!(found(&one, shallow()), [true]);
    // Bytes: one fits, a second does not; what a refused one would have used is spent.
    let two = svg(&format!("{}{}", embed(&inner), embed(&inner)));
    assert_eq!(found(&two, shallow()), [true, true]);
    assert_eq!(found(&two, Limits { embedded_bytes: inner.len() * 2 - 1, ..shallow() }), [true]);
    assert_eq!(found(&one, Limits { embedded_bytes: inner.len() - 1, ..shallow() }), [false; 0]);
    // Depth: a document in a document in a document.
    let three = svg(&embed(&svg(&embed(&inner))));
    assert_eq!(found(&three, shallow()), [true, true]);
    assert_eq!(found(&three, Limits { embedded_depth: 1, ..shallow() }), [true]);
    assert_eq!(found(&three, Limits { embedded_depth: 0, ..shallow() }), [false; 0]);
    // An embedded document goes through the same checks: too deep, it is left out.
    let nested = svg(&embed(&svg(&groups(30))));
    assert_eq!(found(&nested, shallow()), [false; 0]);
    let wide = svg(&embed(&svg(&format!("<g{}/>", (0..40).map(|i| format!(" a{i}='1'")).collect::<String>()))));
    assert_eq!(found(&wide, shallow()), [true]);
    assert_eq!(found(&wide, Limits { attributes: 39, ..shallow() }), [false; 0]);
    // A linked file is never read.
    assert_eq!(found(&svg(r#"<image width="5" height="5" xlink:href="/etc/passwd.svg"/>"#), shallow()), [false; 0]);
}

mod resource_limit_usage {
    use super::*;

    fn data_image(bytes: &[u8]) -> String {
        let encoded: String = bytes.iter().map(|byte| format!("%{byte:02X}")).collect();
        format!(r#"<image width="5" height="5" xlink:href="data:image/svg+xml,{encoded}"/>"#)
    }

    fn packed(text: &str, gzip: bool) -> Vec<u8> {
        if gzip { svgz::compress(text.as_bytes()) } else { text.as_bytes().to_vec() }
    }

    /// The outer document has its own generous text allowance, so tiny resolver limits
    /// test the public embedded admission contract rather than rejecting the outer XML.
    fn resolved(text: &str, limits: Limits, outer_depth: usize) -> (Vec<bool>, ImageUsage) {
        let resolver = ImageResolver::new(limits);
        resolver.set_document_depth(outer_depth);
        let options = usvg::Options { image_href_resolver: resolver.resolver(), ..Default::default() };
        let tree = parse(text.as_bytes(), &shallow(), &options).unwrap();
        let mut found = Vec::new();
        images(tree.root(), &mut found);
        (found, resolver.usage())
    }

    fn expanding_encoded_children() -> Vec<(&'static str, Vec<u8>)> {
        let latin =
            format!("<?xml version='1.0' encoding='ISO-8859-1'?>{}", svg(&format!("<title>{}</title><rect width='1' height='1'/>", "é".repeat(100))));
        let latin = latin.chars().map(|c| u8::try_from(u32::from(c)).unwrap()).collect();
        let wide = svg(&format!("<title>{}</title><rect width='1' height='1'/>", "ࠀ".repeat(400)));
        let mut little = vec![0xff, 0xfe];
        little.extend(wide.encode_utf16().flat_map(u16::to_le_bytes));
        let mut big = vec![0xfe, 0xff];
        big.extend(wide.encode_utf16().flat_map(u16::to_be_bytes));
        vec![("Latin-1", latin), ("UTF-16LE", little), ("UTF-16BE", big)]
    }

    #[test]
    fn decoded_text_expansion_is_a_resource_refusal_for_plain_and_gzip_children() {
        for (encoding, raw) in expanding_encoded_children() {
            let decoded = svgz::decode(&raw).unwrap();
            let cap = raw.len();
            assert!(decoded.len() > cap, "{encoding} fixture must expand after inflation");
            for gzip in [false, true] {
                let data = if gzip { svgz::compress(&raw) } else { raw.clone() };
                assert_eq!(svgz::inflate(&data, cap).unwrap().len(), cap, "raw bytes fit exactly");
                assert_eq!(svgz::text(&data, cap).unwrap().len(), decoded.len(), "decoding itself succeeds");
                let limits = Limits { bytes: cap, embedded_bytes: decoded.len(), ..shallow() };
                assert!(matches!(preflight(&decoded, &limits), Err(Error::Text(svgz::Error::TooLarge(n))) if n == cap));
                let image = data_image(&data);
                let (found, usage) = resolved(&svg(&image), limits, 1);
                assert!(found.is_empty(), "{encoding}, gzip {gzip}");
                assert_eq!((usage.svg_limit_images, usage.svg_resource_limit_images, usage.invalid_svg_images), (1, 1, 0));
                assert_eq!(usage.embedded_svg_bytes, decoded.len(), "spent decoded work remains charged");
                let (found, usage) = resolved(&svg(&format!("{image}{image}")), limits, 1);
                assert!(found.is_empty());
                assert_eq!((usage.svg_limit_images, usage.svg_resource_limit_images, usage.invalid_svg_images), (2, 2, 0));
                assert_eq!(usage.embedded_svg_bytes, decoded.len(), "a refused sibling cannot repeat exhausted work");
            }
        }
    }

    #[test]
    fn decoded_text_expansion_respects_aggregate_bytes_even_when_each_child_fits() {
        for (encoding, raw) in expanding_encoded_children() {
            let decoded = svgz::decode(&raw).unwrap();
            let limits = Limits { bytes: decoded.len(), embedded_bytes: raw.len(), ..shallow() };
            assert!(decoded.len() > limits.embedded_bytes);
            preflight(&decoded, &limits).unwrap(); // The independent per-image cap allows this complete XML.
            for gzip in [false, true] {
                let data = if gzip { svgz::compress(&raw) } else { raw.clone() };
                assert_eq!(svgz::inflate(&data, limits.embedded_bytes).unwrap().len(), raw.len());
                let image = data_image(&data);
                let (found, usage) = resolved(&svg(&image), limits, 1);
                assert!(found.is_empty(), "{encoding}, gzip {gzip}");
                assert_eq!((usage.svg_limit_images, usage.svg_resource_limit_images, usage.invalid_svg_images), (1, 1, 0));
                assert_eq!(usage.embedded_svg_bytes, limits.embedded_bytes, "refusal consumes exactly the remaining allowance");
                let (found, usage) = resolved(&svg(&format!("{image}{image}")), limits, 1);
                assert!(found.is_empty());
                assert_eq!((usage.svg_limit_images, usage.svg_resource_limit_images, usage.invalid_svg_images), (2, 2, 0));
                assert_eq!(usage.embedded_svg_bytes, limits.embedded_bytes, "a later child cannot repeat exhausted work");
                let (found, usage) = resolved(&svg(&image), Limits { embedded_bytes: decoded.len(), ..limits }, 1);
                assert_eq!(found, [true]);
                assert_eq!((usage.svg_limit_images, usage.svg_resource_limit_images, usage.invalid_svg_images), (0, 0, 0));
                assert_eq!(usage.embedded_svg_bytes, decoded.len());
            }
        }
    }

    #[test]
    fn decoded_text_expansion_at_the_cap_is_valid_and_malformed_xml_stays_invalid() {
        for (encoding, raw) in expanding_encoded_children() {
            let decoded = svgz::decode(&raw).unwrap();
            for gzip in [false, true] {
                let data = if gzip { svgz::compress(&raw) } else { raw.clone() };
                let limits = Limits { bytes: decoded.len(), embedded_bytes: decoded.len(), ..shallow() };
                let (found, usage) = resolved(&svg(&data_image(&data)), limits, 1);
                assert_eq!(found, [true], "{encoding}, gzip {gzip}");
                assert_eq!((usage.svg_limit_images, usage.svg_resource_limit_images, usage.invalid_svg_images), (0, 0, 0));
                assert_eq!(usage.embedded_svg_bytes, decoded.len());
                // Preserve the declared/BOM encoding when making the XML malformed;
                // re-encoding Latin-1 text as UTF-8 would test a second expansion instead.
                let closing_bytes = if encoding == "Latin-1" { 6 } else { 12 };
                let malformed = &raw[..raw.len() - closing_bytes];
                let data = if gzip { svgz::compress(malformed) } else { malformed.to_vec() };
                let (found, usage) = resolved(&svg(&data_image(&data)), limits, 1);
                assert!(found.is_empty());
                assert_eq!((usage.svg_limit_images, usage.svg_resource_limit_images, usage.invalid_svg_images), (0, 0, 1));
            }
        }
    }

    #[test]
    fn embedded_chain_caps_are_explicit_resource_refusals_for_plain_and_gzip_children() {
        for gzip in [false, true] {
            let inner = svg("<rect width='1' height='1'/>");
            let middle = svg(&data_image(&packed(&inner, gzip)));
            let outer = svg(&data_image(&packed(&middle, gzip)));
            let (found, usage) = resolved(&outer, Limits { embedded_depth: 1, ..shallow() }, 1);
            assert_eq!(found, [true]);
            assert_eq!(usage.embedded_svg_depth, 1);
            assert_eq!(usage.svg_limit_images, 1, "strict consumers still see the existing limit flag");
            assert_eq!(usage.svg_resource_limit_images, 1, "a chain cap must not become a successful partial bounded preview");
            let (found, usage) = resolved(&outer, Limits { embedded_depth: 2, ..shallow() }, 1);
            assert_eq!(found, [true, true]);
            assert_eq!((usage.svg_limit_images, usage.svg_resource_limit_images), (0, 0));
        }
    }

    #[test]
    fn embedded_per_image_byte_caps_are_resource_refusals_for_plain_and_gzip_children() {
        let inner = svg("<rect width='1' height='1'/>");
        for gzip in [false, true] {
            let outer = svg(&data_image(&packed(&inner, gzip)));
            let (_, usage) = resolved(&outer, Limits { bytes: inner.len() - 1, embedded_bytes: inner.len() * 3, ..shallow() }, 1);
            assert_eq!(usage.embedded_svg_count, 0);
            assert_eq!((usage.svg_limit_images, usage.svg_resource_limit_images), (1, 1));
            let (found, usage) = resolved(&outer, Limits { bytes: inner.len(), ..shallow() }, 1);
            assert_eq!(found, [true]);
            assert_eq!((usage.svg_limit_images, usage.svg_resource_limit_images), (0, 0));
        }
    }

    #[test]
    fn aggregate_embedded_bytes_are_resource_refusals_even_when_each_child_fits() {
        let inner = svg("<rect width='1' height='1'/>");
        for gzip in [false, true] {
            let image = data_image(&packed(&inner, gzip));
            let outer = svg(&format!("{image}{image}"));
            let cap = inner.len() * 2 - 1;
            let (found, usage) = resolved(&outer, Limits { embedded_bytes: cap, ..shallow() }, 1);
            assert_eq!(found, [true]);
            assert_eq!(usage.embedded_svg_bytes, cap, "a refused child still spends the remaining work allowance");
            assert_eq!((usage.svg_limit_images, usage.svg_resource_limit_images), (1, 1));
            let (found, usage) = resolved(&outer, Limits { embedded_bytes: inner.len() * 2, ..shallow() }, 1);
            assert_eq!(found, [true, true]);
            assert_eq!((usage.svg_limit_images, usage.svg_resource_limit_images), (0, 0));
        }
    }

    #[test]
    fn combined_xml_depth_omissions_remain_distinct_from_resource_refusals() {
        let inner = svg("<rect width='1' height='1'/>");
        for gzip in [false, true] {
            let image = data_image(&packed(&inner, gzip));
            let outer = svg(&format!("{}{}{}", "<g>".repeat(4), image, "</g>".repeat(4)));
            let (found, usage) = resolved(&outer, Limits { tree_depth: 6, ..shallow() }, 5);
            assert!(found.is_empty());
            assert_eq!(usage.svg_limit_images, 1, "strict consumers must continue to reject this omission");
            assert_eq!(usage.svg_resource_limit_images, 0, "non-strict consumers may omit this child without disguising a byte/chain overrun");
            let (found, usage) = resolved(&svg(&image), Limits { tree_depth: 6, ..shallow() }, 1);
            assert_eq!(found, [true]);
            assert_eq!((usage.svg_limit_images, usage.svg_resource_limit_images), (0, 0));
        }
    }

    #[test]
    fn invalid_and_corrupt_children_do_not_claim_a_resource_limit() {
        let mut corrupt = svgz::compress(svg("<rect width='1' height='1'/>").as_bytes());
        let crc = corrupt.len() - 8;
        corrupt[crc] ^= 1;
        for data in [b"not XML".to_vec(), corrupt] {
            let (found, usage) = resolved(&svg(&data_image(&data)), shallow(), 1);
            assert!(found.is_empty());
            assert!(usage.invalid_svg_images > 0 || usage.svg_limit_images > 0, "strict consumers still observe unreadable content");
            assert_eq!(usage.svg_resource_limit_images, 0);
        }
    }
}

mod draw_checks {
    use super::*;

    const GIB: u64 = 1 << 30;

    fn check(body: &str, budget: u64) -> Result<(), draw::Refusal> {
        let tree = try_parse(&svg(body), &Limits { depth: 40, ..Limits::default() }).unwrap();
        draw::check(tree.root().children(), usvg::Transform::identity(), 100, 100, 0, budget)
    }

    #[test]
    fn an_ordinary_drawing_fits() {
        let body = "<defs><clipPath id='c'><rect width='5' height='5'/></clipPath>
            <filter id='f'><feGaussianBlur stdDeviation='1'/></filter>
            <pattern id='p' width='2' height='2' patternUnits='userSpaceOnUse'><rect width='1' height='1'/></pattern></defs>
            <g opacity='0.5' clip-path='url(#c)' filter='url(#f)'><rect width='8' height='8' fill='url(#p)'/></g>";
        assert_eq!(check(body, GIB), Ok(()));
        // The target alone is 100 × 100 × 4 bytes.
        assert!(check("<rect width='1' height='1'/>", 40_000).is_ok());
        assert!(check("<rect width='1' height='1'/>", 39_999).is_ok(), "nothing is charged where nothing draws on a layer");
    }

    #[test]
    fn a_filter_region_the_file_sets_is_charged_whatever_the_target() {
        let body = "<filter id='f' filterUnits='userSpaceOnUse' x='0' y='0' width='40000' height='40000'><feGaussianBlur stdDeviation='1'/></filter>
            <rect width='8' height='8' filter='url(#f)'/>";
        let refusal = check(body, GIB).unwrap_err();
        assert!(refusal.memory && !refusal.complex && refusal.message.contains("filter"), "{refusal:?}");
    }

    #[test]
    fn a_pattern_tile_is_charged_at_its_own_size() {
        let body = "<pattern id='p' width='40000' height='40000' patternUnits='userSpaceOnUse'><rect width='1' height='1'/></pattern>
            <rect width='8' height='8' fill='url(#p)'/>";
        let refusal = check(body, GIB).unwrap_err();
        assert!(refusal.memory && refusal.message.contains("pattern tile"), "{refusal:?}");
        let small = "<pattern id='p' width='40' height='40' patternUnits='userSpaceOnUse'><rect width='1' height='1'/></pattern>
            <rect width='8' height='8' fill='url(#p)'/>";
        assert_eq!(check(small, GIB), Ok(()));
    }

    #[test]
    fn counts_are_capped_apart_from_memory() {
        let primitives =
            |n: usize| format!("<filter id='f'>{}</filter><rect width='8' height='8' filter='url(#f)'/>", "<feOffset dx='1'/>".repeat(n));
        assert_eq!(check(&primitives(64), GIB), Ok(()));
        let refusal = check(&primitives(65), GIB).unwrap_err();
        assert!(!refusal.memory && refusal.message.contains("primitives"), "{refusal:?}");
        let clips = |n: usize| {
            let chain: String =
                (0..n).map(|i| format!("<clipPath id='c{i}' clip-path='url(#c{})'><rect width='5' height='5'/></clipPath>", i + 1)).collect();
            format!("{chain}<clipPath id='c{n}'><rect width='5' height='5'/></clipPath><rect width='8' height='8' clip-path='url(#c0)'/>")
        };
        assert_eq!(check(&clips(15), GIB), Ok(()));
        let refusal = check(&clips(16), GIB).unwrap_err();
        assert!(!refusal.memory && refusal.message.contains("clip-paths"), "{refusal:?}");
    }
}

mod invalid_image_accounting {
    use super::*;

    fn usage(body: &str, limits: Limits) -> (Vec<bool>, ImageUsage) {
        let text = svg(body);
        let resolver = ImageResolver::new(limits);
        resolver.check_document(&text).unwrap();
        let options = usvg::Options { image_href_resolver: resolver.resolver(), ..Default::default() };
        let tree = parse_text(&text, &limits, &options).unwrap();
        let mut found = Vec::new();
        images(tree.root(), &mut found);
        (found, resolver.usage())
    }

    fn png() -> Vec<u8> {
        use craft_image_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType, encode};
        let image = Image::new(1, 1, ChannelLayout::Rgba, SampleType::U8).unwrap();
        encode(&image, Format::Png, &EncodeOptions::default()).unwrap()
    }

    fn image(data: &[u8]) -> String {
        let encoded: String = data.iter().map(|b| format!("%{b:02X}")).collect();
        format!(r#"<image width="1" height="1" xlink:href="data:image/png,{encoded}"/>"#)
    }

    #[test]
    fn malformed_base64_is_counted_before_usvg_discards_it() {
        let bad = r#"<image width="1" height="1" xlink:href="data:image/png;base64,A"/>"#;
        let (found, report) = usage(bad, shallow());
        assert!(found.is_empty());
        assert_eq!(report.invalid_raster_images, 1);
        let (found, report) = usage(&embed(&svg(bad)), shallow());
        assert_eq!(found, [true]);
        assert_eq!(report.invalid_raster_images, 1);
    }

    #[test]
    fn malformed_data_url_without_comma_is_counted() {
        for href in ["data:image/png;base64", " data:image/png;base64", "da&#x9;ta:image/png;base64", "da&#xA;ta:image/png;base64"] {
            let bad = format!(r#"<image width="1" height="1" xlink:href="{href}"/>"#);
            let (found, report) = usage(&bad, shallow());
            assert!(found.is_empty(), "{href}");
            assert_eq!(report.invalid_raster_images, 1, "{href}");
            let (found, report) = usage(&embed(&svg(&bad)), shallow());
            assert_eq!(found, [true], "{href}");
            assert_eq!(report.invalid_raster_images, 1, "{href}");
        }
    }

    #[test]
    fn failed_decode_forgets_address_but_preserves_spent_budget() {
        let valid = png();
        let idat = valid.windows(4).position(|b| b == b"IDAT").unwrap();
        let invalid = Arc::new(valid[..idat + 4].to_vec());
        let pointer = Arc::as_ptr(&invalid) as usize;
        let limits = Limits { embedded_raster_pixels: 3, ..shallow() };
        let resolver = ImageResolver::new(limits);
        let used = &resolver.used;
        let format = craft_image_codecs::Format::Png;
        assert!(raster_image(used, &limits, invalid, format).is_none());
        assert!(!used.raster_data.lock().unwrap().contains(&pointer));
        assert_eq!(resolver.usage().raster_pixels, 1);
        let first_data = Arc::new(valid.clone());
        let first = raster_image(used, &limits, first_data.clone(), format).unwrap();
        assert_eq!(resolver.usage().raster_pixels, 2);
        let repeated = raster_image(used, &limits, first_data, format).unwrap();
        assert_eq!(resolver.usage().raster_pixels, 2);
        let second = raster_image(used, &limits, Arc::new(valid.clone()), format).unwrap();
        assert_eq!(resolver.usage().raster_pixels, 3);
        assert!(raster_image(used, &limits, Arc::new(valid), format).is_none());
        assert_eq!(resolver.usage().raster_limit_images, 1);
        drop((first, repeated, second));
    }

    #[test]
    fn corrupt_pixels_do_not_pass_a_valid_header_probe() {
        let valid = png();
        let (found, report) = usage(&image(&valid), shallow());
        assert_eq!(found, [false]);
        assert_eq!(report.invalid_raster_images, 0);
        let idat = valid.windows(4).position(|b| b == b"IDAT").unwrap();
        let corrupt = &valid[..idat + 4];
        assert_eq!(craft_image_codecs::probe_allocation_size(corrupt).unwrap(), (1, 1));
        let (found, report) = usage(&image(corrupt), shallow());
        assert!(found.is_empty());
        assert_eq!(report.invalid_raster_images, 1);
        let (found, report) = usage(&image(&valid), Limits { embedded_raster_pixels: 0, ..shallow() });
        assert!(found.is_empty());
        assert_eq!(report.raster_limit_images, 1);
    }
}

#[test]
fn preflight_keeps_source_depth_and_parses_the_checked_tree() {
    let text = svg("<g><rect width='1' height='1'/></g>");
    let checked = preflight(&text, &Limits::default()).unwrap();
    // svg and g are open; the self-closing rect opens no level.
    assert_eq!(checked.depth(), 2);
    let tree = checked.parse(&options()).unwrap();
    assert_eq!(tree.size().width(), 10.0);
    assert!(matches!(preflight(&svg(&groups(30)), &shallow()), Err(Error::TooDeep(24))));
}

#[test]
fn preflight_matches_parse_text_refusals() {
    for text in ["<svg", "<svg xmlns='http://www.w3.org/2000/svg'><g></svg>"] {
        assert!(matches!(preflight(text, &Limits::default()), Err(Error::Xml(_))));
        assert!(matches!(parse_text(text, &Limits::default(), &options()), Err(Error::Xml(_))));
    }
}

#[test]
fn rewrite_root_viewport_wraps_the_root_and_keeps_the_preamble() {
    let source = concat!(
        "<?xml version='1.0'?>\n<!DOCTYPE svg [<!ENTITY w '4'>]>\n",
        "<svg xmlns='http://www.w3.org/2000/svg' width='20' height='10' viewBox='0 0 2 1'>",
        "<rect width='&w;' height='1'/></svg>"
    );
    let out = rewrite_root_viewport(source, Viewport { width: 40.0, height: 30.0 }, &Limits::default(), 4096).unwrap();
    assert!(out.starts_with("<?xml version='1.0'?>\n<!DOCTYPE svg [<!ENTITY w '4'>]>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"30\" viewBox=\"0 0 20 10\" preserveAspectRatio=\"none\"><svg "));
    assert!(out.ends_with("</svg></svg>"));
    let tree = parse_text(&out, &Limits::default(), &options()).unwrap();
    assert_eq!((tree.size().width(), tree.size().height()), (40.0, 30.0));
}

#[test]
fn rewrite_root_viewport_reads_points_and_pixels() {
    let points = rewrite_root_viewport(
        "<svg xmlns='http://www.w3.org/2000/svg' width='72pt' height='36pt'/>",
        Viewport { width: 1.0, height: 1.0 },
        &Limits::default(),
        4096,
    )
    .unwrap();
    assert!(points.contains("viewBox=\"0 0 96 48\""));
    let pixels = rewrite_root_viewport(
        "<svg xmlns='http://www.w3.org/2000/svg' width='12px' height='8'/>",
        Viewport { width: 1.0, height: 1.0 },
        &Limits::default(),
        4096,
    )
    .unwrap();
    assert!(pixels.contains("viewBox=\"0 0 12 8\""));
}

#[test]
fn rewrite_root_viewport_refusals() {
    let limits = Limits::default();
    let one = Viewport { width: 1.0, height: 1.0 };
    let sized = "<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'/>";
    for target in [
        Viewport { width: f64::NAN, height: 1.0 },
        Viewport { width: 1.0, height: f64::INFINITY },
        Viewport { width: 0.0, height: 1.0 },
        Viewport { width: 1.0, height: -2.0 },
    ] {
        assert!(matches!(rewrite_root_viewport(sized, target, &limits, 4096), Err(Error::Viewport(_))));
    }
    assert!(matches!(rewrite_root_viewport("<html width='10' height='10'/>", one, &limits, 4096), Err(Error::Viewport(_))));
    for source in [
        "<svg xmlns='http://www.w3.org/2000/svg'/>",
        "<svg xmlns='http://www.w3.org/2000/svg' width='50%' height='10'/>",
        "<svg xmlns='http://www.w3.org/2000/svg' width='10mm' height='10'/>",
        "<svg xmlns='http://www.w3.org/2000/svg' width='0' height='10'/>",
        "<svg xmlns='http://www.w3.org/2000/svg' width='1e400' height='10'/>",
    ] {
        assert!(matches!(rewrite_root_viewport(source, one, &limits, 4096), Err(Error::Viewport(_))), "{source}");
    }
    assert!(matches!(rewrite_root_viewport("<svg", one, &limits, 4096), Err(Error::Xml(_))));
    let fits = rewrite_root_viewport(sized, one, &limits, 4096).unwrap();
    assert_eq!(rewrite_root_viewport(sized, one, &limits, fits.len()).unwrap(), fits);
    assert!(matches!(rewrite_root_viewport(sized, one, &limits, fits.len() - 1), Err(Error::OutputTooLarge(n)) if n == fits.len() - 1));
}

#[test]
fn rewrite_root_viewport_output_stays_within_the_limits() {
    let limits = shallow();
    let one = Viewport { width: 1.0, height: 1.0 };
    let mut refused_at_the_limit = false;
    for n in 0..limits.depth {
        let source = svg(&groups(n));
        if preflight(&source, &limits).is_err() {
            break;
        }
        match rewrite_root_viewport(&source, one, &limits, 1 << 20) {
            Ok(out) => assert!(parse_text(&out, &limits, &options()).is_ok(), "{n} groups"),
            Err(Error::TooDeep(_)) => refused_at_the_limit = true,
            Err(e) => panic!("{n} groups: {e:?}"),
        }
    }
    assert!(refused_at_the_limit, "a source at the depth limit is refused, not wrapped past it");
}
