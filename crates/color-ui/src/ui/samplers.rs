//! Colour samplers (F04.04): up to ten numbered points on the document whose colours a panel reads. `ColorSamplers` is
//! the data, `ColorSamplerMarks` paints the points on the canvas, `ColorSamplerTool` and its pass place, move and remove
//! them as events, `SamplerOptions` is the options bar's member and `SamplerReadouts` reads the colours under the points.
//!
//! ```ignore
//! let mut pass = tool.pass(&samplers, size);
//! Canvas::new("Document", &mut pass).show(ui, |ui, body| {
//!     ColorSamplerMarks::new("Color samplers", &samplers).carried(tool.carried()).show_at(ui, body.rect);
//! });
//! for event in tool.take_events() { /* the host edits its points */ }
//! let readouts = SamplerReadouts::new(&samplers, |at| composite_color(at)).revision(document.revision()).get(&mut cache);
//! ```

use std::hash::Hash;
use std::sync::Arc;

use craft_behavior::kurbo::Point;
use craft_behavior::{CancelReason, Cursor, CursorBadge, CursorIcon, IconId, Input, Metrics, PaintedCursor};
use craft_cms::convert::CmykConverter;
use craft_theme::variant::Tone;
use craft_theme::{Srgba, TypeRole};
use craft_ui::buttons::Button;
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolPath, ToolPointer, canvas_space};
use craft_ui::contract::{
    Access, Bind, CommandSet, Common, Component, Cx, EditResponse, MeasureCx, NoValue, Outcome, PaintUi, Props as _, Role, Show as _,
};
use craft_ui::display::{Divider, Elide, Shade, Text};
use craft_ui::primitives::{PaintShape, StrokeWidth, TextRun};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::toast::Toast;

use super::eyedropper::HOTSPOT;
use super::{InfoColor, SampleSize, SampleSizeSelect};
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{Pos2, Response, Ui, Vec2, vec2};

#[cfg(feature = "suite")]
pub(crate) mod stories;

craft_ui::words! {
    /// The colour samplers' words, English by default.
    #[non_exhaustive]
    pub struct SamplerWords {
        /// The options bar's count; `{n}` points of at most `{max}`.
        count: template "colorSampler.count" = "{n} of {max} sample points";
        clear_all: "colorSampler.clearAll" = "Clear All";
        hint: "colorSampler.hint" = "Click to place · Alt-click or drag off-canvas to remove";
        /// The notice when a point is refused for the limit.
        limit: template "colorSampler.limit" = "Color Sampler limit reached: at most {max} points per document";
    }
}

/// The most points a document holds.
pub const MAX_SAMPLERS: usize = 10;

/// A sampler command that did not apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SamplerError {
    /// Ten points are placed already.
    Full,
    /// The position is off the canvas.
    Outside,
    /// No point has this index.
    NoSuchPoint,
}

/// The commands of the colour samplers (11). They are app commands: the app owns the points, and runs them with
/// [`ColorSamplers::apply`]; New opens the app's form of X and Y in document pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "colorSampler")]
#[non_exhaustive]
pub enum ColorSamplerCommand {
    /// Place a point by its numbers.
    #[command(label = "New Color Sampler…")]
    New,
    /// Remove the focused point, else the last one.
    #[command(label = "Delete Color Sampler")]
    Delete,
    /// Remove every point.
    #[command(label = "Clear All Color Samplers")]
    Clear,
}

/// The sample points of a document, in document pixels, at most [`MAX_SAMPLERS`], numbered by their order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorSamplers {
    points: Vec<[f64; 2]>,
}

impl ColorSamplers {
    /// No points.
    pub fn new() -> ColorSamplers {
        ColorSamplers::default()
    }

    /// The points in number order (the first is number 1).
    pub fn points(&self) -> &[[f64; 2]] {
        &self.points
    }

    /// Places a point at the pixel under `pos` on a document of `size` pixels; the new point's index.
    pub fn add(&mut self, pos: [f64; 2], size: [u32; 2]) -> Result<usize, SamplerError> {
        let pixel = pixel(pos, size).ok_or(SamplerError::Outside)?;
        if self.points.len() >= MAX_SAMPLERS {
            return Err(SamplerError::Full);
        }
        self.points.push(pixel);
        Ok(self.points.len() - 1)
    }

