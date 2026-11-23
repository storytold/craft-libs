//! Stories of the type-on-a-path markers: an open path, a closed one, text that overflows and locked markers.

use super::{MARK, PathRange, PathTextMarkers, PathTrack};
use craft_behavior::kurbo::{BezPath, Circle, ParamCurve, ParamCurveArclen, ParamCurveNearest, PathSeg, Point, Shape as _, Vec2 as PathVec};
use craft_ui::contract::Show as _;
use craft_ui::testing::canvas_story::{self, OUTLINE, StoryPicture, StoryShape, paint_shapes};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::{Color32, Ui, vec2};

/// The accuracy of arc length and nearest point computations, in document units.
const ACCURACY: f64 = 1e-3;

/// The length of a glyph box along the path and half its height, in document units.
const GLYPH: f64 = 16.0;
const HALF_HEIGHT: f64 = 9.0;

/// The document point at the centre of the stage.
const CENTER: Point = Point::new(260.0, 250.0);

/// A path of a story: a kurbo path measured by arc length.
struct StoryTrack {
    segments: Vec<PathSeg>,
    /// The arc length at the end of each segment.
    ends: Vec<f64>,
    closed: bool,
}

impl StoryTrack {
    fn new(path: &BezPath, closed: bool) -> Self {
        let segments: Vec<PathSeg> = path.segments().collect();
        let mut total = 0.0;
        let ends = segments
            .iter()
            .map(|segment| {
                total += segment.arclen(ACCURACY);
                total
            })
            .collect();
        StoryTrack { segments, ends, closed }
    }

    /// The arc length at the start of segment `i`.
    fn start_of(&self, i: usize) -> f64 {
        i.checked_sub(1).and_then(|previous| self.ends.get(previous)).copied().unwrap_or(0.0)
    }
}

impl PathTrack for StoryTrack {
    fn length(&self) -> f64 {
        self.ends.last().copied().unwrap_or(0.0)
    }

    fn closed(&self) -> bool {
        self.closed
    }

    fn at(&self, s: f64) -> Option<(Point, PathVec)> {
        let length = self.length();
        let s = if self.closed { s.rem_euclid(length) } else { s };
        if !(0.0..=length).contains(&s) {
            return None;
        }
        let i = self.ends.partition_point(|&end| end < s).min(self.segments.len().checked_sub(1)?);
        let segment = self.segments.get(i)?;
        let t = segment.inv_arclen(s - self.start_of(i), ACCURACY);
        let (before, after) = (segment.eval((t - 1e-4).max(0.0)), segment.eval((t + 1e-4).min(1.0)));
        let direction = after - before;
        let norm = direction.hypot();
        Some((segment.eval(t), if norm > 0.0 { direction / norm } else { PathVec::new(1.0, 0.0) }))
    }

    fn project(&self, p: Point, near: Option<f64>) -> Option<f64> {
        let mut best: Option<(f64, f64)> = None;
        for (i, segment) in self.segments.iter().enumerate() {
            let nearest = segment.nearest(p, ACCURACY);
            let arc = self.start_of(i) + segment.subsegment(0.0..nearest.t).arclen(ACCURACY);
            let better = match best {
                None => true,
                Some((distance, at)) => {
                    nearest.distance_sq < distance - 1e-6
                        || ((nearest.distance_sq - distance).abs() <= 1e-6 && near.is_some_and(|n| (arc - n).abs() < (at - n).abs()))
                }
            };
            if better {
                best = Some((nearest.distance_sq, arc));
            }
        }
        best.map(|(_, arc)| arc)
    }
}

/// A wave across the stage.
fn open_track() -> StoryTrack {
    let mut path = BezPath::new();
    path.move_to((80.0, 260.0));
    path.curve_to((180.0, 100.0), (320.0, 420.0), (440.0, 240.0));
    StoryTrack::new(&path, false)
}

/// A circle round the stage's centre.
fn closed_track() -> StoryTrack {
    StoryTrack::new(&Circle::new(CENTER, 140.0).to_path(0.1), true)
}

/// Boxes for glyphs along `range`, as the host's text layout would give them.
fn glyph_boxes(track: &StoryTrack, range: PathRange) -> Vec<[Point; 4]> {
    let mut boxes = Vec::new();
    let mut s = range.start;
    while s + GLYPH <= range.end {
        if let (Some((a, tangent)), Some((b, _))) = (track.at(s), track.at(s + GLYPH)) {
            let normal = PathVec::new(-tangent.y, tangent.x) * HALF_HEIGHT;
            boxes.push([a - normal, b - normal, b + normal, a + normal]);
        }
        s += GLYPH;
    }
    boxes
}

/// Shows the markers `configure` builds over the story's range as the only canvas control of a canvas whose document
/// is a picture of the path.
fn on_canvas(
    ui: &mut Ui,
    data: &mut StoryData,
    track: &StoryTrack,
    initial: PathRange,
    configure: impl for<'a> Fn(PathTextMarkers<'a>) -> PathTextMarkers<'a>,
) -> StoryOutput {
    const STEPS: usize = 120;
    let (range, range_at) = data.value_at("range", initial);
    let boxes = glyph_boxes(track, *range);
    let length = track.length();
    let mut path = BezPath::new();
    for (i, point) in (0..=STEPS).filter_map(|i| track.at(length * i as f64 / STEPS as f64)).map(|(p, _)| p).enumerate() {
        if i == 0 { path.move_to(point) } else { path.line_to(point) }
    }
    let colour = canvas_story::accent(ui, 1.0);
    let shapes = [StoryShape::line(&path, OUTLINE, colour)];
    let picture = StoryPicture::new(2 * CENTER.x as u32, 2 * CENTER.y as u32, move |p| paint_shapes(&shapes, Color32::TRANSPARENT, p));
    canvas_story::on_picture(ui, &picture, |ui, at, _| {
        StoryOutput::from(configure(PathTextMarkers::new("Text on path", range_at.bind(&mut *range), track).body(&boxes)).show_at(ui, at).range)
    })
}

#[story(PathTextMarkers<'static>, covers = [Default], drive = Drive::Drag(Grip::part(MARK.at(1)).toward(vec2(1.0, 1.0))))]
fn open_path(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let track = open_track();
    let length = track.length();
    on_canvas(ui, data, &track, PathRange { start: 0.15 * length, end: 0.8 * length, flip: false }, |markers| markers)
}

#[story(PathTextMarkers<'static>, covers = [Default])]
fn closed_path(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let track = closed_track();
    let length = track.length();
    on_canvas(ui, data, &track, PathRange { start: 0.1 * length, end: 0.75 * length, flip: false }, |markers| markers)
}

#[story(PathTextMarkers<'static>, covers = [Default])]
fn overflow(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let track = open_track();
    let length = track.length();
    on_canvas(ui, data, &track, PathRange { start: 0.1 * length, end: 0.5 * length, flip: false }, |markers| markers.overflow(true))
}

#[story(PathTextMarkers<'static>, covers = [Default])]
fn locked(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let track = open_track();
    let length = track.length();
    on_canvas(ui, data, &track, PathRange { start: 0.15 * length, end: 0.8 * length, flip: false }, |markers| markers.locked(true))
}
