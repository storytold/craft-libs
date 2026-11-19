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
    assert_eq!(try_parse(&uses, &shallow()).err(), Some(Error::TooDeep(24)));
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
