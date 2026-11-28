//! The painted pointers (8.5): hand, magnifier, brush outline, tool icon, badges, the rotate, distort and skew
//! pointers, the type pointers, the locator, the arrow, the path-text pointer and the crosshair with a tool icon. Their
//! art is drawn in points around the hotspot, like an OS cursor bitmap, so its coordinates are data tables, not layout
//! sizes.

mod art;

use super::Mirror;
use super::SvgIcon;
use crate::runtime::cursor::{CursorArt, CursorBitmap, MAX_SIDE};
use crate::runtime::env::Env;
use crate::style::Ink;
use crate::style::UiTokens;
pub(crate) use art::Art;
use art::Mark;
use craft_behavior::{CountMark, CursorBadge, PaintedCursor, PointerGlyph};
use craft_i18n::Direction;
use egui::{Color32, Pos2, Rect, Stroke, Vec2};
use std::cell::RefCell;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// The window image of the painted pointer `cursor` at `scale` device pixels per point, rasterised from the art the
/// ghost paints; `None` when the art does not fit a cursor image.
pub fn painted_image(env: &Env, cursor: &PaintedCursor, scale: f32) -> Option<CursorBitmap> {
    pointer_art(env, cursor, false, false).rasterise(scale, MAX_SIDE)
}

/// The art of `cursor`: the hand as a fist while `closed`; with `marks_only`, a brush tip's centre cross and badge
/// without its ring. The ghost paints it and the window's cursor image is rasterised from it, so both look the same.
pub(crate) fn pointer_art(env: &Env, cursor: &PaintedCursor, closed: bool, marks_only: bool) -> Art {
    use craft_behavior::PointerGlyph;
    let overlay = &env.tokens.color.overlay;
    let pen = Pen {
        art: RefCell::new(Art::default()),
        fill: crate::style::rgba_color(overlay.cursor),
        halo: crate::style::rgba_color(overlay.halo),
        thin: env.tokens.stroke.thin,
        rtl: env.direction.is_rtl(),
    };
    let ink = crate::style::ink_color(&env.tokens, Ink::OnDark);
    let edge = env.tokens.stroke.thin;
    match cursor {
        PaintedCursor::Outline { rings, centre, badge, .. } => {
            if !marks_only {
                outline(&pen, rings);
            }
            tip_marks(&pen, *centre || marks_only, *badge, pen.thin, pen.fill);
        }
        PaintedCursor::Circle { radius, centre, badge, .. } => {
            if !marks_only {
                circle(&pen, *radius, edge, ink);
            }
            tip_marks(&pen, *centre || marks_only, *badge, edge, ink);
        }
        PaintedCursor::Crosshair => crosshair(&pen, env.tokens.size.icon.sm, edge, ink),
        PaintedCursor::Hand => hand(&pen, closed),
        PaintedCursor::Zoom { out, .. } => zoom(&pen, *out),
        PaintedCursor::Tool { icon, hotspot, badge, .. } => {
            let side = env.tokens.size.icon.md;
            let icon = SvgIcon::of(&env.tokens, env.direction, side, icon, Mirror::Auto);
            tool(&pen, icon, *hotspot, *badge, side);
        }
        PaintedCursor::Badged { badge, .. } => badge_beside(&pen, *badge),
        PaintedCursor::Turn { toward, .. } => turn(&pen, *toward),
        PaintedCursor::Distort { dir, .. } => distort(&pen, *dir),
        PaintedCursor::Skew { dir, .. } => skew(&pen, *dir),
        PaintedCursor::TypeOn { path, .. } => type_on(&pen, *path),
        PaintedCursor::IBeam => i_beam_pointer(&pen),
        PaintedCursor::Locator => locator(&pen),
        PaintedCursor::Arrow { hollow, toward, .. } => arrow(&pen, *hollow, *toward),
        PaintedCursor::PathText { start, end, .. } => path_text(&pen, *start, *end),
        PaintedCursor::PointerTool { pointer, icon, badge, .. } => {
            // The arrow's icon sits under its body, so it takes the larger size to show past the arrow.
            let side = if *pointer == PointerGlyph::Arrow { env.tokens.size.icon.md } else { env.tokens.size.icon.sm };
            let icon = SvgIcon::of(&env.tokens, env.direction, side, icon, Mirror::Auto);
            pointer_tool(&pen, *pointer, icon, *badge, side, edge, ink);
        }
        // `Hidden` has no marks.
        _ => {}
    }
    pen.art.into_inner()
}

