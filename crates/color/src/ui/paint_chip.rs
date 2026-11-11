//! [`PaintChip`] (F04.06): a shape's fill or stroke paint as a chip that opens a popover with No
//! Color, Solid and Gradient, as PhotoCraft's shape options bar and Properties panel; and
//! [`PaintList`], named paints in rows with their chips.
//!
//! The gradient editor is craft-draw's, and craft-draw depends on craft-color, so the chip takes
//! the editor's body from the host ([`PaintChip::gradient_editor`]); so do the Pattern and Picture
//! tabs ([`PaintChip::pattern_editor`], [`PaintChip::picture_editor`]: craft-draw's
//! `PatternFillEditor` and `PictureFillEditor`).

use craft_behavior::Mixed;
use craft_behavior::{Axis, EditEvent, EditEvents, Metrics, PartKey};
use craft_theme::Srgba;
use craft_ui::choice::{ChoiceWords, Segmented, Swatch};
use craft_ui::contract::{
    Access, Activation, Bind, Common, Component, Cx, Edit as _, EditResponse, Entry, Outcome, Overlay, OverlayUi, Overlays as _, PaintUi, PartsCx,
    Pointer as _, PopupKind, Props as _, Remember as _, Role, Show as _, WordSetCx as _,
};
use craft_ui::primitives::textures::CachedTexture;
use craft_ui::primitives::{ContentPainterCx, PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, DataColor, FocusLook, StyleColor, StyleCx};
use egui::{Color32, Pos2, Rect, Response, Ui, pos2, vec2};

use super::popover::compact_picker;
use super::{PickedColor, ScreenPickGesture};

mod paint_list;
#[cfg(feature = "screen-testing")]
mod stories;

pub use paint_list::{PaintLink, PaintLinkKind, PaintList, PaintListEvent, PaintListOptions, PaintListOutput, PaintRow};

/// The chip is this many control heights wide.
const CHIP_WIDTH_IN_HEIGHTS: f32 = 1.5;
/// The most tiles a pattern chip draws; a tile so small that more would fit is drawn as one stretched image.
const MAX_TILES: f32 = 4096.0;
/// The No Color diagonal stops this many control heights short of the chip's edges.
const DIAGONAL_INSET_IN_HEIGHTS: f32 = 0.15;
/// The chip's face: a mixed paint shows the colour well's mixed look.
const WELL: PartKey = PartKey::new("well");

craft_ui::words! {
    /// The words of [`PaintChip`] and [`PaintList`], English by default.
    #[non_exhaustive]
    pub struct PaintWords {
        no_color: "paint.noColor" = "No Color";
        solid: "paint.solid" = "Solid";
        gradient: "paint.gradient" = "Gradient";
        pattern: "paint.pattern" = "Pattern";
        picture: "paint.picture" = "Picture";
        /// The chip's value for a screen reader: `{name}` the pattern's name.
        pattern_named: template "paint.patternNamed" = "Pattern: {name}";
        /// The chip's value for a screen reader: `{name}` the picture's name.
        picture_named: template "paint.pictureNamed" = "Picture: {name}";
        /// A row's chip: opens the picker.
        edit: "paintList.edit" = "Edit color";
        hex: "paintList.hex" = "Color";
        invalid: "paintList.invalid" = "Invalid color";
        opacity: "paintList.opacity" = "Opacity";
        detach_style: "paintList.detachStyle" = "Detach style";
        detach_variable: "paintList.detachVariable" = "Detach variable";
        styles: "paintList.styles" = "Color styles";
        /// A row's button: `{name}` the paint it selects the layers of.
        select_users: template "paintList.selectUsers" = "Select layers with {name}";
        add: "paintList.add" = "Add";
        /// A row's button: `{name}` the paint it removes.
        remove: template "paintList.remove" = "Remove {name}";
    }
}

