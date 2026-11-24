//! Pictures `\includegraphics` names, loaded through the caller's [`ImageLoader`].

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use super::export::LINE_WIDTH;
use super::limits::{MAX_GRAPHICS_PATHS, MAX_IMAGE_PROBES};
use super::units::length_pt;
use super::{Img, model};

/// Reads the bytes of a file a document names (a path relative to the `.tex` file), or `None`
/// when it can't or mustn't be read. The same shape as `craft_html::doc::ImageLoader`, so one
/// loader serves both.
pub type ImageLoader<'a> = &'a dyn Fn(&str) -> Option<Arc<Vec<u8>>>;

/// Extensions tried, in order, for a name written without one (as graphicx does).
const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "webp"];

/// Column width pictures are laid out in (points), the one `make_img` fits pictures to.
const COLUMN: f32 = 468.0;

/// The loader and what the document has said about where its pictures are, shared by every
/// parser of one document.
pub(super) struct Images<'a> {
    load: ImageLoader<'a>,
    /// `\graphicspath` folders, each ending in `/` or empty.
    paths: RefCell<Vec<String>>,
    /// Loader calls left for the document.
    probes: Cell<usize>,
}

impl<'a> Images<'a> {
    pub(super) fn new(load: ImageLoader<'a>) -> Images<'a> {
        Images { load, paths: RefCell::new(Vec::new()), probes: Cell::new(MAX_IMAGE_PROBES) }
    }

    /// `\graphicspath{{a/}{b/}}`, given as its argument's source (`{a/}{b/}`).
    pub(super) fn set_paths(&self, source: &str) {
        let paths = source
            .split(['{', '}'])
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .take(MAX_GRAPHICS_PATHS)
            .map(|p| if p.ends_with('/') { p.to_string() } else { format!("{p}/") })
            .collect();
        *self.paths.borrow_mut() = paths;
    }

    /// The picture `\includegraphics[options]{file}` shows, if the loader has it.
    pub(super) fn picture(&self, options: Option<&str>, file: &str) -> Option<Img> {
        let file = file.trim();
        if file.is_empty() {
            return None;
        }
        let mut dirs = vec![String::new()];
        dirs.extend(self.paths.borrow().iter().cloned());
        let base = file.rsplit('/').next().unwrap_or(file);
        let names: Vec<String> = if base.contains('.') {
            vec![file.to_string()]
        } else {
            std::iter::once(file.to_string()).chain(EXTENSIONS.iter().map(|e| format!("{file}.{e}"))).collect()
        };
        for dir in &dirs {
            for name in &names {
                let left = self.probes.get();
                if left == 0 {
                    return None;
                }
                self.probes.set(left - 1);
                let Some(data) = (self.load)(&format!("{dir}{name}")) else { continue };
                if let Some(img) = sized(data, options.unwrap_or(""), file) {
                    return Some(img);
                }
            }
        }
        None
    }
}

/// A picture from `data` sized as graphicx's `width`, `height`, `scale` and `keepaspectratio`
/// say; `None` when the bytes aren't a picture.
fn sized(data: Arc<Vec<u8>>, options: &str, alt: &str) -> Option<Img> {
    let natural = model::image_px(&data).map(|(w, h)| (w as f32 * 0.75, h as f32 * 0.75));
    let (mut w, mut h, mut scale, mut keep) = (None, None, None, false);
    for opt in options.split(',') {
        let (k, v) = opt.split_once('=').map_or((opt.trim(), ""), |(k, v)| (k.trim(), v.trim()));
        match k {
            "width" => w = length(v),
            "height" | "totalheight" => h = length(v),
            "scale" => scale = v.parse::<f32>().ok().filter(|s| s.is_finite() && *s > 0.0),
            "keepaspectratio" => keep = v.is_empty() || v == "true",
            _ => {}
        }
    }
    if let (Some(bw), Some(bh), true, Some((nw, nh))) = (w, h, keep, natural) {
        // Fit inside the box, keeping the shape.
        let k = (bw / nw.max(1.0)).min(bh / nh.max(1.0));
        (w, h) = (Some(nw * k), Some(nh * k));
    }
    if let (None, None, Some(s), Some((nw, nh))) = (w, h, scale, natural) {
        (w, h) = (Some(nw * s), Some(nh * s));
    }
    model::make_img(data, w, h, alt)
}

/// A graphicx length in points. Fractions of the text width are fractions of the column the
/// picture is shown in, so `0.8\textwidth` keeps its share of the line.
fn length(v: &str) -> Option<f32> {
    let pt = length_pt(v)?;
    let relative = ["\\textwidth", "\\linewidth", "\\columnwidth", "\\hsize"].iter().any(|m| v.contains(m));
    Some(if relative { pt * COLUMN / LINE_WIDTH } else { pt * 72.0 / 72.27 })
}
