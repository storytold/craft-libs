//! Neutral path extraction for EPS and PostScript files.
//!
//! This facade keeps the PostScript interpreter and its format decisions in shared dependencies;
//! applications receive Bézier paths, page bounds and warnings for their own document adapters.

use kurbo::{BezPath, Rect};

/// Limits on the extracted geometry. The interpreter has its own execution and allocation caps.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_paths: usize,
    pub max_path_elements: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_input_bytes: 64 * 1024 * 1024, max_paths: 200_000, max_path_elements: 2_000_000 }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("PostScript input exceeds {limit} bytes")]
    InputLimit { limit: usize },
    #[error("PostScript import failed: {0}")]
    Parse(String),
    #[error("PostScript path output exceeds configured limit")]
    OutputLimit,
}

/// First-page path geometry normalized to the top-left origin in PostScript points, plus notes.
#[derive(Debug, Clone)]
pub struct Imported {
    pub paths: Vec<BezPath>,
    /// Origin-zero page bounds in PostScript points, using the same accepted DSC box as the
    /// interpreter.
    pub page_bounds: Rect,
    pub warnings: Vec<String>,
    pub preview_fallback: bool,
}

/// Interpret EPS/PostScript bytes and return their editable vector path geometry.
pub fn import(bytes: &[u8], limits: Limits) -> Result<Imported, Error> {
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InputLimit { limit: limits.max_input_bytes });
    }
    let source = vectorcraft_eps::import(bytes).map_err(Error::Parse)?;
    let page_bounds = vectorcraft_eps::page(bytes).map_err(Error::Parse)?;
    // Illustrator's editable copy is expressed in its art-box document coordinates. The page
    // reader, in contrast, returns a page at the origin. Normalize extracted paths to match it.
    let origin = source.document.artboards.first().map(|board| board.rect.origin()).unwrap_or_default();
    let to_page = kurbo::Affine::translate((-origin.x, -origin.y));
    let mut paths = Vec::new();
    let mut total_elements = 0usize;
    let mut clipped = false;
    let mut omitted = false;
    for layer in &source.document.layers {
        collect(layer, &mut paths, &mut total_elements, limits, to_page, &mut clipped, &mut omitted)?;
    }
    let mut warnings = source.warnings;
    if clipped {
        warnings.push("EPS clipping groups are returned as unclipped artwork".into());
    }
    if omitted {
        warnings.push("visible EPS text or image content is not returned as paths".into());
    }
    Ok(Imported { paths, page_bounds, warnings, preview_fallback: source.preview })
}

fn collect(
    node: &vectorcraft_doc::Node,
    paths: &mut Vec<BezPath>,
    elements: &mut usize,
    limits: Limits,
    to_page: kurbo::Affine,
    clipped: &mut bool,
    omitted: &mut bool,
) -> Result<(), Error> {
    if !node.visible {
        return Ok(());
    }
    if let Some(data) = node.path_data() {
        if matches!(node.kind, vectorcraft_doc::NodeKind::Path { clipping: true, .. }) {
            return Ok(());
        }
        let path = data.to_bezpath();
        if !path.elements().is_empty() {
            *elements = elements.saturating_add(path.elements().len());
            if paths.len() >= limits.max_paths || *elements > limits.max_path_elements {
                return Err(Error::OutputLimit);
            }
            paths.push(to_page * path);
        }
    } else if let Some(children) = node.children() {
        let is_clip = matches!(node.kind, vectorcraft_doc::NodeKind::Group { clip: true, .. } | vectorcraft_doc::NodeKind::Layer { clip: true, .. });
        *clipped |= is_clip;
        for (index, child) in children.iter().enumerate() {
            if is_clip && index == 0 {
                continue;
            }
            collect(child, paths, elements, limits, to_page, clipped, omitted)?;
        }
    } else {
        *omitted = true;
    }
    Ok(())
}
