//! Printer's marks (ISO 32000-2 §14.11.3): crop and bleed marks, registration targets and color
//! bars in the slug outside the trim box (§14.11.2). [`Marks::new`] is pure geometry;
//! [`Marks::draw`] paints it with craft-pdf's writer, the lines in the registration color so
//! they print on every plate.
//!
//! Coordinates are points, y down, in the caller's space: the space `trim` and `bleed` are given
//! in. Every field of [`Marks`] stays in that space. [`Marks::draw`] alone shifts by
//! `-sheet.x0, -sheet.y0`, so the sheet's top left lands on the page origin; anything an app adds
//! to the slug, such as the information line, takes the same shift.

use craft_pdf::write::backend::color::cmyk;
use craft_pdf::write::backend::geom::{Path, PathBuilder, Rect as PdfRect};
use craft_pdf::write::backend::num::NormalizedF32;
use craft_pdf::write::backend::paint::{Fill, LineCap, LineJoin, Stroke};
use craft_pdf::write::backend::surface::Surface;

use crate::ink::registration;

/// The largest offset, mark length or line width accepted, in points (two inches).
pub const MAX_MARK_SIZE: f64 = 144.0;

/// An axis-aligned rectangle in page space; `x0 <= x1` and `y0 <= y1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }

    fn union(self, o: Rect) -> Rect {
        Rect::new(self.x0.min(o.x0), self.y0.min(o.y0), self.x1.max(o.x1), self.y1.max(o.y1))
    }

    fn grow(self, by: f64) -> Rect {
        Rect::new(self.x0 - by, self.y0 - by, self.x1 + by, self.y1 + by)
    }

    fn is_valid(&self) -> bool {
        [self.x0, self.y0, self.x1, self.y1].iter().all(|v| v.is_finite()) && self.x0 <= self.x1 && self.y0 <= self.y1
    }
}

/// A point in page space.
pub type Point = (f64, f64);

/// Which marks to draw, and their sizes in points.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkOptions {
    /// Crop marks on the trim lines at each corner.
    pub crop: bool,
    /// Half-length marks on the bleed lines, when the bleed extends past the trim.
    pub bleed: bool,
    /// A circle and cross in the middle of each side.
    pub registration: bool,
    /// Process-colour patches along the top.
    pub color_bars: bool,
    /// Gap from the trim edge to where the marks start, widened to clear the bleed. At least 0.
    pub offset: f64,
    /// Length of a crop mark and the depth of the band the marks sit in. Above 0.
    pub length: f64,
    /// Stroke width. Above 0, and at most a third of `length`, so a registration circle's stroke
    /// stays inside its band.
    pub line_width: f64,
}

impl Default for MarkOptions {
    /// Every mark: 6 pt offset, 18 pt marks, 0.25 pt lines.
    fn default() -> Self {
        MarkOptions { crop: true, bleed: true, registration: true, color_bars: true, offset: 6.0, length: 18.0, line_width: 0.25 }
    }
}

impl MarkOptions {
    fn any(&self) -> bool {
        self.crop || self.bleed || self.registration || self.color_bars
    }
}

/// Why marks could not be laid out.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum MarksError {
    #[error("no printer's marks were asked for")]
    NoMarks,
    /// A size that is not finite, below its minimum, or above [`MAX_MARK_SIZE`].
    #[error("printer's marks {what} {value} is out of range")]
    Size { what: &'static str, value: f64 },
    /// A trim or bleed box that is not finite or is inverted, or a sheet that would pass
    /// [`MAX_SHEET`].
    #[error("printer's marks {0} box is not a valid rectangle")]
    Box(&'static str),
}

/// A filled colour-bar patch, in DeviceCMYK (`0.0..=1.0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Patch {
    pub rect: Rect,
    pub cmyk: [f32; 4],
}

/// Where an app's page-information line fits: the bottom band, past the crop marks. In the
/// caller's space, like the rest of [`Marks`]; subtract the sheet origin to draw on the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InfoLine {
    /// The left end of the baseline.
    pub origin: Point,
    /// A font size that fits the band.
    pub size: f64,
    /// The widest the line may be before it reaches the bottom registration target.
    pub max_width: f64,
}

/// The furthest from the origin a sheet edge may be, in points: PDF's largest page side.
pub const MAX_SHEET: f64 = 14_400.0;

