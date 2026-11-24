//! Pictures `\includegraphics` names, loaded through the caller's [`ImageLoader`].

use std::cell::{Cell, RefCell};
use std::rc::Rc;
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
#[cfg(not(feature = "svg"))]
const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "webp"];
#[cfg(feature = "svg")]
const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "svgz"];

/// Column width pictures are laid out in (points), the one `make_img` fits pictures to.
const COLUMN: f32 = 468.0;

/// The loader and what the document has said about where its pictures are, shared by every
/// parser of one document.
pub(super) struct Images<'a> {
    load: ImageLoader<'a>,
    /// `\graphicspath` folders, each ending in `/` or empty. Shared, so saving them when a
    /// group starts copies a pointer, not the folders.
    paths: RefCell<Rc<[String]>>,
    /// Loader calls left for the document.
    probes: Cell<usize>,
}

impl<'a> Images<'a> {
    pub(super) fn new(load: ImageLoader<'a>) -> Images<'a> {
        Images { load, paths: RefCell::new(Rc::from([])), probes: Cell::new(MAX_IMAGE_PROBES) }
    }

    /// `\graphicspath{{a/}{b/}}`, given as its argument's source (`{a/}{b/}`).
    pub(super) fn set_paths(&self, source: &str) {
        let paths = source
            .split(['{', '}'])
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .take(MAX_GRAPHICS_PATHS)
            .map(|p| if p.ends_with('/') { p.to_string() } else { format!("{p}/") })
            .collect::<Vec<_>>();
        *self.paths.borrow_mut() = Rc::from(paths);
    }

    /// The `\graphicspath` folders in effect, to restore when a group or environment ends
    /// (`\graphicspath` is local to it, as in LaTeX).
    pub(super) fn saved_paths(&self) -> Rc<[String]> {
        Rc::clone(&self.paths.borrow())
    }

    pub(super) fn restore_paths(&self, paths: Rc<[String]>) {
        *self.paths.borrow_mut() = paths;
    }

    /// The picture `\includegraphics[options]{file}` shows, if the loader has it. `svg_first`
    /// (`\includesvg`) tries `.svg` before the other extensions.
    pub(super) fn picture(&self, options: Option<&str>, file: &str, svg_first: bool) -> Option<Img> {
        let file = file.trim();
        // Once the loader calls are spent nothing more is looked up, or even prepared.
        if file.is_empty() || self.probes.get() == 0 {
            return None;
        }
        let mut dirs = vec![String::new()];
        dirs.extend(self.paths.borrow().iter().cloned());
        let base = file.rsplit('/').next().unwrap_or(file);
        let names: Vec<String> = if base.contains('.') {
            vec![file.to_string()]
        } else {
            let first = svg_first.then(|| format!("{file}.svg"));
            first.into_iter().chain(std::iter::once(file.to_string())).chain(EXTENSIONS.iter().map(|e| format!("{file}.{e}"))).collect()
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
    #[cfg(feature = "svg")]
    if craft_svg::sniff::is_svg(&data) || craft_svg::sniff::is_svgz(&data) {
        return svg::picture(&data, options, alt);
    }
    let natural = model::image_px(&data).map(|(w, h)| (w as f32 * 0.75, h as f32 * 0.75));
    let (w, h) = box_size(options, natural);
    model::make_img(data, w, h, alt)
}

/// The width and height (points) the options give a picture of `natural` size, either left
/// `None` for the picture's own proportions.
fn box_size(options: &str, natural: Option<(f32, f32)>) -> (Option<f32>, Option<f32>) {
    let (mut w, mut h, mut scale, mut keep) = (None, None, None, false);
    for opt in options.split(',') {
        let (k, v) = opt.split_once('=').map_or((opt.trim(), ""), |(k, v)| (k.trim(), v.trim()));
        // `width={4cm}` is `width=4cm`.
        let v = v.strip_prefix('{').and_then(|v| v.strip_suffix('}')).map_or(v, str::trim);
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
    (w, h)
}

/// SVG pictures, drawn to a PNG at print resolution for the size they are shown at.
#[cfg(feature = "svg")]
mod svg {
    use super::{COLUMN, Img, model};
    use craft_svg::tree::{self, Limits, draw, usvg};

    /// Pixels drawn per point shown (288 dpi), so the picture stays sharp when zoomed or printed.
    const PX_PER_PT: f32 = 4.0;
    /// Longest side drawn, in pixels.
    const MAX_SIDE: f32 = 4096.0;
    /// Bytes the drawing may hold at once.
    const BUDGET: u64 = 256 << 20;

    pub(super) fn picture(data: &[u8], options: &str, alt: &str) -> Option<Img> {
        let tree = tree::parse(data, &Limits::default(), &usvg::Options::default()).ok()?;
        let size = tree.size();
        let (nw, nh) = (size.width() * 0.75, size.height() * 0.75);
        if !(nw.is_finite() && nh.is_finite() && nw > 0.0 && nh > 0.0) {
            return None;
        }
        // The size shown, as `make_img` sets it: the options' box, else the natural size, no
        // wider than the column.
        let (mut w, mut h) = match super::box_size(options, Some((nw, nh))) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w * nh / nw),
            (None, Some(h)) => (h * nw / nh, h),
            (None, None) => (nw, nh),
        };
        if w > COLUMN {
            h *= COLUMN / w;
            w = COLUMN;
        }
        let k = PX_PER_PT.min(MAX_SIDE / w.max(h).max(1.0));
        let (pw, ph) = ((w * k).round().max(1.0) as u32, (h * k).round().max(1.0) as u32);
        let ts = usvg::Transform::from_scale(pw as f32 / size.width(), ph as f32 / size.height());
        let png = draw::render(&tree, ts, pw, ph, 0, BUDGET).ok()?.encode_png().ok()?;
        model::make_img(png, Some(w), Some(h), alt)
    }
}

/// A graphicx length in points. Fractions of the text width are fractions of the column the
/// picture is shown in, so `0.8\textwidth` keeps its share of the line.
fn length(v: &str) -> Option<f32> {
    let pt = length_pt(v)?;
    let relative = ["\\textwidth", "\\linewidth", "\\columnwidth", "\\hsize"].iter().any(|m| v.contains(m));
    Some(if relative { pt * COLUMN / LINE_WIDTH } else { pt * 72.0 / 72.27 })
}
