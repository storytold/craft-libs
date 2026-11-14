//! Stories of the brush overlays: each control on a canvas with a view, as a freehand tool shows it, and a brush run
//! live by the tool story host: its cursor follows the story's tip and cursor preferences, and Ctrl + Alt + drag or
//! Alt + right-drag resizes it with the falloff preview; Play runs either drag.

use super::{
    BrushCursor, BrushCursorOptions, BrushDimensions, BrushResize, BrushResizeOptions, BrushTip, BrushTipOutline, FalloffPreview, PaintingCursor,
    TipCell, TipFalloff, TipPose, TipRings,
};
use crate::canvas::testing::canvas_story::{StoryDocument, on_canvas_view, story_view};
use crate::canvas::testing::tool_story::{ToolScript, ToolStory, tool_story};
use crate::canvas::{CanvasTool, ToolAction, ToolContext, ToolPath, ToolPointer};
use crate::canvas_controls::CanvasControlWords;
use crate::contract::Show as _;
use crate::primitives::Draw;
use crate::runtime::env::Env;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::kurbo::{Point, Rect as DocRect, Vec2};
use craft_behavior::{CancelReason, CanvasView, Cursor, PaintedCursor, PointerButton};
use craft_keymap::Mods;
use egui::{Rect, Ui};

/// The zoom, in percent of one device pixel per document pixel.
const ZOOM: f64 = 400.0;
/// The document point at the middle of the canvas.
const CENTRE: Point = Point::new(100.0, 100.0);
/// The radius of the soft tip, in document units.
const RADIUS: f64 = 40.0;
/// The hardness of the soft tip.
const SOFT: f64 = 0.4;
/// The live brush starts at this diameter and hardness, in document units and a share.
const START: BrushDimensions = BrushDimensions::new(80.0, 0.5);
/// The largest diameter the resize accepts, in document units.
const MAX_DIAMETER: f64 = 400.0;
/// The thin ellipse tip's height over its width.
const THIN: f64 = 0.25;
/// The star tip's inner points, in radius units.
const STAR_INNER: f32 = 0.4;
/// The aliased tip's cell: its corners in document units.
const CELL: DocRect = DocRect::new(96.0, 96.0, 104.0, 104.0);

/// The view of a freehand canvas over `room` that shows `CENTRE` in its middle.
fn brush_view(room: Rect, pixels_per_point: f32) -> CanvasView {
    story_view(room, pixels_per_point, CENTRE, ZOOM)
}

#[story(FalloffPreview<'static>, covers = [Default])]
fn soft_tip(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let falloff = TipFalloff::from_hardness(RADIUS, SOFT);
    on_canvas_view(ui, brush_view, Some(&StoryDocument::STANDARD), |ui, rect, _| FalloffPreview::new(&falloff, CENTRE, RADIUS).show_at(ui, rect))
}

#[story(FalloffPreview<'static>, covers = [Default])]
fn hard_tip(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let falloff = TipFalloff::hard(RADIUS);
    on_canvas_view(ui, brush_view, Some(&StoryDocument::STANDARD), |ui, rect, _| FalloffPreview::new(&falloff, CENTRE, RADIUS).show_at(ui, rect))
}

#[story(TipCell, covers = [Default])]
fn square_cell(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas_view(ui, brush_view, Some(&StoryDocument::STANDARD), |ui, rect, _| TipCell::new(CELL, false).show_at(ui, rect))
}

#[story(TipCell, covers = [Default])]
fn round_cell(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas_view(ui, brush_view, Some(&StoryDocument::STANDARD), |ui, rect, _| TipCell::new(CELL, true).show_at(ui, rect))
}

craft_ui::words! {
    /// The readout of the live brush story.
    #[non_exhaustive]
    pub(crate) struct BrushStoryWords {
        /// The brush's values: `{diameter}` in document pixels, `{level}` the hardness in percent.
        values: template "brushStory.values" = "Diameter {diameter} px, hardness {level} %";
    }
}

/// The tips the live brush story offers.
#[derive(Clone, Copy, Debug, PartialEq)]
enum StoryTip {
    Round,
    Star,
    ThinEllipse,
    Pixel,
}

