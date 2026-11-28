//! Public validated-context contracts; private byte/raster accounting lives in tests.rs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
use super::super::{Limits, PREFLIGHT_PASSES, preflight, usvg};
use super::ValidatedImageResolver;
use std::io::Write;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const NS: &str = "http://www.w3.org/2000/svg";

fn drawing(contents: &str) -> String {
    format!("<svg xmlns='{NS}' width='4' height='4'>{contents}</svg>")
}

fn url(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut value = String::from("data:image/svg+xml,");
    for byte in bytes {
        write!(&mut value, "%{byte:02X}").unwrap();
    }
    value
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn image(href: &str) -> String {
    format!("<image width='4' height='4' href='{href}'/>")
}

fn parse_count(text: &str, limits: Limits) -> (super::super::ImageUsage, usize) {
    PREFLIGHT_PASSES.with(|count| count.set(0));
    let context = ValidatedImageResolver::new(limits);
    let checked = preflight(text, &limits).unwrap();
    let tree = context.parse_preflight(checked, usvg::Options::default()).unwrap();
    assert!(!tree.root().children().is_empty(), "the visible drawing survives validation");
    (context.usage(), PREFLIGHT_PASSES.with(|count| count.get()))
}

#[test]
fn malformed_plain_child_leaves_work_for_valid_siblings() {
    let child = drawing("<rect width='4' height='4' fill='red'/>");
    for invalid in [&[0xff][..], &[0xff, 0xfe, 0xd8][..]] {
        let contents = format!("{}{}", image(&url(invalid)), image(&url(child.as_bytes())));
        let root = drawing(&contents);
        let limits = Limits { embedded_bytes: invalid.len() + child.len(), ..Default::default() };
        let context = ValidatedImageResolver::new(limits);
        let tree = context.parse_preflight(preflight(&root, &limits).unwrap(), usvg::Options::default()).unwrap();
        let usage = context.usage();
        assert_eq!((usage.invalid_svg_images, usage.embedded_svg_count, usage.svg_limit_images), (1, 1, 0));
        assert_eq!(usage.embedded_svg_bytes, limits.embedded_bytes);
        let mut pixmap = super::super::draw::tiny_skia::Pixmap::new(4, 4).unwrap();
        super::super::draw::resvg::render(&tree, Default::default(), &mut pixmap.as_mut());
        assert_eq!(pixmap.pixel(2, 2).unwrap().red(), 255);
    }
}

#[test]
fn a_malformed_image_at_the_svg_byte_cap_does_not_drop_the_drawing() {
    let root = drawing("<rect width='4' height='4' fill='red'/><image width='4' height='4' href='data:;base64'/>");
    let limits = Limits { bytes: root.len(), ..Default::default() };
    let context = ValidatedImageResolver::new(limits);
    let tree = context.parse_preflight(preflight(&root, &limits).unwrap(), usvg::Options::default()).unwrap();
    assert_eq!(context.usage().invalid_raster_images, 1);
    let mut pixmap = super::super::draw::tiny_skia::Pixmap::new(4, 4).unwrap();
    super::super::draw::resvg::render(&tree, Default::default(), &mut pixmap.as_mut());
    assert_eq!(pixmap.pixel(2, 2).unwrap().red(), 255);
}

#[test]
fn validated_root_reuses_checked_xml_and_rechecks_only_changed_attributes() {
    let ordinary = drawing("<rect width='4' height='4' fill='red'/>");
    let repaired = drawing("<rect width='4' height='4' fill='red'/><image width='1' height='1' href='data:image/png;base64,A'/>");
    let (usage, count) = parse_count(&ordinary, Limits::default());
    assert_eq!(count, 1);
    assert_eq!(usage.invalid_raster_images, 0);
    let (usage, count) = parse_count(&repaired, Limits::default());
    assert_eq!(count, 2);
    assert_eq!(usage.invalid_raster_images, 1);
}

#[test]
fn validated_plain_and_gzip_children_reuse_their_original_preflight() {
    for repaired in [false, true] {
        let mut contents = String::from("<rect width='4' height='4' fill='red'/>");
        if repaired {
            contents.push_str("<image width='1' height='1' href='data:image/png;base64,A'/>");
        }
        let child = drawing(&contents);
        for compressed in [false, true] {
            let encoded = if compressed { gzip(child.as_bytes()) } else { child.as_bytes().to_vec() };
            let root = drawing(&image(&url(&encoded)));
            let (usage, count) = parse_count(&root, Limits::default());
            assert_eq!(count, if repaired { 3 } else { 2 }, "compressed={compressed}, repaired={repaired}");
            assert_eq!(usage.embedded_svg_count, 1);
            assert_eq!(usage.embedded_svg_bytes, child.len());
            assert_eq!(usage.invalid_raster_images, usize::from(repaired));
            assert_eq!(usage.invalid_svg_images, 0);
        }
    }
}

#[test]
fn validated_original_xml_guards_precede_normalization_and_conversion() {
    let normal = drawing("<image href='data:image/png;base64,A'/>");
    let entity = format!("<!DOCTYPE svg [<!ENTITY a 'data:image/png;base64,A'>]>{}", drawing("<image width='1' height='1' href='&a;'/>"));
    let markup = format!("<!DOCTYPE svg [<!ENTITY a '<g/>'>]>{}", drawing("&a;"));
    let referenced = drawing("<defs><g id='a'><g id='b'><rect/></g></g></defs><use href='#a'/>");
    for (text, limits) in [
        (&normal, Limits { bytes: normal.len() - 1, ..Default::default() }),
        (&normal, Limits { attributes: 0, ..Default::default() }),
        (&normal, Limits { xml_nodes: 1, ..Default::default() }),
        (&entity, Limits { entity_bytes: 1, ..Default::default() }),
        (&markup, Limits { refuse_entity_markup: true, ..Default::default() }),
        (&referenced, Limits { tree_depth: 2, ..Default::default() }),
    ] {
        assert!(preflight(text, &limits).is_err(), "original source must be refused before normalization: {text}");
    }
    let limits = Limits { entity_bytes: 1024, ..Default::default() };
    let context = ValidatedImageResolver::new(limits);
    let checked = preflight(&entity, &limits).unwrap();
    context.parse_preflight(checked, usvg::Options::default()).unwrap();
    assert_eq!(context.usage().invalid_raster_images, 1, "admitted entity data reaches omission accounting");
}

#[test]
fn validated_context_requires_exact_preflight_limits() {
    let text = drawing("<rect width='1' height='1'/>");
    let limits = Limits { bytes: 4096, tree_depth: 16, embedded_bytes: 2048, ..Default::default() };
    let context = ValidatedImageResolver::new(limits);
    assert_eq!(context.limits(), limits);
    for changed in [
        Limits { bytes: 4095, ..limits },
        Limits { bytes: 4097, ..limits },
        Limits { tree_depth: 15, ..limits },
        Limits { embedded_bytes: 2049, ..limits },
    ] {
        let checked = preflight(&text, &changed).unwrap();
        assert!(context.parse_preflight(checked, usvg::Options::default()).is_err());
        assert_eq!(context.usage(), Default::default(), "policy mismatch must not start conversion");
    }
    context.parse_preflight(preflight(&text, &limits).unwrap(), usvg::Options::default()).unwrap();
}

fn hostile_options(calls: &Arc<AtomicUsize>) -> usvg::Options<'static> {
    let string_calls = Arc::clone(calls);
    let data_calls = Arc::clone(calls);
    usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_string: Box::new(move |_, _| {
                string_calls.fetch_add(1, Ordering::SeqCst);
                panic!("ambient string resolver must not run")
            }),
            resolve_data: Box::new(move |_, _, _| {
                data_calls.fetch_add(1, Ordering::SeqCst);
                panic!("caller data resolver must not bypass validation")
            }),
        },
        ..Default::default()
    }
}

