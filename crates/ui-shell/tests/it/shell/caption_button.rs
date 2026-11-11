//! Caption buttons: untrusted icon files are refused.

use craft_ui::shell::{CaptionIcon, CaptionIcons};
use craft_ui::testing::trial;
use std::sync::Arc;

const GLYPH_PX: u32 = 16;

const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><path d="M4 4h8v8H4z" fill="#2e3436"/></svg>"##;

/// Icon files are untrusted: gzip is not expanded, a DTD or an oversized document is refused, and an `<image>` loads
/// nothing.
#[trial]
fn hostile_svgs_render_nothing() {
    let gzip = [&[0x1f, 0x8b, 0x08, 0, 0, 0, 0, 0, 0, 0xff][..], SQUARE.as_bytes()].concat();
    let dtd = r##"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY a "x">]><svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><path d="M4 4h8v8H4z"/></svg>"##;
    let many = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16">{}</svg>"#, "<g/>".repeat(20_000));
    let image = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><image href="/dev/zero" width="16" height="16"/></svg>"##;
    let mask_of = |svg: &[u8], px: u32| {
        let files = [Some(Arc::<[u8]>::from(svg)), None, None, None];
        CaptionIcons::from_files("T", 0, files).mask(CaptionIcon::Minimize, px)
    };
    for hostile in [&gzip[..], dtd.as_bytes(), many.as_bytes(), b"not svg"] {
        assert!(mask_of(hostile, GLYPH_PX).is_none());
    }
    assert!(mask_of(SQUARE.as_bytes(), 0).is_none() && mask_of(SQUARE.as_bytes(), 100_000).is_none());
    assert!(mask_of(image.as_bytes(), GLYPH_PX).is_some_and(|mask| mask.iter().all(|coverage| *coverage == 0)), "nothing drawn");
}