/// The icon sets draw on a box this many units on a side; a tool pointer's hotspot is in these.
const ICON_UNITS: f32 = 24.0;
/// Where a badge sits beside a system pointer arrow: toward its lower right.
const BADGE_BESIDE: [f32; 2] = [12.0, 12.0];
/// Half the side of a badge glyph.
const BADGE_HALF: f32 = 3.5;
/// The scissors badge: how far its blades' ends sit from the axis, and the radius of its handle rings, as fractions of
/// `BADGE_HALF`; and the radius of the close badge's circle.
const SCISSORS_SPREAD: f32 = 0.5;
const SCISSORS_HANDLE: f32 = 0.35;
const CLOSE_RADIUS: f32 = 0.8;
/// Badge proportions as fractions of `BADGE_HALF`: the barbs of the move cross and of the arrow, the sampler target's
/// ring, how far the continue slash leans, the count numeral's stem and flag, the centre and half side of the glyph
/// beside it; and the segments of the corner badge's arc.
const MOVE_BARB: f32 = 0.4;
const ARROW_BARB: f32 = 0.6;
const TARGET_RING: f32 = 0.6;
const CONTINUE_LEAN: f32 = 0.7;
const COUNT_STEM: f32 = -0.7;
const COUNT_FLAG: f32 = 0.5;
const COUNT_GLYPH_AT: f32 = 0.6;
const COUNT_GLYPH: f32 = 0.7;
const CORNER_SEGMENTS: usize = 6;
/// The I-beam: half its height and half the width of its serifs.
const BEAM_HALF: f32 = 8.0;
const SERIF_HALF: f32 = 3.0;
/// The area-type dots: their count on an ellipse of these radii. The path-type dots: their count on the wave
/// `y = base + amplitude * sin(frequency * x)` from the first x in steps. A dot's side.
const RING_DOTS: usize = 12;
const RING_RADII: [f32; 2] = [8.0, 10.5];
const WAVE_DOTS: usize = 9;
const WAVE_FROM: f32 = -10.0;
const WAVE_STEP: f32 = 2.5;
const WAVE_BASE: f32 = 4.0;
const WAVE_AMPLITUDE: f32 = 2.0;
const WAVE_FREQUENCY: f32 = 0.4;
const DOT_SIDE: f32 = 1.5;
/// The side of the locator's square.
const LOCATOR_SIDE: f32 = 9.0;
/// The arrow's outline with its tip on the pointer, and the triangles that fill it: the outline is not convex.
const ARROW: [[f32; 2]; 7] = [[0.0, 0.0], [0.0, 16.0], [3.6, 12.6], [6.8, 19.0], [9.6, 17.8], [6.6, 11.6], [11.5, 11.5]];
const ARROW_TRIANGLES: [[usize; 3]; 5] = [[0, 1, 2], [0, 2, 5], [0, 5, 6], [2, 3, 4], [2, 4, 5]];
/// The direction the base arrow's tip points in, in radians with y down: opposite the bisector of its two edges (67.5
/// degrees).
const ARROW_TIP_ANGLE: f32 = 11.0 * PI / 8.0;
/// The path-text pointer's triangle toward a grabbable start; the end's is its mirror.
const GRAB_TRIANGLE: [[f32; 2]; 3] = [[3.0, -4.0], [8.0, 0.0], [3.0, 4.0]];
/// The top-left corner of the tool icon beside a crosshair.
const CROSSHAIR_ICON: [f32; 2] = [10.0, 10.0];
/// The top-left corner of the tool icon beside an arrow: under the arrow's body, which is drawn over it.
const ARROW_ICON: [f32; 2] = [6.0, 6.0];
/// The rotate pointer: radius of its arc, half its sweep in radians, and its segment count.
const TURN_RADIUS: f32 = 12.0;
const HALF_SWEEP: f32 = 0.75;
const ARC_SEGMENTS: usize = 16;
/// A rotate arrowhead's unit of size per radius, and its proportions in those units: the tip lies `HEAD_REACH` past the
/// arc's end, the head is `HEAD_LENGTH` long, its base lies back by `HEAD_BACK` and is `2 * HEAD_HALF_WIDTH` wide.
const HEAD_PER_RADIUS: f32 = 1.0 / 12.0;
const HEAD_REACH: f32 = 2.0;
const HEAD_LENGTH: f32 = 3.5;
const HEAD_BACK: f32 = 3.0;
const HEAD_HALF_WIDTH: f32 = 3.5;
/// The distort dart: length, half width, and how far back from the tip its notch lies as a fraction of the length.
const DART_LENGTH: f32 = 16.0;
const DART_HALF_WIDTH: f32 = 7.0;
const DART_NOTCH: f32 = 0.65;
/// The skew arrows: distance of each from the pointer across the edge, half length, head length and head back.
const SKEW_GAP: f32 = 4.0;
const SKEW_HALF_LENGTH: f32 = 9.0;
const SKEW_HEAD: f32 = 3.5;
const SKEW_BACK: f32 = 2.5;
/// The open hand: the offset of its box from the pointer, the palm's centre, half size and corner radius, the
/// fingers' x and tip heights, where the fingers start, their radius, and the thumb's ends and radius.
const HAND_SHIFT: [f32; 2] = [2.0, 0.5];
const PALM_CENTER: [f32; 2] = [0.0, 4.5];
const PALM_HALF: [f32; 2] = [6.8, 5.5];
const PALM_ROUND: f32 = 3.0;
const FINGER_X: [f32; 4] = [-5.1, -1.7, 1.7, 5.1];
const FINGER_TIP: [f32; 4] = [-7.0, -9.5, -8.5, -5.5];
const FINGER_BASE: f32 = 3.0;
const FINGER_R: f32 = 1.7;
const THUMB: ([f32; 2], [f32; 2], f32) = ([-4.5, 7.0], [-10.5, 1.5], 1.9);
/// The fist: the box, the knuckles' ends and radius, the thumb, the curl line across the fingers and where the finger
/// creases start.
const FIST_CENTER: [f32; 2] = [0.0, 3.0];
const FIST_HALF: [f32; 2] = [7.2, 6.5];
const FIST_ROUND: f32 = 3.6;
const KNUCKLE_FROM: f32 = -1.0;
const KNUCKLE_TO: f32 = -2.2;
const KNUCKLE_R: f32 = 1.9;
const FIST_THUMB: ([f32; 2], [f32; 2], f32) = ([-4.5, 7.0], [-9.6, 3.6], 2.0);
const FIST_CURL: ([f32; 2], [f32; 2]) = ([-4.0, 4.4], [6.0, 4.4]);
const FIST_CREASE_FROM: f32 = -2.6;
const CREASE_X: [f32; 3] = [-3.4, 0.0, 3.4];
const CREASE_END: f32 = 3.5;
/// The magnifier: the lens radius (its centre is the pointer), the handle's ends and radius, and the arms of the plus
/// or minus.
const LENS: f32 = 6.5;
const HANDLE: ([f32; 2], [f32; 2], f32) = ([5.2, 5.2], [10.8, 10.8], 2.2);
const GLYPH_ARM: f32 = 3.4;