#[test]
fn validated_root_and_nested_context_override_untrusted_image_callbacks() {
    let calls = Arc::new(AtomicUsize::new(0));
    let child = drawing(
        "<rect width='1' height='1'/><image href='file:///never-read-by-validation.png' width='1' height='1'/><image href='data:image/png,%00' width='1' height='1'/>",
    );
    let root = drawing(&format!("<rect width='1' height='1'/>{}", image(&url(child.as_bytes()))));
    let limits = Limits::default();
    let context = ValidatedImageResolver::new(limits);
    context.parse_preflight(preflight(&root, &limits).unwrap(), hostile_options(&calls)).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(context.usage().embedded_svg_count, 1);
    assert_eq!(context.usage().invalid_raster_images, 1);
    let direct = ValidatedImageResolver::new(limits);
    assert!(matches!(direct.resolve_text(&child, hostile_options(&calls)).unwrap(), Some(usvg::ImageKind::SVG(_))));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(direct.usage().invalid_raster_images, 1);
}

#[test]
fn expanded_root_references_and_child_share_the_converted_depth_limit() {
    let child = drawing("<g><rect width='1' height='1'/></g>"); // height 3
    let href = url(child.as_bytes());
    let root = drawing(&format!("<defs><g id='a'><use href='#b'/></g><g id='b'>{}</g></defs><use href='#a'/>", image(&href))); // height 6
    assert!(preflight(&root, &Limits { tree_depth: 5, ..Default::default() }).is_err());
    assert!(preflight(&root, &Limits { tree_depth: 6, ..Default::default() }).is_ok());
    assert!(preflight(&child, &Limits { tree_depth: 3, ..Default::default() }).is_ok());
    // Expanded root height6 includes the callback leaf: reserve5 ancestors,
    // then admit the child's complete height3. Source depth alone would wrongly
    // accept the cap7 case, while reserving the leaf twice would reject cap8.
    for (depth, admitted) in [(7, false), (8, true)] {
        let limits = Limits { tree_depth: depth, ..Default::default() };
        let checked = preflight(&root, &limits).unwrap();
        assert!(checked.depth() < 6, "source nesting is shallower than expanded references");
        let context = ValidatedImageResolver::new(limits);
        context.parse_preflight(checked, usvg::Options::default()).unwrap();
        let usage = context.usage();
        assert_eq!(usage.embedded_svg_count, usize::from(admitted));
        assert_eq!(usage.svg_limit_images, usize::from(!admitted));
        assert_eq!(usage.invalid_svg_images, 0);
    }
}