/// The brush the story keeps: its resize gesture, whether a stroke is being painted, and where the pointer is.
#[derive(Default)]
struct StoryBrush {
    resize: BrushResize,
    painting: bool,
    pointer: Option<Point>,
}

/// The cursor preferences the story sets.
#[derive(Clone, Copy)]
struct Look {
    tip: StoryTip,
    options: BrushCursorOptions,
    centre: bool,
    second_ring: bool,
}

/// A five-pointed star of radius 1 with its inner points at `inner`, point up.
fn star(inner: f32) -> TipRings {
    let ring = (0..10)
        .map(|i| {
            let angle = i as f32 * std::f32::consts::PI / 5.0 - std::f32::consts::FRAC_PI_2;
            let radius = if i % 2 == 0 { 1.0 } else { inner };
            [radius * angle.cos(), radius * angle.sin()]
        })
        .collect();
    TipRings::new(vec![ring])
}

/// The falloff of a round tip of `dimensions`.
fn falloff(dimensions: BrushDimensions) -> TipFalloff {
    TipFalloff::from_hardness(dimensions.diameter / 2.0, dimensions.level)
}

/// The tip `tip` of `dimensions` with its pose; a pixel tip fills the whole-pixel cell at `pointer`.
fn tip_of(tip: StoryTip, dimensions: BrushDimensions, pointer: Option<Point>) -> (BrushTip, TipPose) {
    let full = dimensions.diameter / 2.0;
    let edge = falloff(dimensions);
    let normal = (edge.core() + edge.reach()) / 2.0;
    match tip {
        StoryTip::Round => (BrushTip::round(dimensions.diameter, normal), TipPose::upright()),
        StoryTip::ThinEllipse => (BrushTip::round(dimensions.diameter, normal), TipPose::new(Vec2::from((1.0, 0.0)), Vec2::from((0.0, THIN)))),
        StoryTip::Star => {
            let share = (normal / full.max(f64::EPSILON)) as f32;
            let scaled =
                TipRings::new(star(STAR_INNER).rings().iter().map(|ring| ring.iter().map(|[x, y]| [x * share, y * share]).collect()).collect());
            (BrushTip::new(dimensions.diameter, BrushTipOutline::traced(scaled, star(STAR_INNER))), TipPose::upright())
        }
        StoryTip::Pixel => {
            let side = dimensions.diameter.round().max(1.0);
            let at = pointer.unwrap_or(CENTRE);
            let (x0, y0) = ((at.x - side / 2.0).round(), (at.y - side / 2.0).round());
            (BrushTip::new(dimensions.diameter, BrushTipOutline::pixels(DocRect::new(x0, y0, x0 + side, y0 + side), false)), TipPose::upright())
        }
    }
}

/// The brush for one pass: the kept brush, the story's values and the tip they make.
struct BrushPass<'a> {
    brush: &'a mut StoryBrush,
    dimensions: &'a mut BrushDimensions,
    look: Look,
    tip: BrushTip,
    pose: TipPose,
}

impl CanvasTool for BrushPass<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(space) = cx.space() else { return ToolAction::Ignored };
        self.brush.pointer = Some(space.view.to_doc(pointer.pos));
        if BrushResize::chord(pointer.button, pointer.mods) {
            if self.brush.resize.begin(&space.view, pointer.pos, *self.dimensions, &BrushResizeOptions::new(MAX_DIAMETER)).is_none() {
                return ToolAction::Ignored;
            }
        } else {
            self.brush.painting = true;
        }
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(space) = cx.space() else { return };
        if !self.brush.resize.active() {
            self.brush.pointer = Some(space.view.to_doc(pointer.pos));
            return;
        }
        if let Some(step) = self.brush.resize.track(&space.view, pointer.pos) {
            *self.dimensions = step.dimensions;
        }
        let rows = self.brush.resize.readout_rows(&cx.units(), &cx.words::<CanvasControlWords>());
        cx.readout(rows);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.brush.painting = false;
        let Some(space) = cx.space() else { return };
        if let Some(step) = self.brush.resize.finish(&space.view, pointer.pos) {
            *self.dimensions = step.dimensions;
        }
    }

    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.brush.pointer = cx.space().map(|space| space.view.to_doc(pointer.pos));
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {
        self.brush.painting = false;
        if let Some(step) = self.brush.resize.cancel() {
            *self.dimensions = step.dimensions;
        }
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        let Some(space) = cx.space().filter(|_| !self.brush.resize.active()) else { return Cursor::Painted(PaintedCursor::Hidden) };
        let mut cursor = BrushCursor::new(&self.tip).pose(self.pose).centre(self.look.centre).painting(self.brush.painting);
        if self.look.second_ring {
            cursor = cursor.inner_ring(falloff(*self.dimensions).core());
        }
        cursor.cursor(cx, &self.look.options, &space.view)
    }

    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        BrushResize::takes(button, mods)
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("a stroke follows the pointer; the bracket keys set the size and the hardness")
    }
}

