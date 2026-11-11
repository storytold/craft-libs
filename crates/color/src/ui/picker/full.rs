//! The Full layout of `ColorPicker` (the Color Picker dialog), and the warnings beside its compare swatch.

use craft_behavior::{ColumnWidth, IconId, Metrics, Mixed, NavShape, Sense};
use craft_theme::{ColorModel, HsbModel, OklchModel, RgbModel, Srgba};
use craft_ui::choice::{Checkbox, Radio, RadioGroup, Swatch};
use craft_ui::contract::{
    Bind, Common, Component, Cx, GlyphLabel, LabelKind, MeasureCx, Outcome, PaintUi, Props as _, Role, Show as _, ShowContainer as _, SideLabel,
};
use craft_ui::display::{Icon, Shade, Text};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::{Grid, Row, Stack};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::{ComponentStyle, StyleCx, TypeRole};
use craft_ui::text::NumberField;
use egui::{Rect, Response, Ui, Vec2};

use super::channel::{ChannelRule, ChannelUnit, channel_name};
use super::compact::Body;
use super::{Collected, ColorPicker, ColorPickerWords, Pass};
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

/// The style of a warning chip: the icon over the chip, and their sizes.
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
/// over a swatch of the nearest passing colour. The host applies [`ColorWarning::nearest`] when it is clicked. The tip
/// is the tooltip and the accessible name.
#[must_use = "show the chip with `show`"]
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct ColorWarningChip<'a> {
    #[common]
    common: Common,
    warning: &'a ColorWarning,
    clicked: bool,
}

impl<'a> ColorWarningChip<'a> {
    /// A chip for `warning`.
    pub fn new(warning: &'a ColorWarning) -> Self {
        ColorWarningChip { common: Common::new(), warning, clicked: false }.label(warning.tip.clone())
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
        Vec2::new(cx.style.icon.max(cx.style.chip), cx.style.icon + cx.style.gap + cx.style.chip)
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let icon = Rect::from_center_size(egui::pos2(cx.rect.center().x, cx.rect.top() + style.icon / 2.0), Vec2::splat(style.icon));
        let chip = Rect::from_center_size(egui::pos2(cx.rect.center().x, cx.rect.bottom() - style.chip / 2.0), Vec2::splat(style.chip));
        let _ = ui.child("icon", icon, Icon::new(IconSpec::svg(self.warning.icon.clone())));
        let swatch = Swatch::new(self.warning.tip.clone(), self.warning.nearest).tooltip(self.warning.tip.clone());
        self.clicked = ui.child("chip", chip, swatch).clicked;
    }

    fn finish(self, cx: Cx<'_, Self>) -> ColorWarningChipOutput {
        ColorWarningChipOutput { response: cx.response.clone(), clicked: self.clicked }
    }
}

/// Columns that take the width of their content: the Full layout's two halves, and its channel grid of one to three
/// columns. A group of fields that fills the width would take all of a row's room otherwise.
const HALVES: &[ColumnWidth] = &[ColumnWidth::Content, ColumnWidth::Content];
const ONE_COLUMN: &[ColumnWidth] = &[ColumnWidth::Content];
const THREE_COLUMNS: &[ColumnWidth] = &[ColumnWidth::Content, ColumnWidth::Content, ColumnWidth::Content];

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
        Grid::new(HALVES).show(ui, |ui, _| {
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
                                    got.clicks.push(warning.nearest);
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
    fn compare_swatch(&self, ui: &mut Ui, pass: &Pass<'a>, current: Mixed<Srgba>, words: &ColorPickerWords, got: &mut Collected) {
        let caption = |text: &str| Text::new(text.to_owned()).role(TypeRole::Caption).shade(Shade::Muted);
        Stack::new().show(ui, |ui, _| {
            caption(words.new_caption.as_str()).show(ui);
            Swatch::new(words.new_color.as_str().to_owned(), pass.shows(pass.shown.color)).fill().show(ui);
            let restore = words.restore.as_str().to_owned();
            if Swatch::new(restore.clone(), current).tooltip(restore).fill().show(ui).clicked
                && let Mixed::Same(current) = current
            {
                got.clicks.push(current);
            }
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
                let widths = match columns.len() {
                    1 => ONE_COLUMN,
                    2 => &THREE_COLUMNS[..2],
                    _ => THREE_COLUMNS,
                };
                Grid::new(widths).show(ui, |ui, _| {
                    for (column, models) in columns.iter().enumerate() {
                        FieldGroup::new(words.channels.as_str().to_owned()).show(ui, |ui, _| {
                            for model in models {
                                self.model_fields(ui, pass, *model, ModelChoice { visible, offered: &offered }, words, got);
                            }
                            if column == 0 {
                                let label = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text("#")));
                                Field::new(words.hex.as_str().to_owned()).side(label).show(ui, |ui, _| self.hex_field(ui, pass, words, got));
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

    /// The rows of one model's channels.
    fn model_fields(
        &self,
        ui: &mut Ui,
        pass: &Pass<'a>,
        model: &'a dyn ColorModel,
        choice: ModelChoice<'_>,
        words: &ColorPickerWords,
        got: &mut Collected,
    ) {
        let ModelChoice { visible, offered } = choice;
        let values = pass.shown.values_in(model);
        for (i, channel) in model.channels().iter().enumerate() {
            let name = channel_name(model, i, visible);
            let short = words.grid_label.fill(&[("label", channel.label)]);
            let short: &str = &short;
            let label = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text(Label::from(short.to_owned()))));
            let mode = offered.iter().find(|mode| mode.model().id() == model.id() && mode.strip() == i).copied();
            let rule = ChannelRule::of(channel);
            let unit = ChannelUnit::of(channel.unit);
            let value = f64::from(values.values[i]);
            let edit = Field::new(name.clone())
                .side(label)
                .show(ui, |ui, _| {
                    Row::new()
                        .show(ui, |ui, _| {
                            if let Some(mode) = mode {
                                Radio::new(mode).label(name.clone()).show(ui);
                            }
                            let field = NumberField::new(name.clone(), Bind::given(pass.shows(Some(value))))
                                .range(rule.range.clone())
                                .step(rule.step)
                                .snap_typed(true)
                                .commit_decimals(usize::from(rule.decimals));
                            let field = match unit {
                                ChannelUnit::None => field,
                                ChannelUnit::Percent => field.unit_kind(craft_ui::runtime::env::UnitKind::Percent),
                                ChannelUnit::Degrees => field.unit_kind(craft_ui::runtime::env::UnitKind::Degrees),
                                ChannelUnit::Text(text) => field.unit(Label::from(text)),
                            };
                            field.show(ui).edit
                        })
                        .inner
                })
                .inner
                .flatten();
            if let Some(edit) = edit {
                got.adopt(edit, pass.shown, |v, mine| {
                    if let Some(v) = v {
                        *mine = mine.with_channel(model, i, *v, &pass.rules);
                    }
                });
            }
        }
    }
}