#[test]
fn plain_root_callback_leaf_does_not_spend_the_child_boundary_twice() {
    let child = drawing("<g><rect width='4' height='4' fill='black'/></g>"); // height3
    let root = drawing(&image(&url(child.as_bytes()))); // height2, one ancestor
    for (depth, admitted) in [(3, false), (4, true)] {
        let limits = Limits { tree_depth: depth, ..Default::default() };
        assert!(preflight(&root, &limits).is_ok());
        assert!(preflight(&child, &limits).is_ok());
        let context = ValidatedImageResolver::new(limits);
        let tree = context.parse_preflight(preflight(&root, &limits).unwrap(), usvg::Options::default()).unwrap();
        assert_eq!(context.usage().embedded_svg_count, usize::from(admitted));
        assert_eq!(context.usage().svg_limit_images, usize::from(!admitted));
        assert_eq!(context.usage().invalid_svg_images, 0);
        let pixels = super::super::draw::render(&tree, usvg::Transform::identity(), 4, 4, 0, 1 << 20).unwrap();
        let sample = pixels.pixel(1, 1).unwrap();
        assert_eq!((sample.red(), sample.green(), sample.blue()), (0, 0, 0));
        assert_eq!(sample.alpha(), if admitted { 255 } else { 0 }, "exact-boundary child must actually draw");
    }
}

#[test]
fn nested_callback_leaves_do_not_spend_the_descendant_boundary_twice() {
    let leaf = drawing("<rect width='4' height='4' fill='black'/>"); // height2
    let middle = drawing(&image(&url(leaf.as_bytes()))); // height2
    let root = drawing(&image(&url(middle.as_bytes()))); // height2
    // Root and middle each contribute one ancestor, then the leaf's full
    // height2: the visible rectangle fits exactly at depth4, but not depth3.
    for (depth, admitted) in [(3, false), (4, true)] {
        let limits = Limits { tree_depth: depth, ..Default::default() };
        let context = ValidatedImageResolver::new(limits);
        let tree = context.parse_preflight(preflight(&root, &limits).unwrap(), usvg::Options::default()).unwrap();
        let usage = context.usage();
        assert_eq!(usage.embedded_svg_count, if admitted { 2 } else { 1 });
        assert_eq!(usage.svg_limit_images, usize::from(!admitted));
        assert_eq!(usage.invalid_svg_images, 0);
        let pixels = super::super::draw::render(&tree, usvg::Transform::identity(), 4, 4, 0, 1 << 20).unwrap();
        let sample = pixels.pixel(1, 1).unwrap();
        assert_eq!(sample.alpha(), if admitted { 255 } else { 0 }, "exact-boundary grandchild must actually draw");
    }
}