#[story(FalloffPreview<'static>, covers = [], drive = Drive::Never)]
fn brush(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tips = [("Round", StoryTip::Round), ("Star", StoryTip::Star), ("Thin ellipse", StoryTip::ThinEllipse), ("Pixel", StoryTip::Pixel)];
    let tip = *data.choice("tip", StoryTip::Round, &tips);
    let cursors = [
        ("Normal Brush Tip", PaintingCursor::NormalBrushTip),
        ("Full Size Brush Tip", PaintingCursor::FullSizeBrushTip),
        ("Precise", PaintingCursor::Precise),
        ("Standard", PaintingCursor::Standard),
    ];
    let painting = *data.choice("cursor", PaintingCursor::NormalBrushTip, &cursors);
    let options = BrushCursorOptions::new(painting).show_only_crosshair_while_painting(*data.value("crosshair while painting", false));
    let look = Look { tip, options, centre: *data.value("centre", false), second_ring: *data.value("second ring", false) };
    let rotation = *data.ranged("rotation", 0.0_f64, -180.0..=180.0);
    let flip = *data.value("flip", false);
    let aspect = *data.ranged("aspect", 1.0_f64, 0.5..=2.0);
    let words = Env::of(ui).words::<BrushStoryWords>();
    let drags = [("Ctrl + Alt + drag", PointerButton::Primary), ("Alt + right-drag", PointerButton::Secondary)];
    let button = *data.choice("play", PointerButton::Primary, &drags);
    let seconds = *data.ranged("play seconds", 2.0_f64, 0.5..=10.0);
    let document = StoryDocument::STANDARD;
    let held = if button == PointerButton::Primary { Mods { ctrl: true, ..Mods::ALT } } else { Mods::ALT };
    let (from, rect) = (document.centre(), document.rect());
    // The old story's drag: right and down by a share of the document.
    let to = from + Vec2::new(rect.width() * 0.15, rect.height() * 0.15);
    let script = ToolScript::new(seconds).stroke(button, &[from, to], held, held);
    let story = ToolStory::new("Document", &document).turned(rotation, flip, aspect).play(script);
    let body = |ui: &mut Ui, at: Rect, _: &CanvasView, brush: &mut StoryBrush, dimensions: &BrushDimensions| {
        let edge = falloff(*dimensions);
        if let Some(preview) = brush.resize.preview(&edge) {
            let _ = preview.show_at(ui, at);
        } else if let Some(cell) = BrushCursor::new(&tip_of(look.tip, *dimensions, brush.pointer).0).tip_cell().filter(|_| brush.pointer.is_some()) {
            let _ = cell.show_at(ui, at);
        }
    };
    tool_story(ui, data, story, StoryBrush::default, START, body, |run| {
        let (tip, pose) = tip_of(look.tip, *run.document, run.tool.pointer);
        let _ = run.host.run(&mut BrushPass { brush: &mut *run.tool, dimensions: &mut *run.document, look, tip, pose });
        let (diameter, level) = (format!("{:.0}", run.document.diameter), format!("{:.0}", run.document.level * 100.0));
        StoryOutput::default().readout(craft_i18n::fmt(words.values.as_str(), &[("diameter", &diameter), ("level", &level)]))
    })
}