fn v(point: [f32; 2]) -> Vec2 {
    Vec2::new(point[0], point[1])
}

/// What the pointers draw with: the art they add marks to, the mark colour, its halo, the hairline width, and whether
/// the reading direction is right to left (marks that point along it mirror).
pub(crate) struct Pen {
    pub art: RefCell<Art>,
    pub fill: Color32,
    pub halo: Color32,
    pub thin: f32,
    pub rtl: bool,
}

impl Pen {
    /// 1 for left to right, -1 for right to left: the x sign of marks that point toward the reading end.
    fn forward(&self) -> f32 {
        if self.rtl { -1.0 } else { 1.0 }
    }
}

/// A line mark, in points from the pointer.
enum Line {
    Segment(Vec2, Vec2),
    /// An axis-aligned segment snapped to whole device pixels.
    Rule(Vec2, Vec2),
    Circle(Vec2, f32),
    Rect(Rect),
    /// An open line through the points.
    Polyline(Vec<Vec2>),
}

impl Pen {
    fn push(&self, mark: Mark) {
        self.art.borrow_mut().push(mark);
    }

    /// `lines` as hairlines in the mark colour, each over a halo line a hairline wider on each side.
    fn marks(&self, lines: &[Line]) {
        self.lines(self.thin, self.fill, lines);
    }

    /// `lines` of `width` in `color`, each over a halo line a hairline wider on each side.
    fn lines(&self, width: f32, color: Color32, lines: &[Line]) {
        for (width, color) in [(width + self.thin * 2.0, self.halo), (width, color)] {
            // A transparent halo (the interface icon's mark has none) would paint every mark twice, once unseen.
            if color.a() == 0 {
                continue;
            }
            let stroke = Stroke::new(width, color);
            for line in lines {
                self.push(match line {
                    Line::Segment(a, b) => Mark::Line(*a, *b, stroke),
                    Line::Rule(a, b) => Mark::Rule(*a, *b, stroke),
                    Line::Circle(center, radius) => Mark::Ring(*center, *radius, stroke),
                    Line::Rect(rect) => Mark::Frame(*rect, stroke),
                    Line::Polyline(points) => Mark::Path(points.clone(), false, stroke),
                });
            }
        }
    }

