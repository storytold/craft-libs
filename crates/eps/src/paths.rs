//! Neutral path extraction for EPS and PostScript files.
//!
//! This facade keeps the PostScript interpreter and its format decisions in shared dependencies;
//! applications receive Bézier paths, page bounds and warnings for their own document adapters.

use kurbo::BezPath;

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
    pub page_points: (f64, f64),
    pub warnings: Vec<String>,
    pub preview_fallback: bool,
}

/// Interpret EPS/PostScript bytes and return their editable vector path geometry.
pub fn import(bytes: &[u8], limits: Limits) -> Result<Imported, Error> {
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::InputLimit { limit: limits.max_input_bytes });
    }
    let source = vectorcraft_eps::import(bytes).map_err(Error::Parse)?;
    let page_points = source.document.artboards.first().map_or((612.0, 792.0), |board| (board.rect.width(), board.rect.height()));
    let mut paths = Vec::new();
    let mut total_elements = 0usize;
    for layer in &source.document.layers {
        collect(layer, &mut paths, &mut total_elements, limits)?;
    }
    Ok(Imported { paths, page_points, warnings: source.warnings, preview_fallback: source.preview })
}

fn collect(node: &vectorcraft_doc::Node, paths: &mut Vec<BezPath>, elements: &mut usize, limits: Limits) -> Result<(), Error> {
    if !node.visible {
        return Ok(());
    }
    if let Some(data) = node.path_data() {
        let path = data.to_bezpath();
        if !path.elements().is_empty() {
            *elements = elements.saturating_add(path.elements().len());
            if paths.len() >= limits.max_paths || *elements > limits.max_path_elements {
                return Err(Error::OutputLimit);
            }
            paths.push(path);
        }
    } else if let Some(children) = node.children() {
        for child in children {
            collect(child, paths, elements, limits)?;
        }
    }
    Ok(())
}
