//! The one SVG parser. Icon files come from the build and from icon themes on the user's disk, so parsing is bounded:
//! no gzip (its size is unlimited), no DTD (entity expansion), a node limit, and no `<image>` loaded, which keeps a
//! document from making the renderer read another file or nest an SVG.

use resvg::usvg;

/// Elements and text nodes an icon SVG may have; a glyph has a handful.
const MAX_NODES: u32 = 10_000;
/// The first bytes of a gzip stream.
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// The render tree of `svg`, or why it was refused.
#[doc(hidden)]
pub fn parse(svg: &[u8]) -> Result<usvg::Tree, String> {
    if svg.starts_with(&GZIP_MAGIC) {
        return Err("compressed SVG".to_owned());
    }
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver { resolve_data: Box::new(|_, _, _| None), resolve_string: Box::new(|_, _| None) },
        ..usvg::Options::default()
    };
    let text = std::str::from_utf8(svg).map_err(|e| e.to_string())?;
    let xml = usvg::roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: MAX_NODES };
    let doc = usvg::roxmltree::Document::parse_with_options(text, xml).map_err(|e| e.to_string())?;
    usvg::Tree::from_xmltree(&doc, &options).map_err(|e| e.to_string())
}