    fn disc(&self, center: Vec2, radius: f32, color: Color32) {
        self.push(Mark::Disc(center, radius, color));
    }

    /// The segment `a` to `b` grown by `radius`.
    fn capsule(&self, a: Vec2, b: Vec2, radius: f32, color: Color32) {
        self.push(Mark::Line(a, b, Stroke::new(2.0 * radius, color)));
        self.disc(a, radius, color);
        self.disc(b, radius, color);
    }

    /// A polygon filled with the mark colour and outlined in the halo.
    fn polygon(&self, points: Vec<Vec2>) {
        self.push(Mark::Polygon(points, self.fill, Stroke::new(self.thin, self.halo)));
    }
}

/// A circle of `radius` points around the pointer.
pub(crate) fn circle(pen: &Pen, radius: f32, width: f32, color: Color32) {
    pen.lines(width, color, &[Line::Circle(Vec2::ZERO, radius)]);
}

/// A crosshair with arms of `arm` points.
pub(crate) fn crosshair(pen: &Pen, arm: f32, width: f32, color: Color32) {
    pen.lines(width, color, &[Line::Segment(v([-arm, 0.0]), v([arm, 0.0])), Line::Segment(v([0.0, -arm]), v([0.0, arm]))]);
}

/// A brush tip's marks on the pointer: the centre mark when `centre`, a crosshair of `PaintedCursor::CENTRE_ARM` arms
/// in the tip's own line of `width` in `color`, and `badge` centred over it.
pub(crate) fn tip_marks(pen: &Pen, centre: bool, badge: Option<CursorBadge>, width: f32, color: Color32) {
    if centre {
        crosshair(pen, PaintedCursor::CENTRE_ARM, width, color);
    }
    if let Some(badge) = badge {
        self::badge(pen, Vec2::ZERO, badge);
    }
}

/// Rings of a brush tip, as offsets from the pointer.
pub(crate) fn outline(pen: &Pen, rings: &[Vec<Pos2>]) {
    for ring in rings.iter().filter(|ring| ring.len() >= 2) {
        let points: Vec<Vec2> = ring.iter().map(|p| p.to_vec2()).collect();
        pen.push(Mark::Path(points.clone(), true, Stroke::new(pen.thin * 3.0, pen.halo)));
        pen.push(Mark::Path(points, true, Stroke::new(pen.thin, pen.fill)));
    }
}

/// The hand: open, or the fist while `closed`. Its silhouette is in the mark colour over the same grown by a hairline in
/// the halo colour, with the creases between the fingers.
pub(crate) fn hand(pen: &Pen, closed: bool) {
    let shift = |p: [f32; 2]| v(p) + v(HAND_SHIFT);
    for (grow, color) in [(pen.thin, pen.halo), (0.0, pen.fill)] {
        let (center, half, round) = if closed { (FIST_CENTER, FIST_HALF, FIST_ROUND) } else { (PALM_CENTER, PALM_HALF, PALM_ROUND) };
        let palm = Rect::from_center_size(shift(center).to_pos2(), v(half) * 2.0).expand(grow);
        pen.push(Mark::Block(palm, round + grow, color));
        if closed {
            for x in FINGER_X {
                pen.capsule(shift([x, KNUCKLE_FROM]), shift([x, KNUCKLE_TO]), KNUCKLE_R + grow, color);
            }
        } else {
            for (x, tip) in FINGER_X.into_iter().zip(FINGER_TIP) {
                pen.capsule(shift([x, FINGER_BASE]), shift([x, tip]), FINGER_R + grow, color);
            }
        }
        let (a, b, r) = if closed { FIST_THUMB } else { THUMB };
        pen.capsule(shift(a), shift(b), r + grow, color);
    }
    let stroke = Stroke::new(pen.thin, pen.halo);
    let crease = |a: [f32; 2], b: [f32; 2]| {
        pen.push(Mark::Line(shift(a), shift(b), stroke));
    };
    if closed {
        crease(FIST_CURL.0, FIST_CURL.1);
        for x in CREASE_X {
            crease([x, FIST_CREASE_FROM], [x, FIST_CURL.0[1]]);
        }
    } else {
        for (x, (a, b)) in CREASE_X.into_iter().zip(FINGER_TIP.into_iter().zip(FINGER_TIP.into_iter().skip(1))) {
            // Each crease starts just below the lower of the two fingertips it separates.
            crease([x, a.max(b) + 1.0], [x, CREASE_END]);
        }
    }
}