/// One page's marks, laid out.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Marks {
    /// The trim box.
    pub trim: Rect,
    /// The bleed box, at least the trim.
    pub bleed: Rect,
    /// The page with its slug: the trim grown on every side to hold the marks. Use it as the
    /// media box.
    pub sheet: Rect,
    /// Gap from the trim edge to the marks: the offset, widened to clear the bleed.
    pub gap: f64,
    /// Straight strokes: crop marks, bleed marks and the registration crosses.
    pub lines: Vec<[Point; 2]>,
    /// Registration circles: centre and radius.
    pub circles: Vec<(Point, f64)>,
    pub patches: Vec<Patch>,
    pub info: InfoLine,
    /// Stroke width for lines and circles.
    pub line_width: f64,
}

/// The colour-bar patches: the four process inks, their two-colour overprints, then black at
/// 75, 50 and 25 per cent.
pub const BAR_PATCHES: [[f32; 4]; 10] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
    [0.0, 1.0, 1.0, 0.0],
    [1.0, 0.0, 1.0, 0.0],
    [1.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 0.0, 0.75],
    [0.0, 0.0, 0.0, 0.5],
    [0.0, 0.0, 0.0, 0.25],
];

impl Marks {
    /// Lays out `opts`' marks around `trim`. `bleed` is taken as at least the trim.
    pub fn new(trim: Rect, bleed: Rect, opts: &MarkOptions) -> Result<Marks, MarksError> {
        check(opts)?;
        if !trim.is_valid() {
            return Err(MarksError::Box("trim"));
        }
        if !bleed.is_valid() {
            return Err(MarksError::Box("bleed"));
        }
        let bleed = trim.union(bleed);
        let reach = (trim.x0 - bleed.x0).max(trim.y0 - bleed.y0).max(bleed.x1 - trim.x1).max(bleed.y1 - trim.y1);
        let gap = opts.offset.max(reach);
        let len = opts.length;
        // The bars and the information line stop short of the top and bottom targets, when drawn.
        let clear = if opts.registration { target_arm(len) } else { 0.0 };
        let sheet = trim.grow(gap + len);
        if [sheet.x0, sheet.y0, sheet.x1, sheet.y1].iter().any(|v| v.abs() > MAX_SHEET) || sheet.width() > MAX_SHEET || sheet.height() > MAX_SHEET {
            return Err(MarksError::Box("sheet"));
        }
        let mut m = Marks {
            trim,
            bleed,
            sheet,
            gap,
            lines: Vec::new(),
            circles: Vec::new(),
            patches: Vec::new(),
            info: info_line(trim, gap, len, clear),
            line_width: opts.line_width,
        };
        if opts.crop {
            m.corner_marks(trim, len);
        }
        if opts.bleed && bleed != trim {
            m.corner_marks(bleed, len / 2.0);
        }
        if opts.registration {
            m.registration_targets(len);
        }
        if opts.color_bars {
            m.color_bars(len, clear);
        }
        Ok(m)
    }

    /// Two strokes per corner of `at`, running outward for `len` from the gap past the trim:
    /// horizontal ones on `at`'s top and bottom lines, vertical ones on its sides.
    fn corner_marks(&mut self, at: Rect, len: f64) {
        let (t, g) = (self.trim, self.gap);
        for y in [at.y0, at.y1] {
            self.lines.push([(t.x0 - g, y), (t.x0 - g - len, y)]);
            self.lines.push([(t.x1 + g, y), (t.x1 + g + len, y)]);
        }
        for x in [at.x0, at.x1] {
            self.lines.push([(x, t.y0 - g), (x, t.y0 - g - len)]);
            self.lines.push([(x, t.y1 + g), (x, t.y1 + g + len)]);
        }
    }

    /// A circle with a cross through it, centred in the band on each side.
    fn registration_targets(&mut self, len: f64) {
        let t = self.trim;
        let mid = self.gap + len / 2.0;
        let (r, arm) = (len / 3.0, target_arm(len));
        let (cx, cy) = (f64::midpoint(t.x0, t.x1), f64::midpoint(t.y0, t.y1));
        for (x, y) in [(cx, t.y0 - mid), (cx, t.y1 + mid), (t.x0 - mid, cy), (t.x1 + mid, cy)] {
            self.circles.push(((x, y), r));
            self.lines.push([(x - arm, y), (x + arm, y)]);
            self.lines.push([(x, y - arm), (x, y + arm)]);
        }
    }

