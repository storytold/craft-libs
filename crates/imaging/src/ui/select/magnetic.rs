//! [`MagneticLassoTool`]: an outline that snaps to the edges the app's [`EdgeTracer`] finds.

use craft_behavior::geometry::SelectionOperation;
use craft_behavior::{CancelReason, Cursor, PaintedCursor};
use craft_keymap::{Key, Mods};
use craft_ui::canvas::{CanvasTool, ToolAccess, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::primitives::{Draw, PaintShape, StrokeWidth, ToolIcon};
use egui::{CornerRadius, Pos2, Rect as ScreenRect, Vec2};
use kurbo::Point;
use serde::{Deserialize, Serialize};

use super::lasso::{DraftKey, draft_command, outline_access, reach};
use super::{Place, Report, SelectEvent, SelectPhase, SelectToolPass, SelectionProbe, SelectionShape};

/// Where the magnetic lasso cursor's icon touches the document: the icon's own origin.
const ICON_HOTSPOT: Vec2 = Vec2::ZERO;

/// The detection settings of one edge query.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct EdgeSearch {
    /// How far from the pointer edges are looked for, in document pixels (Width, narrowed by
    /// stylus pressure when [`MagneticSettings::pressure_width`] is on).
    pub width: f64,
    /// The weakest edge that counts, 0..=1 (Contrast / 100).
    pub contrast: f64,
}

impl EdgeSearch {
    /// A search for edges within `width` document pixels of the pointer, at least `contrast` (0..=1) strong.
    pub fn new(width: f64, contrast: f64) -> Self {
        Self { width, contrast }
    }
}

/// The app's edge finder over the pixels being traced (the active layer's pixels or the composite). The tool calls it
/// on the UI thread; the app refreshes its snapshot when the document or the layer changes, between outlines.
pub trait EdgeTracer {
    /// The edge point near `at` within `search.width`, or `None` for none (the tool uses `at`).
    fn snap(&mut self, at: Point, search: EdgeSearch) -> Option<Point>;

    /// The path along the edges from `from` to `to`, both included, following the pointer's
    /// recent `guide` points. Empty when it finds none: the tool draws a straight segment.
    fn trace(&mut self, from: Point, to: Point, guide: &[Point], search: EdgeSearch) -> Vec<Point>;
}

/// The Magnetic Lasso's options bar, persisted by the app (decisions §2.4). The options bar edits
/// [`MagneticLassoTool::settings_mut`]; `[` and `]` change `width` in the tool.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[non_exhaustive]
pub struct MagneticSettings {
    /// Width in document pixels, 1..=256. Default 10.
    pub width: f32,
    /// Contrast in percent, 1..=100. Default 10.
    pub contrast: f32,
    /// Frequency 0..=100: how often points fasten by themselves. Default 57.
    pub frequency: f32,
    /// Stylus Pressure: pressing harder narrows the width (a pen only). Default off.
    pub pressure_width: bool,
}

impl Default for MagneticSettings {
    fn default() -> Self {
        Self { width: 10.0, contrast: 10.0, frequency: 57.0, pressure_width: false }
    }
}

impl MagneticSettings {
    /// The range of `width`.
    pub const WIDTH_RANGE: (f32, f32) = (1.0, 256.0);

    /// How far the outline runs before a point fastens by itself, in document pixels at `zoom`
    /// screen points per pixel: `(8 + (100 − frequency) · 0.9) / zoom` (8 points at 100, 98 at 0,
    /// 47 at 57). A frequency that is not a number counts as 57; `zoom` at least 0.01.
    pub fn spacing(&self, zoom: f64) -> f64 {
        let frequency = if self.frequency.is_finite() { f64::from(self.frequency.clamp(0.0, 100.0)) } else { 57.0 };
        (8.0 + (100.0 - frequency) * 0.9) / zoom.max(0.01)
    }

    /// The search for one query with pen `pressure` (`None` for a mouse): Width, narrowed to
    /// `1 + (width − 1) · (1 − pressure)` when [`pressure_width`](Self::pressure_width) is on and a
    /// pen sample carries a pressure; Contrast / 100.
    pub fn search(&self, pressure: Option<f32>) -> EdgeSearch {
        let mut width = f64::from(self.width);
        if self.pressure_width
            && let Some(pressure) = pressure.filter(|p| p.is_finite())
        {
            width = 1.0 + (width - 1.0) * f64::from(1.0 - pressure.clamp(0.0, 1.0));
        }
        EdgeSearch { width, contrast: f64::from(self.contrast) / 100.0 }
    }
}