/// The magnifier with a plus, or a minus when `out`.
pub(crate) fn zoom(pen: &Pen, out: bool) {
    let (a, b, r) = HANDLE;
    for (grow, color) in [(pen.thin, pen.halo), (0.0, pen.fill)] {
        pen.disc(Vec2::ZERO, LENS + grow, color);
        pen.capsule(v(a), v(b), r + grow, color);
    }
    let stroke = Stroke::new(pen.thin, pen.halo);
    pen.push(Mark::Line(v([-GLYPH_ARM, 0.0]), v([GLYPH_ARM, 0.0]), stroke));
    if !out {
        pen.push(Mark::Line(v([0.0, -GLYPH_ARM]), v([0.0, GLYPH_ARM]), stroke));
    }
}

/// A tool's `icon` of `side` points with `hotspot` (a point of the icon's 24-unit box) on the pointer, in the mark
/// colour over a halo of the icon drawn a hairline off in every direction, and its badge at the icon's lower right. An
/// icon the catalog lacks leaves only the badge.
pub(crate) fn tool(pen: &Pen, icon: Option<SvgIcon>, hotspot: Vec2, badge: Option<CursorBadge>, side: f32) {
    let rect = Rect::from_min_size((-hotspot * (side / ICON_UNITS)).to_pos2(), Vec2::splat(side));
    if let Some(icon) = icon {
        icon_on_halo(pen, rect, icon);
    }
    if let Some(badge) = badge {
        self::badge(pen, rect.right_bottom().to_vec2(), badge);
    }
}

/// `icon` in `rect` in the mark colour, over copies of it a hairline off in each of eight directions in the halo.
fn icon_on_halo(pen: &Pen, rect: Rect, icon: SvgIcon) {
    for dx in [-1.0, 0.0, 1.0] {
        for dy in [-1.0, 0.0, 1.0] {
            if dx != 0.0 || dy != 0.0 {
                pen.push(Mark::Icon(rect.translate(Vec2::new(dx, dy) * pen.thin), icon, pen.halo));
            }
        }
    }
    pen.push(Mark::Icon(rect, icon, pen.fill));
}

/// The I-beam centred on the pointer: its stem and two serifs, as rules so each lies on whole device pixels and the
/// beam stays crisp at every scale.
fn i_beam() -> [Line; 3] {
    let (b, s) = (BEAM_HALF, SERIF_HALF);
    [Line::Rule(v([0.0, -b]), v([0.0, b])), Line::Rule(v([-s, -b]), v([s, -b])), Line::Rule(v([-s, b]), v([s, b]))]
}

/// The plain I-beam pointer: the beam in the mark colour over its halo, visible on light and dark grounds alike.
pub(crate) fn i_beam_pointer(pen: &Pen) {
    pen.marks(&i_beam());
}

/// The type tool's pointer: the I-beam inside a dotted ellipse, or crossed by a dotted wave when `path`.
pub(crate) fn type_on(pen: &Pen, path: bool) {
    let dots: Vec<Vec2> = if path {
        (0..WAVE_DOTS)
            .map(|i| {
                let x = WAVE_FROM + i as f32 * WAVE_STEP;
                Vec2::new(x, WAVE_BASE + WAVE_AMPLITUDE * (WAVE_FREQUENCY * x).sin())
            })
            .collect()
    } else {
        (0..RING_DOTS)
            .map(|i| {
                let a = i as f32 / RING_DOTS as f32 * TAU;
                Vec2::new(a.cos() * RING_RADII[0], a.sin() * RING_RADII[1])
            })
            .collect()
    };
    let dot = |centre: Vec2, grow: f32, color: Color32| {
        // design: allow[literal-size/vec2](pointer art is a fixed bitmap-like drawing around the hotspot, not a layout size)
        pen.push(Mark::Block(Rect::from_center_size(centre.to_pos2(), Vec2::splat(DOT_SIDE)).expand(grow), 0.0, color));
    };
    // The dots' halos go under the beam and their fills over it, so a dot on the stem stays whole.
    for &centre in &dots {
        dot(centre, pen.thin, pen.halo);
    }
    pen.marks(&i_beam());
    for &centre in &dots {
        dot(centre, 0.0, pen.fill);
    }
}