    /// Moves the point `index` to the pixel under `pos`; dropped off the canvas it is removed and the later points
    /// move up one number.
    pub fn move_point(&mut self, index: usize, pos: [f64; 2], size: [u32; 2]) -> Result<(), SamplerError> {
        if index >= self.points.len() {
            return Err(SamplerError::NoSuchPoint);
        }
        match pixel(pos, size) {
            Some(pixel) => self.points[index] = pixel,
            None => {
                self.points.remove(index);
            }
        }
        Ok(())
    }

    /// Removes the point `index`; the later points move up one number.
    pub fn remove(&mut self, index: usize) -> Result<(), SamplerError> {
        if index >= self.points.len() {
            return Err(SamplerError::NoSuchPoint);
        }
        self.points.remove(index);
        Ok(())
    }

    /// Removes every point.
    pub fn clear(&mut self) {
        self.points.clear();
    }

    /// The point nearest `at` within `tolerance` document units.
    pub fn hit(&self, at: [f64; 2], tolerance: f64) -> Option<usize> {
        let distance = |p: &[f64; 2]| (p[0] - at[0]).hypot(p[1] - at[1]);
        self.points
            .iter()
            .enumerate()
            .filter(|(_, p)| distance(p) <= tolerance)
            .min_by(|a, b| distance(a.1).total_cmp(&distance(b.1)))
            .map(|(index, _)| index)
    }

    /// Runs `command`: Delete removes `focused`, else the last point; Clear removes every point. New changes nothing
    /// here: the form that takes the numbers is the app's.
    pub fn apply(&mut self, command: ColorSamplerCommand, focused: Option<usize>) {
        match command {
            ColorSamplerCommand::New => {}
            ColorSamplerCommand::Delete => {
                if let Some(last) = self.points.len().checked_sub(1) {
                    let _ = self.remove(focused.unwrap_or(last));
                }
            }
            ColorSamplerCommand::Clear => self.clear(),
        }
    }
}

/// The pixel under `pos`, if it is on a document of `size` pixels.
fn pixel(pos: [f64; 2], size: [u32; 2]) -> Option<[f64; 2]> {
    let [x, y] = pos.map(f64::floor);
    (x >= 0.0 && y >= 0.0 && x < f64::from(size[0]) && y < f64::from(size[1])).then_some([x, y])
}

/// The style of the sampler marks.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ColorSamplerMarksStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The reticle and its number.
    ink: StyleColor,
    /// The outline under the reticle and the number's shadow, so the marks read on any image.
    halo: StyleColor,
    /// The reticle's radius.
    radius: f32,
    /// How far the ticks reach past the circle.
    tick: f32,
    /// The number's offset from the reticle's centre, and its shadow's offset from the number.
    shadow: f32,
}

/// The numbered reticles of the points on a canvas. Paint only: a press never lands on it.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct ColorSamplerMarks<'a> {
    #[common]
    common: Common,
    samplers: &'a ColorSamplers,
    /// The point the tool carries: it draws at the pointer, or with no index the next number draws there.
    #[prop]
    carried: Option<SamplerCarry>,
}

impl<'a> ColorSamplerMarks<'a> {
    /// The marks of `samplers`, named `label`.
    pub fn new(label: impl Into<Label>, samplers: &'a ColorSamplers) -> Self {
        ColorSamplerMarks { common: Common::new(), samplers, carried: None }.label(label)
    }
}