/// A gradient as the chip draws it: stops at 0–1 along a start-to-end ramp. The gradient itself
/// (angle, style, midpoints) is the host's; it gives this preview with the paint.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct GradientPreview {
    /// Stops as (position 0–1, colour), ascending by position.
    pub stops: Vec<(f32, Srgba)>,
}

impl GradientPreview {
    /// A preview through `stops`.
    pub fn new(stops: Vec<(f32, Srgba)>) -> Self {
        GradientPreview { stops }
    }
}

/// A pattern or picture as the chip draws it: the host's texture of the image (a pattern's tile,
/// tiled across the chip at 1:1; a picture, stretched to it) and its name for a screen reader. The
/// pattern or picture itself is the host's.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ImagePreview {
    /// "Bubbles", "photo.png".
    pub name: String,
    /// The image's texture, `None` while the host has none (the chip shows the field fill).
    pub texture: Option<CachedTexture>,
}

impl ImagePreview {
    /// A preview of `name` drawn from `texture`.
    pub fn new(name: impl Into<String>, texture: Option<CachedTexture>) -> Self {
        ImagePreview { name: name.into(), texture }
    }
}

/// A paint: none, a solid colour, a gradient, a pattern or a picture. More kinds may come: a
/// `match` on it outside craft-color needs a `_` arm.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Paint {
    /// No paint.
    None,
    /// A flat colour.
    Solid(Srgba),
    /// A gradient, as the chip previews it.
    Gradient(GradientPreview),
    /// A pattern fill (Photoshop's shape Pattern tab, PowerPoint's Pattern fill).
    Pattern(ImagePreview),
    /// A picture fill (PowerPoint's Picture or texture fill).
    Picture(ImagePreview),
}

/// The kinds of paint, as the chip's tabs and an app's fill-type switch name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FillKind {
    #[allow(missing_docs)]
    None,
    #[allow(missing_docs)]
    Solid,
    #[allow(missing_docs)]
    Gradient,
    #[allow(missing_docs)]
    Pattern,
    #[allow(missing_docs)]
    Picture,
}

impl FillKind {
    /// The kind of `paint`.
    pub fn of(paint: &Paint) -> FillKind {
        match paint {
            Paint::None => FillKind::None,
            Paint::Solid(_) => FillKind::Solid,
            Paint::Gradient(_) => FillKind::Gradient,
            Paint::Pattern(_) => FillKind::Pattern,
            Paint::Picture(_) => FillKind::Picture,
        }
    }
}

type PaintEditorFn<'a, T> = Box<dyn FnMut(&mut Ui) -> Option<T> + 'a>;

/// The host's editor body for one fill kind, drawn in the popover: it edits the host's fill and returns the new
/// preview when it changed.
pub struct PaintEditor<'a, T>(PaintEditorFn<'a, T>);

impl<'a, T, F: FnMut(&mut Ui) -> Option<T> + 'a> From<F> for PaintEditor<'a, T> {
    fn from(editor: F) -> Self {
        PaintEditor(Box::new(editor))
    }
}

impl<T> PaintEditor<'_, T> {
    fn show(&mut self, ui: &mut Ui) -> Option<T> {
        (self.0)(ui)
    }
}

/// The style of a paint chip.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PaintChipStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The chip's height.
    height: f32,
    /// The chip's corners.
    radius: egui::CornerRadius,
    /// The ring on the chip's outline.
    edge: StyleColor,
    /// The fill of an empty chip: the field's.
    none_fill: StyleColor,
    /// The No Color diagonal.
    none_mark: StyleColor,
}

/// The last solid colour the chip held: a switch back to Solid starts from it.
#[derive(Clone, Copy, Default)]
struct LastSolid(Option<Srgba>);

/// What the popover's body reported this pass.
#[derive(Default)]
struct Shown {
    kind: Option<FillKind>,
    edit: EditEvents<PickedColor>,
    screen_pick: Option<ScreenPickGesture>,
    gradient: Option<GradientPreview>,
    pattern: Option<ImagePreview>,
    picture: Option<ImagePreview>,
}

