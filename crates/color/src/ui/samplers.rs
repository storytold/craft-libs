//! Colour samplers (F04.04): up to ten numbered points on the document whose colours a panel reads. `ColorSamplers` is
//! the data, `ColorSamplerMarks` paints the points on the canvas, `ColorSamplerTool` places, moves and removes them,
//! `SamplerOptions` is the options bar's member and `SamplerReadouts` reads the colours under the points.
//!
//! ```ignore
//! Canvas::new("Document", ColorSamplerTool::new(&mut samplers, size).command_form()).show(ui, |ui, body| {
//!     ColorSamplerMarks::new("Color samplers", &samplers).show_at(ui, body.rect);
//! });
//! let readouts = SamplerReadouts::new(&samplers, |at| composite_color(at)).revision(document.revision()).get(&mut cache);
//! ```

use std::hash::Hash;
use std::sync::Arc;

use craft_behavior::kurbo::Point;
use craft_behavior::{CancelReason, Cursor, Input, Metrics, PaintedCursor};
use craft_cms::convert::CmykConverter;
use craft_theme::variant::Tone;
use craft_theme::{Srgba, TypeRole};
use craft_ui::buttons::Button;
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolPath, ToolPointer, canvas_space};
use craft_ui::contract::{Access, CommandSet, Common, Component, Cx, MeasureCx, NoValue, Outcome, PaintUi, Props as _, Role, Show as _};
use craft_ui::display::Text;
use craft_ui::primitives::{PaintShape, StrokeWidth, TextRun};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::toast::Toast;
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{Pos2, Response, Ui, Vec2, vec2};

#[cfg(feature = "screen-testing")]
mod stories;

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
#[derive(craft_ui::contract::Component)]
#[component(canvas_control)]
pub struct ColorSamplerMarks<'a> {
    #[common]
    common: Common,
    samplers: &'a ColorSamplers,
}

impl<'a> ColorSamplerMarks<'a> {
    /// The marks of `samplers`, named `label`.
    pub fn new(label: impl Into<Label>, samplers: &'a ColorSamplers) -> Self {
        ColorSamplerMarks { common: Common::new(), samplers }.label(label)
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
        for (index, point) in self.samplers.points.iter().enumerate() {
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

/// The Color Sampler tool: a press on empty canvas places a point and the drag that follows carries it until the
/// release (the eleventh reports [`SamplerError::Full`] as a toast through [`take_error`](Self::take_error)), a drag
/// on a point moves it, a point dropped off the canvas is removed, and Alt-click removes a point and renumbers the rest.
pub struct ColorSamplerTool<'a> {
    samplers: &'a mut ColorSamplers,
    size: [u32; 2],
    /// The point being carried: its index, the press position and where the press found the point.
    moving: Option<(usize, [f64; 2], [f64; 2])>,
    full: Option<Toast>,
}

impl<'a> ColorSamplerTool<'a> {
    /// A tool over `samplers` on a document of `size` pixels.
    pub fn new(samplers: &'a mut ColorSamplers, size: [u32; 2]) -> Self {
        ColorSamplerTool { samplers, size, moving: None, full: None }
    }

    /// The points the tool edits.
    pub fn samplers(&self) -> &ColorSamplers {
        self.samplers
    }

    /// The notice of a press that was refused, once.
    pub fn take_error(&mut self) -> Option<Toast> {
        self.full.take()
    }

    fn doc_at(cx: &ToolContext<'_>, pos: Pos2) -> Option<[f64; 2]> {
        let Point { x, y } = cx.space()?.view.to_doc(pos);
        Some([x, y])
    }
}

impl CanvasTool for ColorSamplerTool<'_> {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let (Some(at), Some(space)) = (Self::doc_at(cx, pointer.pos), cx.space()) else { return ToolAction::Ignored };
        let tolerance = space.view.doc_len(cx.reach());
        self.moving = None;
        match self.samplers.hit(at, tolerance) {
            Some(index) if pointer.mods.alt => {
                let _ = self.samplers.remove(index);
                ToolAction::Done
            }
            Some(index) => {
                self.moving = Some((index, at, self.samplers.points()[index]));
                ToolAction::Claimed
            }
            None if pointer.mods.alt => ToolAction::Done,
            None => match self.samplers.add(at, self.size) {
                Ok(index) => {
                    self.moving = Some((index, at, self.samplers.points()[index]));
                    ToolAction::Claimed
                }
                Err(error) => {
                    if error == SamplerError::Full {
                        let words = cx.words::<SamplerWords>();
                        let text = words.limit.fill(&[("max", &MAX_SAMPLERS.to_string())]);
                        self.full = Some(Toast::new(Label::from(text)).tone(Tone::Bad));
                    }
                    ToolAction::Done
                }
            },
        }
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let (Some((index, ..)), Some(to)) = (self.moving, Self::doc_at(cx, pointer.pos)) else { return };
        // Off the canvas the point stays at its last position; the release removes it.
        if pixel(to, self.size).is_some() {
            let _ = self.samplers.move_point(index, to, self.size);
        }
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        let Some((index, from, _)) = self.moving.take() else { return };
        let Some(to) = Self::doc_at(cx, pointer.pos) else { return };
        if (to[0] - from[0]).hypot(to[1] - from[1]) <= 0.5 {
            return;
        }
        let _ = self.samplers.move_point(index, to, self.size);
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {
        if let Some((index, _, start)) = self.moving.take() {
            let _ = self.samplers.move_point(index, start, self.size);
        }
    }

    fn paint(&self, _: &craft_ui::primitives::Draw<'_>, _: &ToolContext<'_>) {}

    fn cursor(&self, _: &ToolContext<'_>) -> Cursor {
        PaintedCursor::Crosshair.into()
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
}

impl SamplerReadout {
    /// The readout of point `index` at `position` with `color`, CMYK through `cmyk`. A channel that is not finite
    /// reads as 0 and the rest is clamped.
    pub fn new(index: usize, position: [f64; 2], color: Srgba, cmyk: &CmykConverter) -> SamplerReadout {
        let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        let rgb = [color.r, color.g, color.b].map(unit);
        SamplerReadout {
            number: index + 1,
            position: position.map(|v| v.floor() as i64),
            rgb: rgb.map(|v| (v * 255.0).round() as u8),
            cmyk: cmyk.from_srgb(rgb).map(|v| (unit(v) * 100.0).round() as u8),
        }
    }
}

/// The options bar's members: the count, Clear All and the hint.
#[must_use = "show the options with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct SamplerOptions {
    count: usize,
}

/// What [`SamplerOptions::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct SamplerOptionsOutput {
    /// The Clear All button.
    pub response: Response,
    /// Clear All was pressed.
    pub clear_all: bool,
}

impl Outcome for SamplerOptionsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl SamplerOptions {
    /// The members for a document with `count` points.
    pub fn new(count: usize) -> Self {
        SamplerOptions { count }
    }

    /// Shows the members inside the options `Bar`.
    pub fn show(self, ui: &mut Ui) -> SamplerOptionsOutput {
        let words = Env::of(ui).words::<SamplerWords>();
        let counted = words.count.fill(&[("n", &self.count.to_string()), ("max", &MAX_SAMPLERS.to_string())]);
        Text::new(Label::from(counted)).show(ui);
        let clear = Button::new(words.clear_all.clone()).enabled(self.count > 0).show(ui);
        Text::new(words.hint.clone()).show(ui);
        SamplerOptionsOutput { clear_all: craft_ui::contract::clicked(&clear), response: clear.response }
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