impl Component for ColorSamplerMarks<'_> {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = ColorSamplerMarksStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: ColorSamplerMarksStyleKey) -> ColorSamplerMarksStyle {
        let tokens = cx.tokens;
        ColorSamplerMarksStyle {
            metrics: Metrics::new(),
            ink: cx.color(tokens.color.overlay.stroke),
            halo: cx.color(tokens.color.overlay.halo),
            radius: tokens.size.handle.md,
            tick: tokens.space.gap.tight,
            shadow: tokens.stroke.thin,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let draw = ui.draw();
        let style = &cx.style;
        let mut marks: Vec<[f64; 2]> = self.samplers.points.clone();
        match self.carried {
            Some(SamplerCarry { index: Some(index), at }) => {
                if let Some(mark) = marks.get_mut(index) {
                    *mark = at;
                }
            }
            Some(SamplerCarry { index: None, at }) if marks.len() < MAX_SAMPLERS => marks.push(at),
            _ => {}
        }
        for (index, point) in marks.iter().enumerate() {
            let at = space.view.to_screen(Point::new(point[0], point[1]));
            let reticle = |width: StrokeWidth, color: StyleColor| {
                draw.stroke(&PaintShape::Circle(at, style.radius), width, color);
                for direction in [Vec2::X, -Vec2::X, Vec2::Y, -Vec2::Y] {
                    let line = vec![at + direction * style.radius, at + direction * (style.radius + style.tick)];
                    draw.stroke(&PaintShape::Path(line), width, color);
                }
            };
            reticle(StrokeWidth::Medium, style.halo);
            reticle(StrokeWidth::Hairline, style.ink);
            let number = (index + 1).to_string();
            let anchor = at + vec2(style.radius, -style.radius);
            for (offset, color) in [(Vec2::splat(style.shadow), style.halo), (Vec2::ZERO, style.ink)] {
                let run = TextRun::new(number.as_str()).role(TypeRole::Caption).color(color);
                let layout = draw.layout(&run, f32::INFINITY);
                let size = layout.size();
                draw.galley(anchor + offset - vec2(0.0, size.y), &layout);
            }
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// What the Color Sampler tool asks the host to do with its points.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum SamplerEvent {
    /// A point was placed at `at`.
    #[non_exhaustive]
    Add {
        /// Where, in document pixels.
        at: [f64; 2],
    },
    /// Point `index` was moved to `to`.
    #[non_exhaustive]
    Move {
        /// The point's index.
        index: usize,
        /// Where it went, in document pixels.
        to: [f64; 2],
    },
    /// Point `index` was removed.
    #[non_exhaustive]
    Remove {
        /// The point's index.
        index: usize,
    },
}

impl SamplerEvent {
    /// A point placed at `at`.
    pub fn add(at: [f64; 2]) -> Self {
        SamplerEvent::Add { at }
    }

    /// Point `index` moved to `to`.
    pub fn moved(index: usize, to: [f64; 2]) -> Self {
        SamplerEvent::Move { index, to }
    }

    /// Point `index` removed.
    pub fn removed(index: usize) -> Self {
        SamplerEvent::Remove { index }
    }
}

/// A point the tool carries: point `index` draws at `at`, or with `index: None` the next number draws at `at` while
/// below [`MAX_SAMPLERS`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SamplerCarry {
    /// The point carried; `None` for a new one.
    pub index: Option<usize>,
    /// Where it draws, in document units.
    pub at: [f64; 2],
}

/// The Color Sampler tool, owned by the host across passes. A press on a point carries it (Alt-press removes it); a
/// press on empty canvas carries a new point, or at [`MAX_SAMPLERS`] carries nothing and sets a notice
/// ([`take_error`](Self::take_error)). The release decides: on the canvas it moves the carried point or places the new
/// one, off the canvas it removes the carried point or places none; Escape drops the carry. The tool never edits the
/// points: the host applies [`SamplerEvent`]s.
#[derive(Debug, Default)]
pub struct ColorSamplerTool {
    events: Vec<SamplerEvent>,
    carried: Option<SamplerCarry>,
    full: Option<Toast>,
}

impl ColorSamplerTool {
    /// A tool with no gesture running.
    pub fn new() -> Self {
        ColorSamplerTool::default()
    }

    /// The tool for one pass over `samplers` on a document of `size` pixels.
    pub fn pass<'a>(&'a mut self, samplers: &'a ColorSamplers, size: [u32; 2]) -> ColorSamplerPass<'a> {
        ColorSamplerPass { tool: self, samplers, size }
    }

    /// The events since the last call.
    pub fn take_events(&mut self) -> Vec<SamplerEvent> {
        std::mem::take(&mut self.events)
    }

    /// The notice of a press that was refused, once.
    pub fn take_error(&mut self) -> Option<Toast> {
        self.full.take()
    }

    /// The point being carried, for the marks.
    pub fn carried(&self) -> Option<SamplerCarry> {
        self.carried
    }
}

/// One pass of the [`ColorSamplerTool`] over the host's points, which it reads and never edits.
pub struct ColorSamplerPass<'a> {
    tool: &'a mut ColorSamplerTool,
    samplers: &'a ColorSamplers,
    size: [u32; 2],
}

impl ColorSamplerPass<'_> {
    /// The tool's events since the last call: the host holding the pass reads them without the tool.
    pub fn take_events(&mut self) -> Vec<SamplerEvent> {
        self.tool.take_events()
    }

    /// The tool's notice of a refused press, once.
    pub fn take_error(&mut self) -> Option<Toast> {
        self.tool.take_error()
    }

    fn doc_at(cx: &ToolContext<'_>, pos: Pos2) -> Option<[f64; 2]> {
        let Point { x, y } = cx.space()?.view.to_doc(pos);
        Some([x, y])
    }

    /// The point under `pos`, within the pointer's reach.
    fn hit(&self, cx: &ToolContext<'_>, pos: Pos2) -> Option<usize> {
        let space = cx.space()?;
        let at = Self::doc_at(cx, pos)?;
        self.samplers.hit(at, space.view.doc_len(cx.reach()))
    }
}