/// What a paint chip reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PaintChipOutput {
    /// The edit of the paint: a switch of kind is one `Set`, the picker's gestures report as the picker does.
    pub edit: EditResponse<Paint>,
    /// The screen eyedropper's press: the host runs the screen pick and answers through [`PaintChip::sampled`].
    pub screen_pick: Option<ScreenPickGesture>,
}

impl Outcome for PaintChipOutput {
    type Value = Paint;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// A paint chip, a control high and 1.5 times as wide.
///
/// - Look: a solid paint fills the chip over the checkerboard (alpha shows); a gradient is a
///   start-to-end ramp through its stops; No Color is the field fill with a `status.bad` diagonal
///   from bottom start to top end; a pattern tiles its texture at 1:1; a picture is stretched.
/// - Click, Enter or Space opens a popover under the chip: a segmented No Color / Solid / Gradient / Pattern /
///   Picture switch (No Color only with [`none_allowed`](Self::none_allowed); Gradient, Pattern and Picture only
///   when the host gave their editor), then the Compact [`ColorPicker`] (Solid, with the screen eyedropper) or the
///   host's editor. Switching to Solid starts from the last solid colour (or the fallback), to Gradient, Pattern
///   or Picture from the host's default.
/// - Edits: a switch is a `Set`; picker gestures report as the picker does. A screen sample keeps alpha.
/// - AccessKit: a colour well named by the label, its value the hex or "No Color", "Gradient", the pattern's or
///   picture's name.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(overlay, content_painter)]
pub struct PaintChip<'a> {
    #[common]
    common: Common,
    paint: Bind<'a, Paint>,
    fallback: Srgba,
    /// No Color is offered (a stroke can be none, a text colour cannot).
    #[prop(flag)]
    none_allowed: bool,
    gradient: Option<(GradientPreview, PaintEditor<'a, GradientPreview>)>,
    pattern: Option<(ImagePreview, PaintEditor<'a, ImagePreview>)>,
    picture: Option<(ImagePreview, PaintEditor<'a, ImagePreview>)>,
    /// The host's answer to the screen eyedropper: a `Set` of the sampled colour that keeps the alpha.
    #[prop]
    sampled: Option<Srgba>,
    screen_pick: Option<ScreenPickGesture>,
}

impl<'a> PaintChip<'a> {
    /// A chip for `paint` named `label`; `fallback` is the solid colour a switch to Solid starts from when the
    /// paint has none.
    pub fn new(label: impl Into<Label>, paint: Bind<'a, Paint>, fallback: Srgba) -> Self {
        PaintChip {
            common: Common::new(),
            paint,
            fallback,
            none_allowed: false,
            gradient: None,
            pattern: None,
            picture: None,
            sampled: None,
            screen_pick: None,
        }
        .label(label)
    }

    /// The Gradient tab: `default_gradient` is what a switch to Gradient sets, `editor` draws the host's editor.
    /// Without it there is no Gradient tab.
    #[must_use]
    pub fn gradient_editor(mut self, default_gradient: GradientPreview, editor: impl Into<PaintEditor<'a, GradientPreview>>) -> Self {
        self.gradient = Some((default_gradient, editor.into()));
        self
    }

    /// The Pattern tab: `default_pattern` is what a switch to Pattern sets, `editor` draws the host's editor.
    /// Without it there is no Pattern tab.
    #[must_use]
    pub fn pattern_editor(mut self, default_pattern: ImagePreview, editor: impl Into<PaintEditor<'a, ImagePreview>>) -> Self {
        self.pattern = Some((default_pattern, editor.into()));
        self
    }

    /// The Picture tab: `default_picture` is what a switch to Picture sets, `editor` draws the host's editor.
    /// Without it there is no Picture tab.
    #[must_use]
    pub fn picture_editor(mut self, default_picture: ImagePreview, editor: impl Into<PaintEditor<'a, ImagePreview>>) -> Self {
        self.picture = Some((default_picture, editor.into()));
        self
    }

    /// The tabs the popover shows, in order: No Color (when allowed), Solid, then Gradient, Pattern and Picture
    /// when the host gave their editors.
    pub fn kinds(&self) -> Vec<FillKind> {
        let offered = [
            (FillKind::None, self.none_allowed),
            (FillKind::Solid, true),
            (FillKind::Gradient, self.gradient.is_some()),
            (FillKind::Pattern, self.pattern.is_some()),
            (FillKind::Picture, self.picture.is_some()),
        ];
        offered.into_iter().filter_map(|(kind, on)| on.then_some(kind)).collect()
    }

    /// The gradient a switch to Gradient sets, when the host gave an editor.
    pub fn default_gradient(&self) -> Option<&GradientPreview> {
        self.gradient.as_ref().map(|(default, _)| default)
    }

    /// The paint a switch to `kind` sets.
    fn switched(&self, kind: FillKind, solid: Srgba) -> Paint {
        match kind {
            FillKind::None => Paint::None,
            FillKind::Solid => Paint::Solid(solid),
            FillKind::Gradient => Paint::Gradient(self.gradient.as_ref().map(|(default, _)| default.clone()).unwrap_or_default()),
            FillKind::Pattern => Paint::Pattern(self.pattern.as_ref().map(|(default, _)| default.clone()).unwrap_or_default()),
            FillKind::Picture => Paint::Picture(self.picture.as_ref().map(|(default, _)| default.clone()).unwrap_or_default()),
        }
    }
}

fn kind_label(words: &PaintWords, kind: FillKind) -> Label {
    match kind {
        FillKind::None => words.no_color.clone(),
        FillKind::Solid => words.solid.clone(),
        FillKind::Gradient => words.gradient.clone(),
        FillKind::Pattern => words.pattern.clone(),
        FillKind::Picture => words.picture.clone(),
    }
}

/// The chip's value for a screen reader: the hex, "No Color", "Gradient", "Pattern: Bubbles" or "Picture:
/// photo.png"; the kind alone for an image without a name.
fn value_text(words: &PaintWords, paint: &Paint) -> String {
    let (named, kind, preview) = match paint {
        Paint::None => return words.no_color.as_str().to_owned(),
        Paint::Solid(color) => return color.to_hex(),
        Paint::Gradient(_) => return words.gradient.as_str().to_owned(),
        Paint::Pattern(preview) => (&words.pattern_named, &words.pattern, preview),
        Paint::Picture(preview) => (&words.picture_named, &words.picture, preview),
    };
    if preview.name.is_empty() { kind.as_str().to_owned() } else { named.fill(&[("name", preview.name.as_str())]) }
}

impl Component for PaintChip<'_> {
    const ROLE: Role = Role::COLOR_WELL;
    type Style = PaintChipStyle;
    type Output = PaintChipOutput;

    fn style(cx: &StyleCx<'_>, _: PaintChipStyleKey) -> PaintChipStyle {
        let colors = &cx.tokens.color;
        let metrics = craft_ui::choice::SwatchMetrics::of(cx);
        PaintChipStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            height: metrics.side,
            radius: metrics.radius,
            edge: cx.color(colors.line.control_edge),
            none_fill: cx.color(colors.surface.sunken),
            none_mark: cx.color(colors.status.bad.solid),
        }
    }

    fn sense(&self) -> craft_behavior::Sense {
        craft_behavior::Sense::CLICK | craft_behavior::Sense::FOCUS
    }

    fn measure(&mut self, cx: &craft_ui::contract::MeasureCx<'_, Self>) -> egui::Vec2 {
        vec2(cx.style.height * CHIP_WIDTH_IN_HEIGHTS, cx.style.height)
    }

    fn parts(&mut self, rect: egui::Rect, cx: &mut PartsCx<'_, Self>) {
        let _ = cx.part(WELL, rect);
    }

    fn bind(&mut self) -> Option<craft_ui::contract::BindMut<'_, Paint>> {
        Some(self.paint.reborrow())
    }

