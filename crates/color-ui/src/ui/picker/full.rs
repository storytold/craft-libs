//! The Full layout of `ColorPicker` (the Color Picker dialog), and the warnings beside its compare swatch.

use craft_behavior::{ColumnWidth, IconId, Metrics, Mixed, NavShape, Sense};
use craft_theme::{ColorModel, HsbModel, OklchModel, RgbModel, Srgba};
use craft_ui::choice::{Checkbox, Radio, RadioGroup, Swatch};
use craft_ui::contract::{
    Bind, Common, Component, Cx, FieldLayout, Gap, LabelKind, MeasureCx, Outcome, PaintUi, Props as _, Role, Show as _, ShowContainer as _,
    SideLabel, Sizing,
};
use craft_ui::display::{Icon, Shade, Text};
use craft_ui::layout_components::{Grid, Row, Stack};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::style::{ComponentStyle, StyleCx, TypeRole};
use craft_ui::text::{HexField, NumberField, UnitPlacement};
use egui::{Rect, Response, Ui, Vec2};

use super::channel::{ChannelRule, ChannelUnit, channel_name};
use super::compact::Body;
use super::{Collected, ColorPicker, ColorPickerWords, Pass, PickedColor};
use crate::ui::AreaMode;

/// A warning the host shows beside the Full picker's compare swatch, as the out-of-gamut and web-colour warnings of
/// Photoshop's Color Picker: an icon over a chip of the nearest colour that passes. A click on it sets that colour;
/// AccessKit and the tooltip name it by its tip.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorWarning {
    tip: Label,
    nearest: Srgba,
    icon: IconId,
}

/// The models the picker shows and the area modes it offers.
#[derive(Clone, Copy)]
struct ModelChoice<'m> {
    visible: &'m [&'m dyn ColorModel],
    offered: &'m [AreaMode],
    words: &'m ColorPickerWords,
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

    /// The icon over the chip.
    #[must_use]
    pub fn icon(mut self, icon: IconId) -> Self {
        self.icon = icon;
        self
    }
}

/// The style of a warning chip: the icon over the chip or beside it, and their sizes.
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

/// A [`ColorWarning`] as the Full picker draws it, for a host that draws it beside its own swatch: the warning's icon
/// over a swatch of the nearest passing colour, or with [`beside`](ColorWarningChip::beside) the icon and a swatch of
/// its size on one line, as Photoshop's Color panel shows it under the wells. The host applies
/// [`ColorWarning::nearest`] when it is clicked. The tip is the tooltip and the accessible name.
#[must_use = "show the chip with `show`"]
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct ColorWarningChip<'a> {
    #[common]
    common: Common,
    warning: &'a ColorWarning,
    /// The icon and a swatch of the icon's size on one line, in place of the icon over a control-sized swatch.
    #[prop]
    beside: bool,
    clicked: bool,
}

impl<'a> ColorWarningChip<'a> {
    /// A chip for `warning`.
    pub fn new(warning: &'a ColorWarning) -> Self {
        ColorWarningChip { common: Common::new(), warning, beside: false, clicked: false }.label(warning.tip.clone())
    }
}

impl Component for ColorWarningChip<'_> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = ColorWarningChipStyle;
    type Output = ColorWarningChipOutput;

    fn style(cx: &StyleCx<'_>, _: ColorWarningChipStyleKey) -> ColorWarningChipStyle {
        ColorWarningChipStyle { metrics: Metrics::new(), icon: cx.tokens.size.icon.md, chip: cx.control().height, gap: cx.tokens.space.gap.tight }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let style = cx.style;
        if self.beside {
            return Vec2::new(style.icon + style.gap + style.icon, style.icon);
        }
        Vec2::new(style.icon.max(style.chip), style.icon + style.gap + style.chip)
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let flow = craft_behavior::Flow::new(cx.rect, cx.flow.is_rtl());
        let (icon, chip) = if self.beside {
            let side = Vec2::splat(style.icon);
            let icon = Rect::from_min_size(cx.rect.min, side);
            let chip = Rect::from_min_size(egui::pos2(cx.rect.min.x + style.icon + style.gap, cx.rect.min.y), side);
            (flow.to_physical(icon), flow.to_physical(chip))
        } else {
            let icon = Rect::from_center_size(egui::pos2(cx.rect.center().x, cx.rect.top() + style.icon / 2.0), Vec2::splat(style.icon));
            let chip = Rect::from_center_size(egui::pos2(cx.rect.center().x, cx.rect.bottom() - style.chip / 2.0), Vec2::splat(style.chip));
            (icon, chip)
        };
        let _ = ui.child("icon", icon, Icon::new(IconSpec::svg(self.warning.icon.clone())));
        let swatch = Swatch::new(self.warning.tip.clone(), self.warning.nearest).tooltip(self.warning.tip.clone());
        self.clicked = ui.child("chip", chip, swatch).clicked;
    }

    fn finish(self, cx: Cx<'_, Self>) -> ColorWarningChipOutput {
        ColorWarningChipOutput { response: cx.response.clone(), clicked: self.clicked }
    }
}