impl CanvasTool for ColorSamplerPass<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(at) = Self::doc_at(cx, pointer.pos) else { return ToolAction::Ignored };
        self.tool.carried = None;
        match self.hit(cx, pointer.pos) {
            Some(index) if pointer.mods.alt => {
                self.tool.events.push(SamplerEvent::Remove { index });
                ToolAction::Done
            }
            Some(index) => {
                self.tool.carried = Some(SamplerCarry { index: Some(index), at });
                ToolAction::Claimed
            }
            None if self.samplers.points().len() >= MAX_SAMPLERS => {
                let words = cx.words::<SamplerWords>();
                let text = words.limit.fill(&[("max", &MAX_SAMPLERS.to_string())]);
                self.tool.full = Some(Toast::new(Label::from(text)).tone(Tone::Bad));
                ToolAction::Done
            }
            None => {
                self.tool.carried = Some(SamplerCarry { index: None, at });
                ToolAction::Claimed
            }
        }
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(carried) = self.tool.carried.as_mut() else { return };
        if let Some(to) = Self::doc_at(cx, pointer.pos) {
            carried.at = to;
            cx.request_repaint();
        }
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some(carried) = self.tool.carried.take() else { return };
        let landed = Self::doc_at(cx, pointer.pos).and_then(|to| pixel(to, self.size));
        let event = match (carried.index, landed) {
            // A release on the pixel the point is on moves nothing.
            (Some(index), Some(to)) => (self.samplers.points().get(index) != Some(&to)).then_some(SamplerEvent::Move { index, to }),
            (Some(index), None) => Some(SamplerEvent::Remove { index }),
            (None, Some(at)) => Some(SamplerEvent::Add { at }),
            (None, None) => None,
        };
        self.tool.events.extend(event);
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {
        self.tool.carried = None;
    }

    fn paint(&self, _: &craft_ui::primitives::Draw<'_>, _: &ToolContext<'_>) {}

    /// The target ring over a point or while carrying (scissors for Alt over a point); else the sampler icon, or the
    /// crosshair when the precise cursor is on.
    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        let over = cx.pointer.as_ref().and_then(|pointer| self.hit(cx, pointer.pos));
        match (self.tool.carried, over) {
            (Some(_), _) => PaintedCursor::badged(CursorIcon::Default, CursorBadge::Target).into(),
            (None, Some(_)) if cx.mods.alt => PaintedCursor::badged(CursorIcon::Default, CursorBadge::Scissors).into(),
            (None, Some(_)) => PaintedCursor::badged(CursorIcon::Default, CursorBadge::Target).into(),
            _ if cx.precise() => PaintedCursor::Crosshair.into(),
            _ => PaintedCursor::tool(IconId::new("sampler"), Vec2::from(HOTSPOT), None).into(),
        }
    }

    fn autoscrolls(&self) -> bool {
        false
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the New Color Sampler… command")
    }
}

/// The colour under a point, as a panel shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SamplerReadout {
    /// The point's number (1 to 10).
    pub number: usize,
    /// The pixel it is on.
    pub position: [i64; 2],
    /// The colour in 8-bit sRGB.
    pub rgb: [u8; 3],
    /// The colour in CMYK percent through the document's converter.
    pub cmyk: [u8; 4],
    /// The colour in Lab (L 0 to 100, a and b -128 to 127), rounded, when the host fills it: the RAW samplers while
    /// Show Lab Color Readouts is on. `None` by default.
    pub lab: Option<[i16; 3]>,
}

impl SamplerReadout {
    /// The readout of point number `number` at `position` with the given 8-bit `rgb` and CMYK percent: how a host whose
    /// colours come from its own engine fills the rows.
    pub fn from_values(number: usize, position: [i64; 2], rgb: [u8; 3], cmyk: [u8; 4]) -> SamplerReadout {
        SamplerReadout { number, position, rgb, cmyk, lab: None }
    }