/// The locator: a square frame centred on the pointer between halo frames a hairline outside and inside it.
pub(crate) fn locator(pen: &Pen) {
    let rect = Rect::from_center_size(Pos2::ZERO, Vec2::splat(LOCATOR_SIDE));
    let halo = Stroke::new(pen.thin, pen.halo);
    pen.push(Mark::Frame(rect.expand(pen.thin), halo));
    pen.push(Mark::Frame(rect.shrink(pen.thin), halo));
    pen.push(Mark::Frame(rect, Stroke::new(pen.thin, pen.fill)));
}

/// The arrow with its tip on the pointer: filled with the halo and edged in the mark colour, or the other way round when
/// `hollow`. With `toward`, the art turns about the tip so the tip points along that direction (a zero or non-finite
/// one leaves the art unturned); `None` is the base art.
pub(crate) fn arrow(pen: &Pen, hollow: bool, toward: Option<Vec2>) {
    let (fill, edge) = if hollow { (pen.fill, pen.halo) } else { (pen.halo, pen.fill) };
    let points: Vec<Vec2> = ARROW.iter().map(|&p| v(p)).collect();
    let turned: Vec<Vec2> = match toward {
        None => points,
        Some(dir) => {
            // The base art's tip points up and left along the bisector of its two edges: 247.5 degrees, y down.
            let base = Vec2::angled(ARROW_TIP_ANGLE);
            let u = unit(dir, base);
            let (cos, sin) = (base.dot(u), base.x * u.y - base.y * u.x);
            points.into_iter().map(|p| Vec2::new(cos * p.x - sin * p.y, sin * p.x + cos * p.y)).collect()
        }
    };
    for triangle in ARROW_TRIANGLES {
        pen.push(Mark::Polygon(triangle.iter().map(|&i| turned[i]).collect(), fill, Stroke::NONE));
    }
    pen.push(Mark::Path(turned, true, Stroke::new(pen.thin, edge)));
}

/// The I-beam across a path with a filled triangle toward each grabbable end: right for `start`, left for `end`.
pub(crate) fn path_text(pen: &Pen, start: bool, end: bool) {
    pen.marks(&i_beam());
    for sign in [start.then_some(1.0f32), end.then_some(-1.0)].into_iter().flatten() {
        pen.polygon(GRAB_TRIANGLE.iter().map(|&[x, y]| Vec2::new(x * sign, y)).collect());
    }
}

/// `pointer` with the tool's `icon` of `side` points below and right of it: a crosshair with arms of `side` points in a
/// line of `width` in `color`, as the plain crosshair pointer, beside the icon; or the solid arrow, as the plain arrow
/// pointer, over the icon's top-left corner; `badge` at the icon's lower right. An icon the catalog lacks leaves only the
/// pointer and the badge.
pub(crate) fn pointer_tool(
    pen: &Pen,
    pointer: PointerGlyph,
    icon: Option<SvgIcon>,
    badge: Option<CursorBadge>,
    side: f32,
    width: f32,
    color: Color32,
) {
    let corner = match pointer {
        PointerGlyph::Arrow => ARROW_ICON,
        _ => CROSSHAIR_ICON,
    };
    let rect = Rect::from_min_size(v(corner).to_pos2(), Vec2::splat(side));
    if let Some(icon) = icon {
        icon_on_halo(pen, rect, icon);
    }
    if let Some(badge) = badge {
        self::badge(pen, rect.right_bottom().to_vec2(), badge);
    }
    match pointer {
        PointerGlyph::Arrow => arrow(pen, false, None),
        _ => crosshair(pen, side, width, color),
    }
}

/// A plus of arm `half` centred `center` from the pointer.
fn plus(center: Vec2, half: f32) -> [Line; 2] {
    [
        Line::Segment(center - Vec2::new(half, 0.0), center + Vec2::new(half, 0.0)),
        Line::Segment(center - Vec2::new(0.0, half), center + Vec2::new(0.0, half)),
    ]
}

/// A minus of arm `half` centred `center` from the pointer.
fn minus(center: Vec2, half: f32) -> Line {
    Line::Segment(center - Vec2::new(half, 0.0), center + Vec2::new(half, 0.0))
}

