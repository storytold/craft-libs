//! The Ui-free rules of a marquee drag: [`MarqueeGesture`], the options bar's [`Constraint`], and the
//! pixel rectangles a marquee selects.

use craft_keymap::Mods;
use emath::Pos2;
use kurbo::{Point, Rect};

/// The two corners of a marquee in document space, as dragged: not ordered, not rounded.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct MarqueeRect {
    /// The corner where the drag began.
    pub a: Point,
    /// The corner where the pointer is now.
    pub b: Point,
}

impl MarqueeRect {
    /// The marquee from the corner `a` to the corner `b`.
    pub const fn new(a: Point, b: Point) -> Self {
        MarqueeRect { a, b }
    }

    /// The rectangle between the corners, with `x0 <= x1` and `y0 <= y1`.
    pub fn rect(&self) -> Rect {
        Rect::from_points(self.a, self.b)
    }
}

/// The options bar's marquee style. Serialises camelCase (`"free"`, `{"ratio": [1, 1]}`,
/// `{"size": [64, 64]}`) for the tools' persisted settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum Constraint {
    /// Sized by the drag; ⇧ makes it square.
    #[default]
    Free,
    /// Width and height keep the ratio `(w, h)`; the larger drag extent wins.
    Ratio(f64, f64),
    /// A fixed size `(w, h)` in document units: the drag positions it. The marquee begins at
    /// the press and follows the pointer, and a click places it there.
    Size(f64, f64),
}

impl Constraint {
    /// The corners of the marquee dragged from `start` to `end`: the style applied, ⇧ (`square`)
    /// making it square when free, ⌥ (`centre`) drawing it from the centre. For a sized marquee
    /// drawn from the centre the drag is the half extent; a fixed size is positioned at `end`,
    /// centred on it with ⌥.
    pub fn corners(self, start: Point, end: Point, square: bool, centre: bool) -> (Point, Point) {
        if let Constraint::Size(w, h) = self {
            let (w, h) = (w.max(1.0), h.max(1.0));
            let a = if centre { Point::new(end.x - w / 2.0, end.y - h / 2.0) } else { end };
            return (a, Point::new(a.x + w, a.y + h));
        }
        let (dx, dy) = (end.x - start.x, end.y - start.y);
        let (sx, sy) = (if dx < 0.0 { -1.0 } else { 1.0 }, if dy < 0.0 { -1.0 } else { 1.0 });
        let e = match self {
            Constraint::Ratio(w, h) if w > 0.0 && h > 0.0 => {
                let k = (dx.abs() / w).max(dy.abs() / h);
                Point::new(start.x + sx * k * w, start.y + sy * k * h)
            }
            _ if square => {
                let m = dx.abs().max(dy.abs());
                Point::new(start.x + sx * m, start.y + sy * m)
            }
            _ => end,
        };
        if !centre {
            return (start, e);
        }
        let (dx, dy) = (e.x - start.x, e.y - start.y);
        (Point::new(start.x - dx, start.y - dy), Point::new(start.x + dx, start.y + dy))
    }
}

/// The pixel rectangle of a fixed-size marquee between the corners `a` and `b`: exactly `size`
/// `(w, h)` in whole pixels (at least 1 × 1). Snapping both edges outwards added a pixel to each
/// side whenever the press sat between pixels, so 10 × 10 read 11 × 11. The near edge goes to the
/// closest pixel edge and the far one is the size away.
pub fn fixed_size_pixels(a: Point, b: Point, size: (f64, f64)) -> Rect {
    let (w, h) = (size.0.round().max(1.0), size.1.round().max(1.0));
    let (x0, y0) = (a.x.min(b.x).round(), a.y.min(b.y).round());
    Rect::new(x0, y0, x0 + w, y0 + h)
}

/// The pixel rectangle a marquee between two corners selects: both edges snapped outwards to
/// whole pixels, so every pixel the marquee touches is in.
pub fn marquee_pixels(a: Point, b: Point) -> Rect {
    Rect::new(a.x.min(b.x).floor(), a.y.min(b.y).floor(), a.x.max(b.x).ceil(), a.y.max(b.y).ceil())
}