/// The outline being drawn.
#[derive(Clone, Debug)]
struct MagneticOutline {
    /// The fastened outline, from the first fastening point to the last.
    path: Vec<Point>,
    /// Indices into `path` of the fastening points (the first is 0).
    anchors: Vec<usize>,
    /// The live segment from the last fastening point to the edge near the pointer.
    live: Vec<Point>,
    /// Where the pointer has been since the last fastening point.
    trail: Vec<Point>,
    /// The operation the first click latched.
    operation: SelectionOperation,
    /// The document the outline is drawn on.
    document: u64,
    /// An ⌥-drag is drawing a freehand segment.
    freehand: bool,
    /// The button is down (a drag follows the edges or draws freehand).
    down: bool,
    /// The pointer's last position (the trail ends at a fastened point instead after one fastens).
    pointer: Point,
}

impl MagneticOutline {
    /// The outline and the live segment after its start: what is drawn and counted.
    fn points(&self) -> impl Iterator<Item = Point> + '_ {
        self.path.iter().chain(self.live.iter().skip(1)).copied()
    }

    /// The pointer's position `q` joins the trail, keeping its points `gap` apart.
    fn extend_trail(&mut self, q: Point, width: f64) {
        let gap = (width / 4.0).clamp(1.0, 8.0);
        match self.trail.len() {
            n if n >= 2 && self.trail[n - 2].distance(q) < gap => self.trail[n - 1] = q,
            _ => self.trail.push(q),
        }
    }

    /// The live segment fastens: its end becomes a fastening point.
    fn fasten_live(&mut self) {
        let live = std::mem::take(&mut self.live);
        let before = self.path.len();
        for q in live.into_iter().skip(1) {
            if self.path.last().is_none_or(|l| l.distance(q) > 1e-9) {
                self.path.push(q);
            }
        }
        if self.path.len() > before {
            self.anchors.push(self.path.len() - 1);
        }
        if let Some(&end) = self.path.last() {
            self.trail = vec![end];
        }
    }

    /// An ⌥-drag ends: where it stopped is a fastening point.
    fn end_freehand(&mut self) {
        if std::mem::take(&mut self.freehand) && self.anchors.last().is_some_and(|&a| a + 1 < self.path.len()) {
            self.anchors.push(self.path.len() - 1);
        }
    }

    /// `p` is within `radius` of the first point of an outline long enough to close.
    fn near_start(&self, p: Point, radius: f64) -> bool {
        self.path.first().is_some_and(|&first| first.distance(p) <= radius) && length(&self.path) + length(&self.live) > 2.0 * radius
    }
}