    /// Square patches along the top band, from a mark length in from the left trim edge (past
    /// the crop marks) up to the middle (short of the registration target); as many as fit.
    fn color_bars(&mut self, len: f64, clear: f64) {
        let t = self.trim;
        let size = len * 0.6;
        let y = t.y0 - self.gap - len + (len - size) / 2.0;
        let room = t.width() / 2.0 - len - clear;
        let fit = if room > 0.0 { (room / size).floor() as usize } else { 0 };
        for (i, c) in BAR_PATCHES.iter().take(fit).enumerate() {
            let x = t.x0 + len + size * i as f64;
            self.patches.push(Patch { rect: Rect::new(x, y, x + size, y + size), cmyk: *c });
        }
    }

    /// Paints the marks onto `surface`: lines and circles stroked in the registration colour
    /// (`Separation /All`), patches filled in DeviceCMYK. The surface's page should be
    /// [`Marks::sheet`]-sized, with the sheet's top left at the origin; the marks are drawn in
    /// that space. Nothing is tagged: in a tagged document, wrap the call in a page artifact.
    pub fn draw(&self, surface: &mut Surface<'_>) {
        let (dx, dy) = (-self.sheet.x0, -self.sheet.y0);
        let at = |p: Point| ((p.0 + dx) as f32, (p.1 + dy) as f32);
        let mut pb = PathBuilder::new();
        for [a, b] in &self.lines {
            let (a, b) = (at(*a), at(*b));
            pb.move_to(a.0, a.1);
            pb.line_to(b.0, b.1);
        }
        for (c, r) in &self.circles {
            circle(&mut pb, at(*c), *r as f32);
        }
        if let Some(path) = pb.finish() {
            surface.set_fill(None);
            surface.set_stroke(Some(Stroke {
                paint: registration(1.0).into(),
                width: self.line_width as f32,
                miter_limit: 4.0,
                line_cap: LineCap::Butt,
                line_join: LineJoin::Miter,
                opacity: NormalizedF32::ONE,
                dash: None,
            }));
            surface.draw_path(&path);
            surface.set_stroke(None);
        }
        for p in &self.patches {
            let (x, y) = at((p.rect.x0, p.rect.y0));
            let Some(path) = rect_path(x, y, p.rect.width() as f32, p.rect.height() as f32) else { continue };
            let [c, m, yl, k] = p.cmyk;
            surface.set_fill(Some(Fill { paint: cmyk::Color::new_f32(c, m, yl, k).into(), ..Fill::default() }));
            surface.draw_path(&path);
        }
        surface.set_fill(None);
    }
}

fn check(opts: &MarkOptions) -> Result<(), MarksError> {
    if !opts.any() {
        return Err(MarksError::NoMarks);
    }
    for (what, value, zero_ok) in [("offset", opts.offset, true), ("length", opts.length, false), ("line width", opts.line_width, false)] {
        let low = if zero_ok { value < 0.0 } else { value <= 0.0 };
        if !value.is_finite() || low || value > MAX_MARK_SIZE {
            return Err(MarksError::Size { what, value });
        }
    }
    if opts.line_width > opts.length / 3.0 {
        return Err(MarksError::Size { what: "line width", value: opts.line_width });
    }
    Ok(())
}

/// The bottom band's information line: left-aligned past the crop marks, centred in the band.
fn info_line(trim: Rect, gap: f64, len: f64, clear: f64) -> InfoLine {
    let size = (len * 0.4).min(7.0);
    let baseline = trim.y1 + gap + len / 2.0 + size * 0.35;
    InfoLine { origin: (trim.x0 + len, baseline), size, max_width: (trim.width() / 2.0 - len - clear).max(0.0) }
}

/// How far a registration target's cross reaches from its centre; the bars and the information
/// line stop short of it.
fn target_arm(len: f64) -> f64 {
    len / 3.0 * 1.4
}

/// A circle of four Béziers (κ = 0.5523).
fn circle(pb: &mut PathBuilder, (x, y): (f32, f32), r: f32) {
    let k = r * 0.552_284_8;
    pb.move_to(x + r, y);
    pb.cubic_to(x + r, y + k, x + k, y + r, x, y + r);
    pb.cubic_to(x - k, y + r, x - r, y + k, x - r, y);
    pb.cubic_to(x - r, y - k, x - k, y - r, x, y - r);
    pb.cubic_to(x + k, y - r, x + r, y - k, x + r, y);
    pb.close();
}

fn rect_path(x: f32, y: f32, w: f32, h: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.push_rect(PdfRect::from_xywh(x, y, w, h)?);
    pb.finish()
}
