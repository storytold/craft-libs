//! Caption buttons: untrusted icon files are refused.

use craft_ui::shell::{CaptionIcon, CaptionIcons};
use craft_ui::testing::trial;
use std::sync::Arc;

const GLYPH_PX: u32 = 16;

const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><path d="M4 4h8v8H4z" fill="#2e3436"/></svg>"##;

/// The minimize glyph's mask of `svg` at `px`, through the theme-file path the shell uses.
fn mask_of(svg: &[u8], px: u32) -> Option<craft_ui::shell::CaptionMask> {
    let files = [Some(Arc::<[u8]>::from(svg)), None, None, None];
    CaptionIcons::from_files("T", 0, files).mask(CaptionIcon::Minimize, px)
}

/// Icon files are untrusted: gzip is not expanded, a DTD or an oversized document is refused, and an `<image>` loads
/// nothing.
#[trial]
fn hostile_svgs_render_nothing() {
    let gzip = [&[0x1f, 0x8b, 0x08, 0, 0, 0, 0, 0, 0, 0xff][..], SQUARE.as_bytes()].concat();
    let dtd = r##"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY a "x">]><svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><path d="M4 4h8v8H4z"/></svg>"##;
    let many = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16">{}</svg>"#, "<g/>".repeat(20_000));
    let image = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><image href="/dev/zero" width="16" height="16"/></svg>"##;
    for hostile in [&gzip[..], dtd.as_bytes(), many.as_bytes(), b"not svg"] {
        assert!(mask_of(hostile, GLYPH_PX).is_none());
    }
    assert!(mask_of(SQUARE.as_bytes(), 0).is_none() && mask_of(SQUARE.as_bytes(), 100_000).is_none());
    assert!(mask_of(image.as_bytes(), GLYPH_PX).is_some_and(|mask| mask.iter().all(|coverage| *coverage == 0)), "nothing drawn");
}

/// Icons parse on the UI thread: a document nested deeper than its stack takes is refused, not a stack overflow.
#[trial]
fn an_icon_nested_past_its_depth_is_refused_not_a_crash() {
    let deep = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16">{}<path d="M4 4h8v8H4z"/>{}</svg>"#,
        "<g>".repeat(200),
        "</g>".repeat(200)
    );
    assert!(mask_of(deep.as_bytes(), GLYPH_PX).is_none());
    let shallow = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16">{}<path d="M4 4h8v8H4z"/>{}</svg>"#,
        "<g>".repeat(8),
        "</g>".repeat(8)
    );
    assert!(mask_of(shallow.as_bytes(), GLYPH_PX).is_some_and(|mask| mask.iter().any(|coverage| *coverage > 0)), "a shallow icon draws");
}

/// A glyph is a square coverage mask of the drawing, fitted to the asked side; the glyphs a theme lacks have none.
#[trial]
fn masks_are_square_coverage() {
    let mask = mask_of(SQUARE.as_bytes(), 32).expect("renders");
    assert_eq!(mask.len(), 32 * 32);
    assert_eq!(mask[16 * 32 + 16], 255, "inside the square");
    assert_eq!(mask[0], 0, "outside it");
    let files = [Some(Arc::<[u8]>::from(SQUARE.as_bytes())), None, None, None];
    assert!(CaptionIcons::from_files("T", 0, files).mask(CaptionIcon::Close, 32).is_none(), "the theme has no close glyph");
}

/// resvg panics rendering an offset past the pixmap's size: the icon is unreadable instead.
#[trial]
fn a_mask_the_renderer_panics_on_is_none() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16">
        <filter id="f"><feOffset dx="3e9" dy="-3e9"/></filter><rect width="16" height="16" filter="url(#f)"/>
    </svg>"##;
    assert!(mask_of(svg.as_bytes(), GLYPH_PX).is_none());
}

/// An icon's `<image>` is never loaded: not a device file, not an embedded picture.
#[trial]
fn masks_load_no_images() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="16" height="16">
        <image href="/dev/zero" width="16" height="16"/>
        <image xlink:href="data:image/svg+xml;utf8,&lt;svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'&gt;&lt;rect width='16' height='16'/&gt;&lt;/svg&gt;" width="16" height="16"/>
    </svg>"##;
    let mask = mask_of(svg.as_bytes(), GLYPH_PX).expect("renders");
    assert!(mask.iter().all(|coverage| *coverage == 0), "nothing drawn");
}