/// The length of the polyline `p`.
fn length(p: &[Point]) -> f64 {
    p.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// The polyline `p` up to `len` along it (ending at that point) and the point, when more than a
/// pixel of it lies beyond.
fn cut(p: &[Point], len: f64) -> Option<(Vec<Point>, Point)> {
    if length(p) < len + 1.0 {
        return None;
    }
    let mut head = Vec::new();
    let mut acc = 0.0;
    for w in p.windows(2) {
        let (a, b) = (w[0], w[1]);
        head.push(a);
        let d = a.distance(b);
        if acc + d >= len && d > 0.0 {
            let point = a.lerp(b, (len - acc) / d);
            head.push(point);
            return Some((head, point));
        }
        acc += d;
    }
    None
}

/// The pen's pressure for `pointer`, `None` for a mouse (pressure 1, no tilt, no rotation).
///
/// A pen that reports full pressure and no tilt reads as a mouse; the width then stays as set, which is also what a
/// mouse gets.
fn pen_pressure(pointer: &ToolPointer) -> Option<f32> {
    let sample = pointer.sample;
    let pen = sample.pressure != 1.0 || sample.tilt != [0.0; 2] || sample.rotation != 0.0 || sample.eraser;
    pen.then_some(sample.pressure)
}

/// The Magnetic Lasso: a selection outline that snaps to edges as the pointer moves.
///
/// Pointer (document units through the host; the tool takes button-up moves too):
/// - The first click sets the first fastening point at the edge near the pointer
///   ([`EdgeTracer::snap`]) and latches the operation.
/// - Moving the pointer (button up or down) traces the live segment from the last fastening point
///   to the edge near the pointer ([`EdgeTracer::trace`], guided by the pointer's trail); points
///   fasten by themselves every [`MagneticSettings::spacing`] along it. A jump is walked in steps
///   of `spacing / 2` (at most 256) so points keep fastening along it.
/// - A click fastens a point; ⌥-click adds a straight segment; ⌥-drag draws freehand. Pressing ⌥
///   during a drag anchors a point at the pointer and draws freehand from there; releasing it goes
///   back to the edges.
/// - A click within the close / zoom of the first point (once the outline is longer
///   than twice that) closes along the edges (⌥ straight); so does a double-click, and Enter
///   (⌥ Enter straight).
///
/// Keys: Backspace or Delete remove the last fastening point (the first cancels); Esc cancels
/// (during a press too: the host's `cancel(Escape)` drops the outline, any other reason keeps it); `[`
/// and `]` change the width by 1 px within 1..=256. The tool keeps Backspace, Delete and Enter from
/// the app's shortcuts while an outline is open.
///
/// The closed outline is [`SelectEvent::Commit`] with a `SelectionShape::Polygon`: the selection is exactly the outline drawn. An
/// outline of fewer than 3 points commits nothing. A document change or [`reset`](Self::reset)
/// drops it. No selection drag: a press inside the selection draws.
///
/// Drawn: the outline as ants, a square with an edge at each fastening point, and over the first point the closing
/// ring beside the pointer. The cursor is the lasso icon; with Other Cursors > Precise
/// ([`precise_cursor`](Self::precise_cursor)) or Caps Lock it is the width circle.
#[derive(Clone, Debug)]
pub struct MagneticLassoTool<T> {
    tracer: T,
    settings: MagneticSettings,
    bar: SelectionOperation,
    outline: Option<MagneticOutline>,
    /// The modifiers of the last pass, to see Alt pressed or released during a drag.
    mods: Mods,
    /// Other Cursors › Precise: the width circle shows round the crosshair.
    precise: bool,
    report: Report,
}

impl<T: EdgeTracer> MagneticLassoTool<T> {
    /// A Magnetic Lasso over `tracer`, with the default settings.
    pub fn new(tracer: T) -> Self {
        Self {
            tracer,
            settings: MagneticSettings::default(),
            bar: SelectionOperation::New,
            outline: None,
            mods: Mods::NONE,
            precise: false,
            report: Report::default(),
        }
    }

    /// The app's edge finder, to refresh its snapshot.
    pub fn tracer_mut(&mut self) -> &mut T {
        &mut self.tracer
    }

    /// The options bar's values.
    pub fn settings(&self) -> &MagneticSettings {
        &self.settings
    }

    /// The options bar's values, to edit (and to restore the persisted ones).
    pub fn settings_mut(&mut self) -> &mut MagneticSettings {
        &mut self.settings
    }

    /// The options bar's operation for outlines begun from now on.
    pub fn operation(&mut self, bar: SelectionOperation) -> &mut Self {
        self.bar = bar;
        self
    }

    /// Preferences › Cursors › Other Cursors is Precise (`CursorOptions::other_precise`): the
    /// width circle shows round the crosshair.
    pub fn precise_cursor(&mut self, precise: bool) -> &mut Self {
        self.precise = precise;
        self
    }

    /// The tool as the `CanvasTool` for one pass, asking `probe` about the document.
    pub fn pass<'a>(&'a mut self, probe: &'a dyn SelectionProbe) -> SelectToolPass<'a, Self> {
        SelectToolPass::new(self, probe)
    }

    /// The events since the last call, in order.
    pub fn take_events(&mut self) -> Vec<SelectEvent> {
        self.report.take()
    }

    /// Drop the outline without an event (a tool switch).
    pub fn reset(&mut self) {
        self.outline = None;
    }

    /// What the tool is doing: an outline with its point count.
    pub fn phase(&self) -> SelectPhase {
        self.outline.as_ref().map_or(SelectPhase::Idle, |o| SelectPhase::Outline { points: o.points().count() })
    }

    /// An outline with the operation New is open: the app hides the selection's ants.
    pub fn replaces_selection(&self) -> bool {
        self.outline.as_ref().is_some_and(|o| o.operation == SelectionOperation::New)
    }

    /// Close the outline: the live segment fastens, then the last point joins the first along the
    /// edges, or with a straight segment with `straight`; `Commit` when 3 or more points.
    fn close(&mut self, straight: bool) {
        let Some(mut o) = self.outline.take() else { return };
        o.end_freehand();
        o.fasten_live();
        if !straight && let (Some(&first), Some(&last)) = (o.path.first(), o.path.last()) {
            let mut closing = self.tracer.trace(last, first, &[], self.settings.search(None));
            // The outline ends where it starts.
            closing.pop();
            o.path.extend(closing.into_iter().skip(1));
        }
        if o.path.len() >= 3 {
            self.report.push(SelectEvent::Commit { shape: SelectionShape::Polygon(o.path), operation: o.operation });
        }
    }

    /// Remove the last fastening point and the outline back to the one before; with only the first
    /// left, cancel. The live segment is traced to the pointer again without fastening.
    fn remove_last_point(&mut self) {
        let Some(o) = self.outline.as_mut() else { return };
        o.end_freehand();
        if o.anchors.len() <= 1 {
            self.outline = None;
            return;
        }
        o.anchors.pop();
        let keep = o.anchors.last().map_or(1, |&a| a + 1);
        o.path.truncate(keep);
        o.live.clear();
        let pointer = o.trail.last().copied();
        o.trail = o.path.last().copied().into_iter().collect();
        // The live segment runs from the earlier point to the pointer again; it fastens only when
        // the pointer moves on, so Backspace can keep removing points.
        if let Some(p) = pointer {
            self.follow(p, self.settings.search(None), 0.0, false);
        }
    }

    /// The pointer is at `q`: trace the live segment to the edge near it and, with `fasten`, fasten
    /// points behind the pointer while the live segment is longer than the spacing (at most 64 per
    /// call).
    fn follow(&mut self, q: Point, search: EdgeSearch, spacing: f64, fasten: bool) {
        let Self { tracer, outline, .. } = self;
        let Some(o) = outline.as_mut() else { return };
        o.extend_trail(q, search.width);
        let target = tracer.snap(q, search).unwrap_or(q);
        for _ in 0..64 {
            let Some(&from) = o.path.last() else { return };
            let mut live = tracer.trace(from, target, &o.trail, search);
            if live.is_empty() {
                live = vec![from, target];
            }
            // The part of the outline `spacing` behind its start no longer changes as the pointer
            // moves: it fastens.
            let Some((head, point)) = fasten.then(|| cut(&live, spacing)).flatten() else {
                o.live = live;
                return;
            };
            o.path.extend(head.into_iter().skip(1));
            o.anchors.push(o.path.len() - 1);
            // Edges are looked for along the pointer's path from where it passed the new point.
            let near = o.trail.iter().enumerate().min_by(|a, b| a.1.distance(point).total_cmp(&b.1.distance(point))).map_or(0, |(i, _)| i);
            let rest: Vec<Point> = o.trail.iter().skip(near + 1).copied().collect();
            o.trail = std::iter::once(point).chain(rest).collect();
        }
    }

    /// The pointer moved to `p` (button up or down): freehand adds points, otherwise the outline
    /// follows the edges, walking a long jump in steps.
    fn moved(&mut self, pointer: &ToolPointer, p: Point, zoom: f64) {
        let Some(o) = self.outline.as_mut() else { return };
        o.pointer = p;
        if o.freehand {
            // ⌥-drag: the pointer's path is the outline.
            if o.path.last().is_none_or(|q| q.distance(p) >= 0.5) {
                o.path.push(p);
            }
            o.trail = vec![p];
            return;
        }
        let search = self.settings.search(pen_pressure(pointer));
        let spacing = self.settings.spacing(zoom);
        let step = (spacing / 2.0).max(1.0);
        let last = o.trail.last().copied().unwrap_or(p);
        // A long jump (a fast flick, automation) is walked in steps, so points keep fastening
        // along it.
        let n = ((last.distance(p) / step).ceil() as usize).clamp(1, 256);
        for k in 1..=n {
            self.follow(last.lerp(p, k as f64 / n as f64), search, spacing, true);
        }
    }

    /// The button went up or the gesture was cancelled: an ⌥-drag ends.
    fn end_gesture(&mut self) {
        if let Some(o) = self.outline.as_mut() {
            o.down = false;
            o.end_freehand();
        }
    }
}

impl<T: EdgeTracer> CanvasTool for SelectToolPass<'_, MagneticLassoTool<T>> {
    /// Drops an outline begun on another document; keeps the keys while an outline is open.
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        let (tool, probe) = (&mut *self.tool, self.probe);
        if tool.outline.as_ref().is_some_and(|o| o.document != probe.document()) {
            tool.outline = None;
        }
        let before = std::mem::replace(&mut tool.mods, cx.mods);
        if let Some(o) = tool.outline.as_mut()
            && o.down
            && before.alt != cx.mods.alt
        {
            if cx.mods.alt {
                // Alt pressed during the drag: the pointer's last place anchors, freehand goes on from it.
                let p = o.pointer;
                o.fasten_live();
                if o.path.last().is_none_or(|q| q.distance(p) > 1e-9) {
                    o.path.push(p);
                    o.anchors.push(o.path.len() - 1);
                }
                o.trail = vec![p];
                o.freehand = true;
            } else {
                // Alt released: the edges again, from where the freehand stopped.
                o.end_freehand();
                o.live.clear();
                o.trail = o.path.last().copied().into_iter().collect();
            }
        }
    }

    /// Between presses only: a kept Escape would reach the tool as a chord instead of cancelling the press.
    fn claims(&self) -> ToolClaims {
        if self.tool.outline.as_ref().is_some_and(|o| !o.down) { ToolClaims::NONE.keyboard().deletes() } else { ToolClaims::NONE }
    }

    /// The width circle with Other Cursors > Precise or Caps Lock (`width * zoom`, at least the slop); otherwise the
    /// lasso icon with the operation's badge.
    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        let tool = &*self.tool;
        if (tool.precise || cx.caps_lock)
            && let Some(place) = Place::of(cx)
        {
            let (min, max) = MagneticSettings::WIDTH_RANGE;
            let radius = (f64::from(tool.settings.width.clamp(min, max)) * place.zoom()) as f32;
            return PaintedCursor::circle(radius.max(cx.slop)).into();
        }
        let badge = match &tool.outline {
            Some(o) => o.operation.badge(),
            None if cx.mods.cmd => None,
            None => SelectionOperation::resolve(tool.bar, cx.mods, self.probe.has_selection()).badge(),
        };
        PaintedCursor::tool(ToolIcon::MagneticLasso.icon(), ICON_HOTSPOT, badge).into()
    }

    /// The first click starts an outline; later ones close near the first point, add a straight segment with Alt (and
    /// freehand while dragging), or fasten a point; a double-click closes.
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let (tool, probe) = (&mut *self.tool, self.probe);
        let Some(place) = Place::of(cx) else { return ToolAction::Ignored };
        let search = tool.settings.search(pen_pressure(pointer));
        let p = place.doc(pointer.pos);
        let Some(o) = tool.outline.as_mut() else {
            // Cmd makes it the Move tool (the app's rule).
            if pointer.mods.cmd {
                return ToolAction::Ignored;
            }
            let start = tool.tracer.snap(p, search).unwrap_or(p);
            tool.outline = Some(MagneticOutline {
                path: vec![start],
                anchors: vec![0],
                live: Vec::new(),
                trail: vec![p],
                operation: SelectionOperation::resolve(tool.bar, pointer.mods, probe.has_selection()),
                document: probe.document(),
                freehand: false,
                down: true,
                pointer: p,
            });
            return ToolAction::Claimed;
        };
        if pointer.clicks >= 2 || o.near_start(p, close_radius(cx, &place)) {
            tool.close(pointer.mods.alt);
            return ToolAction::Claimed;
        }
        o.down = true;
        o.pointer = p;
        if pointer.mods.alt {
            // A straight segment to here; dragging on draws freehand.
            if o.path.last().is_none_or(|q| q.distance(p) > 1e-9) {
                o.path.push(p);
                o.anchors.push(o.path.len() - 1);
            }
            o.live.clear();
            o.trail = vec![p];
            o.freehand = true;
        } else {
            // A click fastens a point at the edge near the pointer.
            tool.follow(p, search, 0.0, false);
            if let Some(o) = tool.outline.as_mut() {
                o.fasten_live();
            }
        }
        ToolAction::Claimed
    }

    /// An Alt drag adds freehand points (0.5 px apart); otherwise the outline follows the edges.
    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if let Some(place) = Place::of(cx) {
            self.tool.moved(pointer, place.doc(pointer.pos), place.zoom());
        }
    }

    /// Ends an Alt drag: where it stopped fastens.
    fn release(&mut self, _pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {
        self.tool.end_gesture();
    }

    /// Button-up moves follow the edges too.
    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if self.tool.outline.is_some()
            && let Some(place) = Place::of(cx)
        {
            self.tool.moved(pointer, place.doc(pointer.pos), place.zoom());
            cx.request_repaint();
        }
    }

    /// Escape during a press drops the whole outline, as Escape between clicks does. A lost pointer or a
    /// `ToolHost::cancel` for another reason ends an Alt drag as `release` would and keeps the outline (the app calls
    /// [`reset`](MagneticLassoTool::reset) to drop it).
    fn cancel(&mut self, reason: CancelReason, _cx: &mut ToolContext<'_>) {
        self.tool.end_gesture();
        if reason == CancelReason::Escape {
            self.tool.outline = None;
        }
    }

    /// Enter (Alt+Enter straight), Escape, Backspace and Delete while an outline is open; `[` and `]` change the width
    /// (the tool keeps them only while an outline is open).
    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        let tool = &mut *self.tool;
        if tool.outline.is_some()
            && let Some((command, mods)) = draft_command(key, cx)
        {
            match command {
                DraftKey::Close if mods.is_none() || mods == Mods::ALT => {
                    tool.close(mods.alt);
                    return true;
                }
                DraftKey::Cancel => {
                    tool.outline = None;
                    return true;
                }
                DraftKey::Retract if mods.is_none() || mods == Mods::ALT => {
                    tool.remove_last_point();
                    return true;
                }
                _ => {}
            }
        }
        let step = match key {
            ToolKey::Kept(chord) if chord.mods.is_none() && chord.key == Key::OpenBracket => -1.0,
            ToolKey::Kept(chord) if chord.mods.is_none() && chord.key == Key::CloseBracket => 1.0,
            _ => return false,
        };
        let (min, max) = MagneticSettings::WIDTH_RANGE;
        let width = &mut tool.settings.width;
        *width = if width.is_finite() { (width.round() + step).clamp(min, max) } else { MagneticSettings::default().width };
        true
    }

    /// The outline as ants, a square at each fastening point and the closing ring beside the pointer over the first
    /// point.
    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        let (Some(o), Some(place)) = (&self.tool.outline, Place::of(cx)) else { return };
        let points: Vec<Pos2> = o.points().map(|at| place.screen(at)).collect();
        if points.len() >= 2 {
            cx.marching_ants(&points, false);
        }
        let Some(style) = cx.style() else { return };
        let (tokens, overlay) = (style.tokens, &style.tokens.color.overlay);
        let side = Vec2::splat(tokens.size.handle.md);
        for at in o.anchors.iter().filter_map(|&i| o.path.get(i)).map(|&at| place.screen(at)) {
            let square = PaintShape::Rect(ScreenRect::from_center_size(at, side), CornerRadius::ZERO);
            draw.fill(&square, style.color(overlay.handle_fill));
            draw.stroke(&square, StrokeWidth::Thin, style.color(overlay.handle_stroke));
        }
        if let Some(pointer) = cx.pointer
            && o.near_start(place.doc(pointer.pos), close_radius(cx, &place))
        {
            // The closing cursor: a small circle beside the pointer.
            let ring = PaintShape::Circle(pointer.pos + side, tokens.shape.radius.sm);
            draw.stroke(&ring, StrokeWidth::Medium, style.color(overlay.halo));
            draw.stroke(&ring, StrokeWidth::Thin, style.color(overlay.handle_fill));
        }
    }

    /// The outline being drawn as one object: its name and point count over the points' bounds; `None` while no
    /// outline is open.
    fn access(&self, cx: &ToolContext<'_>) -> Option<ToolAccess> {
        let points: Vec<Point> = self.tool.outline.as_ref()?.points().collect();
        outline_access(&points, cx)
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Select Area... command")
    }
}

/// How near the first point a press closes the outline, in document units.
fn close_radius(cx: &ToolContext<'_>, place: &Place) -> f64 {
    reach(cx).close / place.zoom().max(0.01)
}