    /// A click, Enter or Space toggles the popover; the last solid colour is kept for a switch back to Solid.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(Paint::Solid(color)) = cx.value() {
            let color = *color;
            cx.memory::<LastSolid>().0 = Some(color);
        }
        let popup = cx.popup("picker");
        let activated = cx.activation().filter(|_: &Activation| cx.resolved.enabled).is_some();
        let open = popup.is_open(cx);
        if activated {
            if open {
                popup.close(cx);
            } else {
                popup.open(cx);
            }
        }
    }

    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("picker");
        let words = cx.words::<PaintWords>();
        let name = cx.label();
        let paint = cx.value().cloned().unwrap_or(Paint::None);
        let solid = cx.memory::<LastSolid>().0.unwrap_or(self.fallback);
        let current = FillKind::of(&paint);
        let sampled = self.sampled;
        let entries: Vec<Entry<FillKind>> = self.kinds().into_iter().map(|kind| Entry::new(kind, kind_label(&words, kind))).collect();
        let (gradient, pattern, picture) = (&mut self.gradient, &mut self.pattern, &mut self.picture);
        let shown = ui.show(popup, Overlay::popover(), |ui, _| {
            let mut shown = Shown::default();
            let mut kind = current;
            Segmented::new(name.clone(), Bind::bound(&mut kind), &entries).show(ui);
            shown.kind = (kind != current).then_some(kind);
            match &paint {
                Paint::None => {}
                Paint::Solid(color) => {
                    let picked = compact_picker(name.clone(), Bind::given(PickedColor::rgb(*color)), sampled).alpha(true).eyedropper(true).show(ui);
                    shown.edit = picked.events;
                    shown.screen_pick = picked.screen_pick;
                }
                Paint::Gradient(_) => shown.gradient = gradient.as_mut().and_then(|(_, editor)| editor.show(ui)),
                Paint::Pattern(_) => shown.pattern = pattern.as_mut().and_then(|(_, editor)| editor.show(ui)),
                Paint::Picture(_) => shown.picture = picture.as_mut().and_then(|(_, editor)| editor.show(ui)),
            }
            shown
        });
        let Some(shown) = shown else { return };
        self.screen_pick = shown.screen_pick;
        if let Some(kind) = shown.kind {
            let next = self.switched(kind, solid);
            cx.set(next);
        } else {
            for event in &shown.edit {
                forward(cx, event);
            }
        }
        if let Some(preview) = shown.gradient {
            cx.set(Paint::Gradient(preview));
        }
        if let Some(preview) = shown.pattern {
            cx.set(Paint::Pattern(preview));
        }
        if let Some(preview) = shown.picture {
            cx.set(Paint::Picture(preview));
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<PaintWords>();
        match cx.value() {
            Some(paint) => access.value(value_text(&words, paint)),
            None => access.value(cx.words::<ChoiceWords>().swatch_mixed.as_str()),
        };
        access.has_popup(PopupKind::Dialog);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let well = cx.rect;
        let shape = PaintShape::Rect(well, style.radius);
        let Some(paint) = cx.value().cloned() else {
            ui.face(WELL, Swatch::new(cx.resolved.label.to_owned(), Mixed::<Srgba>::Mixed));
            return;
        };
        let painter = ui.content_painter(cx);
        match &paint {
            Paint::Solid(color) => {
                if color.a < 1.0 {
                    painter.checker(well);
                }
                ui.draw().data_fill(&shape, DataColor::new(Color32::from(*color)));
            }
            Paint::None => {
                let draw = ui.draw();
                draw.fill(&shape, style.none_fill);
                let inner = well.shrink(style.height * DIAGONAL_INSET_IN_HEIGHTS);
                let (from, to) = if cx.flow.is_rtl() { (inner.right_bottom(), inner.left_top()) } else { (inner.left_bottom(), inner.right_top()) };
                draw.stroke(&PaintShape::Path(vec![from, to]), StrokeWidth::Medium, style.none_mark);
            }
            Paint::Gradient(preview) => {
                painter.checker(well);
                if !preview.stops.is_empty() {
                    painter.ramp(well, &ramp_stops(preview, cx.flow.is_rtl()), Axis::X);
                }
            }
            Paint::Pattern(image) | Paint::Picture(image) => {
                painter.checker(well);
                match image.texture {
                    Some(texture) => paint_image(&painter, well, texture, matches!(paint, Paint::Pattern(_))),
                    None => ui.draw().fill(&shape, style.none_fill),
                }
            }
        }
        ui.draw().stroke(&shape, StrokeWidth::Thin, style.edge);
    }

    fn finish(self, cx: Cx<'_, Self>) -> PaintChipOutput {
        PaintChipOutput { edit: cx.edit_response(), screen_pick: self.screen_pick }
    }
}