    /// The readout of point `index` at `position` with `color`, CMYK through `cmyk`. A channel that is not finite
    /// reads as 0 and the rest is clamped.
    pub fn new(index: usize, position: [f64; 2], color: Srgba, cmyk: &CmykConverter) -> SamplerReadout {
        let InfoColor { rgb, cmyk } = InfoColor::new(color, cmyk);
        SamplerReadout::from_values(index + 1, position.map(|v| v.floor() as i64), rgb, cmyk)
    }
}

/// The options bar's members: the Sample Size, the count, Clear All and the hint.
#[must_use = "show the options with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct SamplerOptions<'a> {
    count: usize,
    size: Bind<'a, SampleSize>,
}

/// What [`SamplerOptions::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct SamplerOptionsOutput {
    /// The Clear All button.
    pub response: Response,
    /// Clear All was pressed.
    pub clear_all: bool,
    /// The Sample Size select's edit.
    pub size: EditResponse<SampleSize>,
}

impl Outcome for SamplerOptionsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl<'a> SamplerOptions<'a> {
    /// The members for a document with `count` points and the Sample Size `size`.
    pub fn new(count: usize, size: Bind<'a, SampleSize>) -> Self {
        SamplerOptions { count, size }
    }

    /// Shows the members inside the options `Bar`.
    pub fn show(self, ui: &mut Ui) -> SamplerOptionsOutput {
        let SamplerOptions { count, size } = self;
        let words = Env::of(ui).words::<SamplerWords>();
        let counted = words.count.fill(&[("n", &count.to_string()), ("max", &MAX_SAMPLERS.to_string())]);
        let size = SampleSizeSelect::new(size).show(ui);
        Divider::vertical().show(ui);
        Text::new(Label::from(counted)).show(ui);
        Divider::vertical().show(ui);
        let clear = Button::new(words.clear_all.clone()).enabled(count > 0).show(ui);
        Divider::vertical().show(ui);
        Text::new(words.hint.clone()).shade(Shade::Faint).elide(Elide::End).show(ui);
        SamplerOptionsOutput { clear_all: craft_ui::contract::clicked(&clear), response: clear.response, size }
    }
}

type Cached = (u64, u64, usize, Vec<[f64; 2]>, Arc<Vec<SamplerReadout>>);

/// The readouts a host cached: keyed by the revision, the points and the CMYK profile.
#[derive(Clone, Default)]
pub struct SamplerCache {
    cached: Option<Cached>,
}

/// The readouts of every point, cached by the host between passes.
pub struct SamplerReadouts<'a, F> {
    samplers: &'a ColorSamplers,
    read: F,
    revision: u64,
    document: u64,
    cmyk: Option<&'a CmykConverter>,
}

impl<'a, F: Fn([f64; 2]) -> Srgba> SamplerReadouts<'a, F> {
    /// The readouts of `samplers`, reading the colour under a point with `read`.
    pub fn new(samplers: &'a ColorSamplers, read: F) -> Self {
        SamplerReadouts { samplers, read, revision: 0, document: 0, cmyk: None }
    }

    /// The document's revision: a new one reads again.
    #[must_use]
    pub fn revision(mut self, revision: u64) -> Self {
        self.revision = revision;
        self
    }

    /// Which document is read: another document at the same revision reads again.
    #[must_use]
    pub fn document(mut self, document: impl Hash) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        document.hash(&mut hasher);
        self.document = std::hash::Hasher::finish(&hasher);
        self
    }

    /// The document's CMYK converter; the built-in coated one without it.
    #[must_use]
    pub fn cmyk(mut self, converter: &'a CmykConverter) -> Self {
        self.cmyk = Some(converter);
        self
    }

    /// The readouts, from `cache` while the revision, the document, the points and the CMYK profile are as they were.
    pub fn get(self, cache: &mut SamplerCache) -> Arc<Vec<SamplerReadout>> {
        let cmyk = self.cmyk.unwrap_or_else(|| CmykConverter::coated());
        let profile = Arc::as_ptr(cmyk.profile()) as usize;
        if let Some((revision, document, cached_profile, points, readouts)) = &cache.cached
            && *revision == self.revision
            && *document == self.document
            && *cached_profile == profile
            && *points == self.samplers.points
        {
            return Arc::clone(readouts);
        }
        let readouts: Arc<Vec<SamplerReadout>> =
            Arc::new(self.samplers.points.iter().enumerate().map(|(i, p)| SamplerReadout::new(i, *p, (self.read)(*p), cmyk)).collect());
        cache.cached = Some((self.revision, self.document, profile, self.samplers.points.clone(), Arc::clone(&readouts)));
        readouts
    }
}
