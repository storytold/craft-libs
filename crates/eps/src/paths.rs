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

/// First-page path geometry in PostScript points, plus the page size and import notes.
#[derive(Debug, Clone)]
pub struct Imported {
    pub paths: Vec<BezPath>,
    /// Page bounds in source PostScript points, including a nonzero or negative origin.
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
    let section = crate::sections(bytes).map_err(|e| Error::Parse(e.to_string()))?;
    let source_box = crate::Comments::read(section.postscript).page_box().unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let [llx, lly, urx, ury] = source_box;
    let page_bounds = Rect::new(llx, lly, urx, ury);
    let mut paths = Vec::new();
    let mut total_elements = 0usize;
    let mut clipped = false;
    let mut omitted = false;
    for layer in &source.document.layers {
        collect(layer, &mut paths, &mut total_elements, limits, &mut clipped, &mut omitted)?;
    }
    let inverse_page = kurbo::Affine::new([1.0, 0.0, 0.0, -1.0, llx, ury]);
    for path in &mut paths {
        *path = inverse_page * path.clone();
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
            paths.push(path);
        }
    } else if let Some(children) = node.children() {
        let is_clip = matches!(node.kind, vectorcraft_doc::NodeKind::Group { clip: true, .. } | vectorcraft_doc::NodeKind::Layer { clip: true, .. });
        *clipped |= is_clip;
        for (index, child) in children.iter().enumerate() {
            if is_clip && index == 0 {
                continue;
            }
            collect(child, paths, elements, limits, clipped, omitted)?;
        }
    } else {
        *omitted = true;
    }
    Ok(())
}