/// Reports one of the picker's `event`s as an edit of the chip's paint.
fn forward(cx: &mut Cx<'_, PaintChip<'_>>, event: &EditEvent<PickedColor>) {
    let value = event.value().map(|picked| Paint::Solid(picked.color));
    match (event, value) {
        (EditEvent::Set { .. }, Some(value)) => cx.set(value),
        (EditEvent::Begin { .. }, value) => {
            cx.begin();
            if let Some(value) = value {
                cx.propose(value);
            }
        }
        (EditEvent::Update { .. }, Some(value)) => cx.propose(value),
        (EditEvent::Commit { .. }, Some(value)) => {
            cx.propose(value);
            cx.commit();
        }
        (EditEvent::Cancel { reason, .. }, _) => cx.cancel(*reason),
        _ => {}
    }
}

/// The gradient's stops for the ramp painter, held flat before the first and after the last, mirrored for right to
/// left.
pub(super) fn ramp_stops(preview: &GradientPreview, rtl: bool) -> Vec<(f32, DataColor)> {
    let mut stops: Vec<(f32, Color32)> = preview.stops.iter().map(|(at, color)| (at.clamp(0.0, 1.0), Color32::from(*color))).collect();
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    if rtl {
        stops = stops.into_iter().rev().map(|(at, color)| (1.0 - at, color)).collect();
    }
    stops.into_iter().map(|(at, color)| (at, DataColor::new(color))).collect()
}

/// Paints a pattern tiled across `well` at the tile's pixel size, or a picture stretched to it.
pub(super) fn paint_image(painter: &ContentPainterCx<'_>, well: Rect, texture: CachedTexture, tiled: bool) {
    let whole = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let tile = texture.size_vec2() / painter.painter.pixels_per_point();
    let tiled = tiled && tile.x > 0.0 && tile.y > 0.0 && tile.x.is_finite() && tile.y.is_finite();
    let (cols, rows) = if tiled { ((well.width() / tile.x).ceil(), (well.height() / tile.y).ceil()) } else { (1.0, 1.0) };
    if !tiled || cols * rows > MAX_TILES {
        painter.picture(texture.id, well, whole, 0.0);
        return;
    }
    for row in 0..rows as usize {
        for col in 0..cols as usize {
            let cell = Rect::from_min_size(well.min + vec2(col as f32 * tile.x, row as f32 * tile.y), tile).intersect(well);
            let uv_max: Pos2 = pos2(cell.width() / tile.x, cell.height() / tile.y);
            painter.picture(texture.id, cell, Rect::from_min_max(pos2(0.0, 0.0), uv_max), 0.0);
        }
    }
}