/// The mark of `badge` in `ink`, centred in `rect` at a cursor badge's size and with no halo: the drawing the cursor
/// badges use, for interface icons that show the same mark. The hairline and the reading direction are those of the
/// component that paints the icon: `tokens` and `direction`.
pub(crate) fn paint_badge(painter: &egui::Painter, tokens: &UiTokens, direction: Direction, rect: Rect, badge: CursorBadge, ink: Color32) {
    let pen = Pen { art: RefCell::new(Art::default()), fill: ink, halo: Color32::TRANSPARENT, thin: tokens.stroke.thin, rtl: direction.is_rtl() };
    self::badge(&pen, Vec2::ZERO, badge);
    pen.art.into_inner().paint(painter, rect.center());
}

/// A badge centred `center` from the pointer.
pub(crate) fn badge(pen: &Pen, center: Vec2, badge: CursorBadge) {
    let h = BADGE_HALF;
    let at = |x: f32, y: f32| center + Vec2::new(x, y);
    let axes = [Vec2::X, -Vec2::X, Vec2::Y, -Vec2::Y];
    let marks: Vec<Line> = match badge {
        CursorBadge::Add => plus(center, h).into(),
        CursorBadge::Subtract => vec![minus(center, h)],
        CursorBadge::Intersect => {
            let d = 0.8 * h;
            vec![Line::Segment(at(-d, -d), at(d, d)), Line::Segment(at(-d, d), at(d, -d))]
        }
        CursorBadge::Copy => {
            // A sheet behind a sheet.
            let sheet = Vec2::splat(h * 1.4);
            let near = h / 3.0;
            vec![
                Line::Rect(Rect::from_center_size(at(-near, -near).to_pos2(), sheet)),
                Line::Rect(Rect::from_center_size(at(near, near).to_pos2(), sheet)),
            ]
        }
        CursorBadge::NotAllowed => {
            let d = h * std::f32::consts::FRAC_1_SQRT_2;
            vec![Line::Circle(center, h), Line::Segment(at(-d, d), at(d, -d))]
        }
        CursorBadge::Scissors => {
            let (spread, handle) = (h * SCISSORS_SPREAD, h * SCISSORS_HANDLE);
            vec![
                Line::Segment(at(h, -spread), at(-h, spread)),
                Line::Segment(at(h, spread), at(-h, -spread)),
                Line::Circle(at(-h, spread), handle),
                Line::Circle(at(-h, -spread), handle),
            ]
        }
        CursorBadge::Close => vec![Line::Circle(center, h * CLOSE_RADIUS)],
        CursorBadge::Move => {
            let b = h * MOVE_BARB;
            let mut marks: Vec<Line> = plus(center, h).into();
            for d in axes {
                let (tip, back, across) = (center + d * h, d * b, Vec2::new(-d.y, d.x) * b);
                marks.push(Line::Segment(tip, tip - back + across));
                marks.push(Line::Segment(tip, tip - back - across));
            }
            marks
        }
        CursorBadge::Arrow => {
            let (s, b) = (pen.forward(), h * ARROW_BARB);
            vec![
                Line::Segment(at(-s * h, 0.0), at(s * h, 0.0)),
                Line::Segment(at(s * h, 0.0), at(s * (h - b), -b)),
                Line::Segment(at(s * h, 0.0), at(s * (h - b), b)),
            ]
        }
        CursorBadge::Target => {
            let r = h * TARGET_RING;
            let mut marks = vec![Line::Circle(center, r)];
            marks.extend(axes.map(|d| Line::Segment(center + d * r, center + d * h)));
            marks
        }
        CursorBadge::Continue => {
            let x = pen.forward() * h * CONTINUE_LEAN;
            vec![Line::Segment(at(-x, h), at(x, -h))]
        }
        CursorBadge::Count(mark) => {
            let glyph_at = at(h * COUNT_GLYPH_AT, 0.0);
            let glyph: Vec<Line> = match mark {
                CountMark::Add => plus(glyph_at, h * COUNT_GLYPH).into(),
                CountMark::Delete => vec![minus(glyph_at, h * COUNT_GLYPH)],
                _ => Vec::new(),
            };
            // The numeral alone sits on the centre; with a glyph it moves toward the start to make room.
            let stem = if glyph.is_empty() { 0.0 } else { h * COUNT_STEM };
            let flag = h * COUNT_FLAG;
            let mut marks = vec![Line::Segment(at(stem, -h), at(stem, h)), Line::Segment(at(stem, -h), at(stem - flag, flag - h))];
            marks.extend(glyph);
            marks
        }
        CursorBadge::Corner => {
            // Two edges meeting in a quarter circle round the top-left corner.
            let arc = (0..=CORNER_SEGMENTS).map(|i| {
                let a = PI + FRAC_PI_2 * i as f32 / CORNER_SEGMENTS as f32;
                center + Vec2::new(a.cos(), a.sin()) * h
            });
            vec![Line::Polyline(std::iter::once(at(-h, h)).chain(arc).chain(std::iter::once(at(h, -h))).collect())]
        }
        _ => Vec::new(),
    };
    pen.marks(&marks);
}