/// The columns of one channel group: radio, letter, and the value cell (field and unit), each as wide as its content.
const THREE_COLUMNS: &[ColumnWidth] = &[ColumnWidth::Content, ColumnWidth::Content, ColumnWidth::Content, ColumnWidth::Content];

/// A grid column of the Full layout: HSB and RGB, then Lab and the host's models, then OKLCH.
fn column_of(model: &dyn ColorModel) -> usize {
    match model.id() {
        id if id == HsbModel.id() || id == RgbModel.id() => 0,
        id if id == OklchModel.id() => 2,
        _ => 1,
    }
}

impl<'a> Body<'a> {
    /// The Color Picker dialog: the area and strip over the check box, the opacity row and the screen-pick button;
    /// beside them the compare swatch and the warnings over the channel grid, where radios choose what the area shows.
    pub(super) fn full(&self, ui: &mut Ui, pass: &Pass<'a>, got: &mut Collected) {
        let words = Env::of(ui).words::<ColorPickerWords>();
        let visible: Vec<&dyn ColorModel> = self.models.iter().copied().chain([pass.mode.model()]).collect();
        Row::new().wrap(true).gap(Gap::Section).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                self.area_and_strip(ui, pass, &visible, got);
                if self.has_web_check {
                    let check = Checkbox::new(Bind::given(pass.web)).label(words.only_web_colors.as_str().to_owned()).show(ui);
                    if let Some(on) = check.value() {
                        got.web_pick = Some(*on);
                    }
                }
                self.alpha_row(ui, pass, &words, got);
                self.sample_button(ui, &words, got);
            });
            Stack::new().show(ui, |ui, _| {
                if self.compare.is_some() || !self.warnings.is_empty() {
                    Row::new().show(ui, |ui, _| {
                        if let Some(current) = self.compare {
                            self.compare_swatch(ui, pass, current, &words, got);
                        }
                        Stack::new().show(ui, |ui, _| {
                            for warning in &self.warnings {
                                if ColorWarningChip::new(warning).show(ui).clicked {
                                    got.clicks.push(PickedColor { color: warning.nearest, model: None });
                                }
                            }
                        });
                    });
                }
                self.grid(ui, pass, &visible, &words, got);
            });
        });
    }

    /// The compare swatch: the colour being edited over the current one, with their captions; a click on the current
    /// half sets it.
    fn compare_swatch(&self, ui: &mut Ui, pass: &Pass<'a>, current: Mixed<PickedColor>, words: &ColorPickerWords, got: &mut Collected) {
        let caption = |text: &str| Text::new(text.to_owned()).role(TypeRole::Caption).shade(Shade::Muted);
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            caption(words.new_caption.as_str()).show(ui);
            Stack::new().gap(Gap::None).show(ui, |ui, _| {
                Swatch::new(words.new_color.as_str().to_owned(), pass.shows(pass.shown.color)).sizing(Sizing::FillWidth).show(ui);
                let restore = words.restore.as_str().to_owned();
                let shown = match current {
                    Mixed::Same(current) => Mixed::Same(current.color),
                    Mixed::Mixed => Mixed::Mixed,
                };
                if Swatch::new(restore.clone(), shown).tooltip(restore).sizing(Sizing::FillWidth).show(ui).clicked
                    && let Mixed::Same(current) = current
                {
                    got.clicks.push(current);
                }
            });
            caption(words.current_caption.as_str()).show(ui);
        });
    }

    /// The channel grid: one column of fields per group of models, each column sharing its label column, with the
    /// radios of "Field shows" in front of the channels that choose an area mode, and the hex field under the first.
    fn grid(&self, ui: &mut Ui, pass: &Pass<'a>, visible: &[&dyn ColorModel], words: &ColorPickerWords, got: &mut Collected) {
        let offered = ColorPicker::offered(&self.models);
        let group = RadioGroup::new(words.field_shows.as_str().to_owned(), Bind::given(pass.mode)).nav(NavShape::Spatial);
        let picked = group
            .show(ui, |ui, _| {
                let columns: Vec<Vec<&'a dyn ColorModel>> = (0..3)
                    .map(|column| self.models.iter().copied().filter(|m| column_of(*m) == column).collect::<Vec<_>>())
                    .enumerate()
                    .filter(|(column, models)| *column == 0 || !models.is_empty())
                    .map(|(_, models)| models)
                    .collect();
                // The groups sit side by side from the top; each is its own grid of radio, letter and value cells, so the
                // fields of a group share one x whether or not their row has a radio.
                Row::new().gap(Gap::Section).show(ui, |ui, _| {
                    for (column, models) in columns.iter().enumerate() {
                        Grid::new(THREE_COLUMNS).show(ui, |ui, _| {
                            for model in models {
                                self.model_fields(ui, pass, *model, ModelChoice { visible, offered: &offered, words }, got);
                            }
                            if column == 0 {
                                Text::new(Label::from("")).show(ui);
                                Text::new(Label::from(HexField::HASH)).shade(Shade::Muted).show(ui);
                                self.hex_field(ui, pass, words, Some(Sizing::Content), got);
                                Text::new(Label::from("")).show(ui);
                            }
                        });
                    }
                });
            })
            .output;
        if let Some(mode) = picked.value() {
            got.mode_pick = Some(*mode);
        }
    }

    /// The rows of one model's channels, three cells each: the radio that chooses the area mode (an empty cell where the
    /// channel offers none), the channel's letter and its number field. The radio's name stays its accessible name.
    fn model_fields(&self, ui: &mut Ui, pass: &Pass<'a>, model: &'a dyn ColorModel, choice: ModelChoice<'_>, got: &mut Collected) {
        let ModelChoice { visible, offered, words } = choice;
        let values = pass.shown.values_in(model);
        for (i, channel) in model.channels().iter().enumerate() {
            let name = channel_name(model, i, visible);
            let letter = words.grid_label.fill(&[("label", channel.label)]);
            let letter: &str = &letter;
            let mode = offered.iter().find(|mode| mode.model().id() == model.id() && mode.strip() == i).copied();
            match mode {
                Some(mode) => {
                    // A grid cell takes one child that is a component: the Row is the cell, the labelled radio its content.
                    Row::new().show(ui, |ui, cx| {
                        cx.labelled(
                            ui,
                            (model.id(), i),
                            name.clone(),
                            SideLabel::new().layout(FieldLayout::After).kind(LabelKind::Hidden),
                            Radio::new(mode),
                        );
                    });
                }
                None => {
                    Text::new(Label::from("")).show(ui);
                }
            }
            Text::new(Label::from(letter.to_owned())).shade(Shade::Muted).show(ui);
            let rule = ChannelRule::of(channel);
            let value = f64::from(values.values[i]);
            // The unit is the field's kind (typed units, decimals) and sits in its own cell after the field.
            let (kind, text) = match ChannelUnit::of(channel.unit) {
                ChannelUnit::None => (None, None),
                ChannelUnit::Percent => (Some(UnitKind::Percent), Some(Env::of(ui).units().symbol(UnitKind::Percent))),
                ChannelUnit::Degrees => (Some(UnitKind::Degrees), Some(Env::of(ui).units().symbol(UnitKind::Degrees))),
                ChannelUnit::Text(text) => (None, Some(text.to_owned())),
            };
            let field = NumberField::new(name.clone(), Bind::given(pass.shows(Some(value))))
                .range(rule.range.clone())
                .step(rule.step)
                .snap_typed(true)
                .commit_decimals(usize::from(rule.decimals))
                .fixed_decimals(usize::from(rule.decimals))
                .unit_placement(UnitPlacement::After)
                .sizing(Sizing::Content);
            let field = match kind {
                Some(kind) => field.unit_kind(kind),
                None => field,
            };
            got.adopt(field.show(ui).edit, pass.shown, |v, mine| {
                if let Some(v) = v {
                    *mine = mine.with_channel(model, i, *v, &pass.rules);
                }
            });
            Text::new(Label::from(text.unwrap_or_default())).shade(Shade::Muted).show(ui);
        }
    }
}
