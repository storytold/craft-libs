//! [`CompareSwatch`]: the Color Picker's compare block, the colour being edited over the colour it started from.

use craft_behavior::{Metrics, Mixed, PartKey, Sense};
use craft_theme::Srgba;
use craft_ui::contract::{Access, Common, Component, Cx, MeasureCx, Name, Outcome, PaintUi, PartsCx, Props as _, Role};
use craft_ui::display::{Shade, Text};
use craft_ui::primitives::{PaintShape, StrokeWidth, TextRun};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, FocusLook, StyleColor, StyleCx, TypeRole};
use egui::{CornerRadius, Rect, Response, Vec2, pos2};

use super::picker::ColorPickerWords;
use super::warning::{ColorWarning, ColorWarningChip};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The style of a compare block: the size of the block and of each half, the edge round it and the gap between a
/// caption and the block.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CompareSwatchStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The focus ring of the current half.
    #[style(focus)]
    focus: FocusLook,
    /// The block's width: `size.thumb.sample.md`.
    width: f32,
    /// Each half's height: `size.thumb.sample.sm`, so the block is taller than wide.
    half: f32,
    /// The one edge round the block.
    edge: StyleColor,
    /// Between a caption and the block, the block and the warnings column, and two warnings.
    gap: f32,
    /// The warnings column's width, and the side of a warning's icon and of its chip: `size.icon.sm`.
    warning: f32,
}

/// What a [`CompareSwatch`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct CompareSwatchOutput {
    /// The block's response.
    pub response: Response,
    /// The current half was activated by click, Enter or Space: the host sets the current colour again.
    pub restore: bool,
    /// The index of the warning whose chip was activated: the host sets its nearest colour.
    pub warning: Option<usize>,
}

impl Outcome for CompareSwatchOutput {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        craft_ui::testing::StoryOutput { id: Some(self.response.id), clicked: self.restore || self.warning.is_some(), ..Default::default() }
    }
}

/// Photoshop's compare block beside the Color Picker's field: one block of two touching halves inside one edge, the
/// colour being edited (`new`) over the colour the picker started from (`current`), with the caption "new" above it
/// and "current" under it. A translucent colour shows over the checkerboard; a mixed one shows the mixed face.
///
/// After the block stands its warnings column, `size.icon.sm` wide whether or not a warning shows, so a warning that
/// appears moves nothing: each [`warning`](CompareSwatch::warnings) is a [`ColorWarningChip`] (its icon over its chip),
/// the first level with the block's top.
///
/// - The new half is a picture: AccessKit names the block by its label ("New color") and gives the new colour's hex
///   as its value.
/// - The current half is a part of the block that is a button named "Restore current color", which is also its
///   tooltip: a click, Enter or Space on it reports [`restore`](CompareSwatchOutput::restore), and the host sets the
///   current colour again.
#[must_use = "show the block with `show`"]
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct CompareSwatch<'a> {
    #[common]
    common: Common,
    new: Mixed<Srgba>,
    current: Mixed<Srgba>,
    warnings: &'a [ColorWarning],
    restore: bool,
    warning: Option<usize>,
}

impl<'a> CompareSwatch<'a> {
    /// A block named `label` ("New color", the new half's name) showing `new` over `current`.
    pub fn new(label: impl Into<Label>, new: impl Into<Mixed<Srgba>>, current: impl Into<Mixed<Srgba>>) -> Self {
        CompareSwatch { common: Common::new(), new: new.into(), current: current.into(), warnings: &[], restore: false, warning: None }.label(label)
    }

    /// The warnings in the column after the block, top to bottom.
    pub fn warnings(mut self, warnings: &'a [ColorWarning]) -> Self {
        self.warnings = warnings;
        self
    }

    /// The height of one caption line.
    fn caption_height(cx: &MeasureCx<'_, Self>, text: &str) -> f32 {
        cx.layout(&TextRun::new(text).role(TypeRole::Caption)).size().y
    }
}

/// The current half: a button that restores the current colour.
const CURRENT: PartKey = PartKey::new("current");

/// The height of each caption in a block of height `height`: the captions share what the halves leave evenly.
fn caption_of(height: f32, style: &CompareSwatchStyle) -> f32 {
    ((height - 2.0 * style.half) / 2.0 - style.gap).max(0.0)
}

