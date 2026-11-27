//! The one SVG parser, craft-svg's bounded entry point. Icon files come from the build and from icon themes on the
//! user's disk, so parsing is bounded: no gzip, no DTD (entity expansion), a node limit, a nesting limit, and no
//! `<image>` loaded, which keeps a document from making the renderer read another file or nest an SVG.

use craft_svg::tree::{self, Limits, usvg};

/// Elements and text nodes an icon SVG may have; a glyph has a handful.
const MAX_NODES: u32 = 10_000;
/// The first bytes of a gzip stream.
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// What an icon may use. Icons parse on the UI thread, whose stack may be 1 MB (Windows' main thread): craft-svg
/// gives about 34 levels for that, so nesting stops at 32.
fn icon_limits() -> Limits {
    Limits {
        bytes: 1 << 20,
        depth: 32,
        tree_depth: 32,
        xml_nodes: MAX_NODES,
        allow_dtd: false,
        refuse_entity_markup: true,
        embedded_bytes: 0,
        embedded_depth: 0,
        embedded_raster_pixels: 0,
        ..Limits::default()
    }
}

/// The render tree of `svg`, or why it was refused: the suite's one bounded SVG entry point, for icons and for
/// pictures alike. A gzip stream, a DTD and entity markup are refused; the input is capped at 1 MiB, the XML at
/// 10,000 nodes and the nesting at 32 levels; an `<image>` resolves to nothing, so a document reads no other file and
/// embeds no raster or nested SVG.
pub fn parse(svg: &[u8]) -> Result<usvg::Tree, String> {
    parse_styled(svg, None)
}

/// The render tree of `svg` under the injected style sheet `style_sheet`, or why it was refused: the one bounded
/// parser, so a layer by class is read by the SVG engine's own selectors.
#[doc(hidden)]
pub fn parse_styled(svg: &[u8], style_sheet: Option<&str>) -> Result<usvg::Tree, String> {
    if svg.starts_with(&GZIP_MAGIC) {
        return Err("compressed SVG".to_owned());
    }
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver { resolve_data: Box::new(|_, _, _| None), resolve_string: Box::new(|_, _| None) },
        style_sheet: style_sheet.map(str::to_owned),
        ..usvg::Options::default()
    };
    tree::parse(svg, &icon_limits(), &options).map_err(|e| e.to_string())
}