/// A badge beside a system pointer arrow.
pub(crate) fn badge_beside(pen: &Pen, kind: CursorBadge) {
    badge(pen, v(BADGE_BESIDE), kind);
}

/// The rotate pointer: an arc with an arrowhead at each end, bent round the corner in direction `toward`.
pub(crate) fn turn(pen: &Pen, toward: Vec2) {
    let len = toward.length();
    if !len.is_finite() || len < f32::EPSILON {
        return;
    }
    let u = toward / len;
    let centre = u * TURN_RADIUS;
    let mid = (-u.y).atan2(-u.x);
    let on_arc = |a: f32| centre + Vec2::new(a.cos(), a.sin()) * TURN_RADIUS;
    let points: Vec<Vec2> = (0..=ARC_SEGMENTS).map(|i| on_arc(mid - HALF_SWEEP + 2.0 * HALF_SWEEP * i as f32 / ARC_SEGMENTS as f32)).collect();
    pen.push(Mark::Path(points.clone(), false, Stroke::new(pen.thin * 3.0, pen.halo)));
    pen.push(Mark::Path(points, false, Stroke::new(pen.thin, pen.fill)));
    let head = TURN_RADIUS * HEAD_PER_RADIUS;
    for (a, sign) in [(mid - HALF_SWEEP, -1.0f32), (mid + HALF_SWEEP, 1.0)] {
        let tangent = Vec2::new(-a.sin(), a.cos()) * sign;
        let normal = Vec2::new(a.cos(), a.sin());
        let tip = on_arc(a) + tangent * HEAD_REACH * head;
        pen.polygon(vec![
            tip + tangent * HEAD_LENGTH * head,
            tip - tangent * HEAD_BACK * head + normal * HEAD_HALF_WIDTH * head,
            tip - tangent * HEAD_BACK * head - normal * HEAD_HALF_WIDTH * head,
        ]);
    }
}

/// The unit vector along `dir`, or `fallback` for a zero or non-finite one.
fn unit(dir: Vec2, fallback: Vec2) -> Vec2 {
    let len = dir.length();
    if len.is_finite() && len > f32::EPSILON { dir / len } else { fallback }
}

/// The free-distort pointer: a dart with its tip on the pointer, pointing along `dir` (up and left for zero).
pub(crate) fn distort(pen: &Pen, dir: Vec2) {
    let u = unit(dir, v([-1.0, -1.0]).normalized());
    let perp = Vec2::new(-u.y, u.x);
    let base = -u * DART_LENGTH;
    let (left, right, notch) = (base + perp * DART_HALF_WIDTH, base - perp * DART_HALF_WIDTH, -u * (DART_LENGTH * DART_NOTCH));
    // Two triangles meeting along the tip and the notch: the dart is not convex.
    for triangle in [vec![Vec2::ZERO, left, notch], vec![Vec2::ZERO, notch, right]] {
        pen.push(Mark::Polygon(triangle, pen.fill, Stroke::NONE));
    }
    pen.push(Mark::Path(vec![Vec2::ZERO, left, notch, right], true, Stroke::new(pen.thin * 2.0, pen.halo)));
}

/// The skew pointer: two opposed arrows beside each other along `dir` (along x for zero).
pub(crate) fn skew(pen: &Pen, dir: Vec2) {
    let u = unit(dir, Vec2::X);
    let perp = Vec2::new(-u.y, u.x);
    for (offset, sign) in [(SKEW_GAP, 1.0f32), (-SKEW_GAP, -1.0)] {
        let centre = perp * offset;
        let end = centre + u * (sign * SKEW_HALF_LENGTH);
        let start = centre - u * (sign * SKEW_HALF_LENGTH);
        pen.push(Mark::Line(start, end, Stroke::new(pen.thin * 3.0, pen.halo)));
        pen.push(Mark::Line(start, end, Stroke::new(pen.thin, pen.fill)));
        pen.polygon(vec![
            end + u * (sign * SKEW_HEAD),
            end - u * (sign * SKEW_BACK) + perp * SKEW_HEAD,
            end - u * (sign * SKEW_BACK) - perp * SKEW_HEAD,
        ]);
    }
}