/// The block's rect and its two halves at the start of `rect`, under a caption `caption` high and the gap, in reading
/// order (the caller mirrors them right to left).
fn halves(rect: Rect, style: &CompareSwatchStyle, caption: f32) -> (Rect, Rect, Rect) {
    let top = rect.top() + caption + style.gap;
    let left = rect.left();
    let new = Rect::from_min_size(pos2(left, top), Vec2::new(style.width, style.half));
    let current = Rect::from_min_size(pos2(left, new.bottom()), Vec2::new(style.width, style.half));
    (new.union(current), new, current)
}

impl Component for CompareSwatch<'_> {
    const ROLE: Role = Role::IMAGE;
    const OVERLAYS: bool = false;
    type Style = CompareSwatchStyle;
    type Output = CompareSwatchOutput;

    fn style(cx: &StyleCx<'_>, _: CompareSwatchStyleKey) -> CompareSwatchStyle {
        let tokens = cx.tokens;
        CompareSwatchStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            width: tokens.size.thumb.sample.md,
            half: tokens.size.thumb.sample.sm,
            edge: cx.color(tokens.color.line.control_edge),
            gap: tokens.space.gap.tight,
            warning: tokens.size.icon.sm,
        }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let words = cx.env.words::<ColorPickerWords>();
        let caption = Self::caption_height(cx, words.new_caption.as_str()).max(Self::caption_height(cx, words.current_caption.as_str()));
        let style = cx.style;
        Vec2::new(style.width + style.gap + style.warning, 2.0 * (caption + style.gap + style.half))
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let restore = cx.env.words::<ColorPickerWords>().restore.as_str().to_owned();
        let flow = craft_behavior::Flow::new(rect, cx.flow.is_rtl());
        let (_, _, current) = halves(rect, cx.style, caption_of(rect.height(), cx.style));
        cx.part(CURRENT, flow.to_physical(current)).sense(Sense::CLICK | Sense::FOCUS).focusable().tooltip(restore);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.restore = cx.part_activated(CURRENT).is_some();
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if let Some(color) = self.new.same() {
            access.value(color.to_hex());
        }
        access.node(CURRENT, Role::BUTTON).name = Name::Text(cx.env.words::<ColorPickerWords>().restore.as_str().to_owned());
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let words = cx.env.words::<ColorPickerWords>();
        let style = cx.style;
        let caption = caption_of(cx.rect.height(), style);
        let flow = craft_behavior::Flow::new(cx.rect, cx.flow.is_rtl());
        let (block, new, current) = halves(cx.rect, style, caption);
        let caption_text = |text: &str| Text::new(Label::from(text.to_owned())).role(TypeRole::Caption).shade(Shade::Muted);
        let above = Rect::from_min_max(pos2(block.left(), cx.rect.top()), pos2(block.right(), block.top() - style.gap));
        let under = Rect::from_min_max(pos2(block.left(), block.bottom() + style.gap), pos2(block.right(), cx.rect.bottom()));
        let (block, new, current, above, under) =
            (flow.to_physical(block), flow.to_physical(new), flow.to_physical(current), flow.to_physical(above), flow.to_physical(under));
        let _ = ui.child("new_caption", above, caption_text(words.new_caption.as_str()));
        ui.draw().swatch_well(new, CornerRadius::ZERO, self.new.same().copied());
        ui.draw().swatch_well(current, CornerRadius::ZERO, self.current.same().copied());
        let _ = ui.child("current_caption", under, caption_text(words.current_caption.as_str()));
        ui.draw().stroke(&PaintShape::Rect(block, CornerRadius::ZERO), StrokeWidth::Thin, style.edge);
        // The warnings column: each chip its icon over its swatch, the first level with the block's top.
        let column = cx.rect.left() + style.width + style.gap;
        let tall = style.warning + style.gap + style.warning;
        for (at, warning) in self.warnings.iter().enumerate() {
            let top = block.top() + at as f32 * (tall + style.gap);
            let chip = flow.to_physical(Rect::from_min_size(pos2(column, top), Vec2::new(style.warning, tall)));
            if ui.child(("warning", at), chip, ColorWarningChip::new(warning)).clicked {
                self.warning = Some(at);
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> CompareSwatchOutput {
        CompareSwatchOutput { response: cx.response.clone(), restore: self.restore, warning: self.warning }
    }
}