/// A square marquee's pixel rectangle `r`, as [`marquee_pixels`] returns it, with equal sides.
/// Snapping each axis on its own made one side a pixel longer than the other when the press sat
/// between pixels (a ⇧ drag read 101 × 102). The shorter side grows to the longer one, away from
/// the press point `start` towards `end`, so the square still covers every pixel the drag
/// touched.
pub fn square_pixels(r: Rect, start: Point, end: Point) -> Rect {
    let n = r.width().max(r.height());
    let grow = |lo: f64, hi: f64, from: f64, to: f64| {
        let d = (n - (hi - lo)).max(0.0);
        if to < from { (lo - d, hi) } else { (lo, hi + d) }
    };
    let (x0, x1) = grow(r.x0, r.x1, start.x, end.x);
    let (y0, y1) = grow(r.y0, r.y1, start.y, end.y);
    Rect::new(x0, y0, x1, y1)
}

/// The rules of a marquee drag: ⇧ and ⌥ counted only when pressed after the
/// press (or released and pressed again), Space moving the whole marquee, and the options bar's
/// [`Constraint`]. A canvas tool runs it from its pointer and key calls.
///
/// One press is one value: [`press`](Self::press) at the press, then each pass
/// [`follow`](Self::follow) the pointer and mods, [`begin`](Self::begin) on the tool's first drag call,
/// and once begun [`size_to`](Self::size_to) the pointer's document point. Esc, the release and
/// the click are the caller's: a press that never dragged is a click.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarqueeGesture {
    /// Where the pointer is now, on the screen.
    pointer: Pos2,
    /// The mods held at the press, and now.
    at_press: Mods,
    live: Mods,
    /// ⇧ and ⌥ have been up since the press.
    released_shift: bool,
    released_alt: bool,
    /// The corner the drag started from, and where it is now, in document space.
    start: Point,
    end: Point,
    begun: bool,
}

impl MarqueeGesture {
    /// A press at `screen` (screen points) and `at` (document units, snapped by the host) with
    /// `mods` held: they choose the selection mode, not square or centre.
    pub fn press(screen: Pos2, at: Point, mods: Mods) -> Self {
        Self { pointer: screen, at_press: mods, live: mods, released_shift: false, released_alt: false, start: at, end: at, begun: false }
    }

    /// The pointer on screen as last followed.
    pub fn pointer(&self) -> Pos2 {
        self.pointer
    }

    /// The tool began the gesture with its first drag call.
    pub fn has_begun(&self) -> bool {
        self.begun
    }

    /// The pass's mods and pointer (`None` keeps the last one): records ⇧ and ⌥ being
    /// released since the press.
    pub fn follow(&mut self, screen: Option<Pos2>, mods: Mods) {
        self.live = mods;
        self.released_shift |= !mods.shift;
        self.released_alt |= !mods.alt;
        if let Some(screen) = screen {
            self.pointer = screen;
        }
    }

    /// Begins the gesture: the tool calls it on its first `drag`, the press arena having separated the drag from a
    /// click. True on the call that begins it, so the caller begins its edit gesture there.
    pub fn begin(&mut self) -> bool {
        !std::mem::replace(&mut self.begun, true)
    }

    /// ⇧ (square) and ⌥ (from the centre) for the marquee: held now, and either not held at the
    /// press or released since.
    pub fn square_and_centre(&self) -> (bool, bool) {
        (self.live.shift && (!self.at_press.shift || self.released_shift), self.live.alt && (!self.at_press.alt || self.released_alt))
    }

    /// Sizes the marquee to the pointer's document point `at` (the host's snap applied): with
    /// `space` held the whole marquee moves by the pointer's step instead. Gives the corners
    /// [`Constraint::corners`] makes of the drag with [`square_and_centre`](Self::square_and_centre).
    pub fn size_to(&mut self, at: Point, space: bool, constraint: Constraint) -> MarqueeRect {
        if space {
            self.start += at - self.end;
        }
        self.end = at;
        let (square, centre) = self.square_and_centre();
        let (a, b) = constraint.corners(self.start, self.end, square, centre);
        MarqueeRect { a, b }
    }
}
