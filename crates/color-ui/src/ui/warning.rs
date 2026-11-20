//! [`ColorWarning`] and [`ColorWarningChip`]: a warning about the colour shown beside a swatch (out of the printing
//! gamut, not a web colour), with a chip of the nearest colour that passes.

use craft_behavior::{IconId, Metrics, Sense};
use craft_theme::Srgba;
use craft_ui::choice::Swatch;
use craft_ui::contract::{Common, Component, Cx, MeasureCx, Outcome, PaintUi, Props as _, Role};
use craft_ui::display::Icon;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, StyleCx};
use egui::{Rect, Response, Vec2};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// A warning the host shows beside a colour's swatch, as the out-of-gamut and web-colour warnings of Photoshop's Color
/// Picker and Color panel: an icon and a chip of the nearest colour that passes. A click on it sets that colour;
/// AccessKit and the tooltip name it by its tip.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorWarning {
    pub(crate) tip: Label,
    nearest: Srgba,
    icon: IconId,
}

impl ColorWarning {
    /// A warning that says `tip` ("Out of gamut for printing") and offers `nearest`.
    pub fn new(tip: impl Into<Label>, nearest: Srgba) -> Self {
        ColorWarning { tip: tip.into(), nearest, icon: IconId::new("warning") }
    }

    /// A warning that the colour is not a web-safe colour (the cube Photoshop's Color Picker shows), offering
    /// `nearest`, the nearest web colour.
    pub fn web_colors(tip: impl Into<Label>, nearest: Srgba) -> Self {
        Self::new(tip, nearest).icon(IconId::new("web_colors"))
    }

    /// The colour that passes, nearest the warned one: what a click on the warning sets.
    pub fn nearest(&self) -> Srgba {
        self.nearest
    }

    /// The icon with the chip.
    #[must_use]
    pub fn icon(mut self, icon: IconId) -> Self {
        self.icon = icon;
        self
    }
}

/// How a [`ColorWarningChip`] arranges its icon and its chip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColorWarningArrangement {
    /// The icon over the chip, both `size.icon.sm`: the column beside the Color Picker's compare swatch.
    #[default]
    Stacked,
    /// The icon (`size.icon.md`) and the chip (`size.icon.sm`) on one line: the Color panel, under its wells.
    Beside,
}

/// The style of a warning chip: the sizes of its icon and chip, and the gap between them.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ColorWarningChipStyle {
    #[style(metrics)]
    metrics: Metrics,
    icon: f32,
    chip: f32,
    gap: f32,
}

/// What a [`ColorWarningChip`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ColorWarningChipOutput {
    /// The chip's response.
    pub response: Response,
    /// The chip was activated by click, Enter or Space: the host applies [`ColorWarning::nearest`].
    pub clicked: bool,
}

impl Outcome for ColorWarningChipOutput {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A [`ColorWarning`] as a picker or panel draws it beside its swatch: the warning's icon and a swatch of the nearest
/// passing colour, in its [`arrangement`](ColorWarningChip::arrangement). The host applies [`ColorWarning::nearest`]
/// when it is clicked. The tip is the tooltip and the accessible name.
#[must_use = "show the chip with `show`"]
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct ColorWarningChip<'a> {
    #[common]
    common: Common,
    warning: &'a ColorWarning,
    /// How the icon and the chip are arranged; the icon over the chip by default.
    #[style_key]
    #[prop]
    arrangement: ColorWarningArrangement,
    clicked: bool,
}

impl<'a> ColorWarningChip<'a> {
    /// A chip for `warning`.
    pub fn new(warning: &'a ColorWarning) -> Self {
        ColorWarningChip { common: Common::new(), warning, arrangement: ColorWarningArrangement::Stacked, clicked: false }.label(warning.tip.clone())
    }
}

impl Component for ColorWarningChip<'_> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = ColorWarningChipStyle;
    type Output = ColorWarningChipOutput;

    fn style(cx: &StyleCx<'_>, key: ColorWarningChipStyleKey) -> ColorWarningChipStyle {
        let size = &cx.tokens.size.icon;
        let icon = match key.arrangement {
            ColorWarningArrangement::Stacked => size.sm,
            ColorWarningArrangement::Beside => size.md,
        };
        ColorWarningChipStyle { metrics: Metrics::new(), icon, chip: size.sm, gap: cx.tokens.space.gap.tight }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let style = cx.style;
        match self.arrangement {
            ColorWarningArrangement::Stacked => Vec2::new(style.icon.max(style.chip), style.icon + style.gap + style.chip),
            ColorWarningArrangement::Beside => Vec2::new(style.icon + style.gap + style.chip, style.icon.max(style.chip)),
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let flow = craft_behavior::Flow::new(cx.rect, cx.flow.is_rtl());
        let (icon, chip) = match self.arrangement {
            ColorWarningArrangement::Stacked => {
                let icon = Rect::from_center_size(egui::pos2(cx.rect.center().x, cx.rect.top() + style.icon / 2.0), Vec2::splat(style.icon));
                let chip = Rect::from_center_size(egui::pos2(cx.rect.center().x, cx.rect.bottom() - style.chip / 2.0), Vec2::splat(style.chip));
                (icon, chip)
            }
            ColorWarningArrangement::Beside => {
                let middle = cx.rect.center().y;
                let icon = Rect::from_min_size(egui::pos2(cx.rect.min.x, middle - style.icon / 2.0), Vec2::splat(style.icon));
                let chip =
                    Rect::from_min_size(egui::pos2(cx.rect.min.x + style.icon + style.gap, middle - style.chip / 2.0), Vec2::splat(style.chip));
                (flow.to_physical(icon), flow.to_physical(chip))
            }
        };
        let _ = ui.child("icon", icon, Icon::new(IconSpec::svg(self.warning.icon.clone())));
        let swatch = Swatch::new(self.warning.tip.clone(), self.warning.nearest).tooltip(self.warning.tip.clone());
        self.clicked = ui.child("chip", chip, swatch).clicked;
    }

    fn finish(self, cx: Cx<'_, Self>) -> ColorWarningChipOutput {
        ColorWarningChipOutput { response: cx.response.clone(), clicked: self.clicked }
    }
}
