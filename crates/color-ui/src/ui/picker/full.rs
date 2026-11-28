//! The Full layout of `ColorPicker`: the Color Picker dialog's fixed arrangement.

use craft_behavior::{ColumnWidth, Mixed, NavShape};
use craft_theme::{ColorModel, HsbModel, RgbModel};
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, FieldLayout, FooterSlot, Gap, GlyphLabel, LabelKind, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing};
use craft_ui::display::{Shade, Text};
use craft_ui::form::Field;
use craft_ui::layout::{CellAlign, GridFit, RowAlign, empty_cell};
use craft_ui::layout_components::{Grid, Row, Stack};
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::text::{HexField, NumberField, UnitPlacement};
use egui::Ui;

use super::channel::{ChannelRule, ChannelUnit, channel_name};
use super::compact::Body;
use super::{Collected, ColorPicker, ColorPickerWords, Pass, PickedColor};
use crate::ui::{AreaMode, AreaShape, CompareSwatch};

/// Two columns that never stack: the two halves of the picker, the right half (compare block and actions over the two
/// field groups), each field group (the radio cell, then the channel's letter, field and unit), and the dialog's Color
/// Swatches view (the swatches, then the buttons' cell).
pub(crate) const TWO_COLUMNS: &[ColumnWidth] = &[ColumnWidth::Content, ColumnWidth::Content];

/// The models the picker shows and the area modes it offers.
#[derive(Clone, Copy)]
struct ModelChoice<'m> {
    visible: &'m [&'m dyn ColorModel],
    offered: &'m [AreaMode],
    words: &'m ColorPickerWords,
}

/// Whether `model` stands in the first field group: HSB and RGB. Lab, OKLCH and the host's models stand in the second.
fn in_first_group(model: &dyn ColorModel) -> bool {
    model.id() == HsbModel.id() || model.id() == RgbModel.id()
}

impl<'a> Body<'a> {
    /// The Color Picker dialog, one arrangement in every room: the square area and the strip over Only Web Colors and
    /// the screen-pick button; beside them the compare block with its warnings and the actions cell (the dialog's
    /// buttons, empty outside a dialog), over the two field groups, where radios choose what the area shows.
    pub(super) fn full(&self, ui: &mut Ui, pass: &Pass<'a>, got: &mut Collected) {
        let words = Env::of(ui).words::<ColorPickerWords>();
        let visible: Vec<&dyn ColorModel> = self.models.iter().copied().chain([pass.mode.model()]).collect();
        // The two halves in a grid that never stacks or squeezes, so each keeps its width in any room: the Only Web Colors
        // row spans the area and the strip, its button under the strip's end.
        Grid::new(TWO_COLUMNS).fit(GridFit::Fixed).cells(CellAlign::Top).gap(Gap::Section).show(ui, |ui, _| {
            Stack::new().gap(Gap::Item).show(ui, |ui, _| {
                self.area_and_strip(ui, pass, &visible, AreaShape::Square, got);
                // The check box at the row's start; the rest of the row puts the screen-pick button at its end, under the
                // strip.
                Row::new().fill_width().show(ui, |ui, _| {
                    let check = Checkbox::new(Bind::given(pass.web)).label(words.only_web_colors.as_str().to_owned()).show(ui);
                    if let Some(on) = check.value() {
                        got.web_pick = Some(*on);
                    }
                    Row::new().fill_width().align(RowAlign::End).show(ui, |ui, _| self.sample_button(ui, &words, got));
                });
            });
            self.right_half(ui, pass, &visible, &words, got);
        });
    }

    /// The compare block and the actions cell over the two field groups, in a grid whose columns never stack: the
    /// actions cell stands over the second group, and both groups start under the taller of the two cells. The radios
    /// of "Field shows" are one group across both.
    fn right_half(&self, ui: &mut Ui, pass: &Pass<'a>, visible: &[&dyn ColorModel], words: &ColorPickerWords, got: &mut Collected) {
        let offered = ColorPicker::offered(&self.models);
        let choice = ModelChoice { visible, offered: &offered, words };
        let (first, second): (Vec<&'a dyn ColorModel>, Vec<&'a dyn ColorModel>) = self.models.iter().copied().partition(|m| in_first_group(*m));
        let group = RadioGroup::new(words.field_shows.as_str().to_owned(), Bind::given(pass.mode)).nav(NavShape::Spatial);
        let picked = group
            .show(ui, |ui, _| {
                Grid::new(TWO_COLUMNS).fit(GridFit::Fixed).cells(CellAlign::Top).gap(Gap::Section).show(ui, |ui, _| {
                    self.compare_block(ui, pass, words, got);
                    let _ = FooterSlot::new().show(ui);
                    self.field_group(ui, pass, &first, choice, true, got);
                    self.field_group(ui, pass, &second, choice, false, got);
                });
            })
            .output;
        if let Some(mode) = picked.value() {
            got.mode_pick = Some(*mode);
        }
    }

    /// The compare block, the colour being edited over the one the picker started from, with the warnings column after
    /// it; a click on the current half or on a warning sets that colour. The Compact layout shows it beside its area
    /// when the host gives a compare colour.
    pub(super) fn compare_block(&self, ui: &mut Ui, pass: &Pass<'a>, words: &ColorPickerWords, got: &mut Collected) {
        let current = match pass.compare {
            Mixed::Same(current) => Mixed::Same(current.color),
            Mixed::Mixed => Mixed::Mixed,
        };
        let shown = CompareSwatch::new(words.new_color.as_str().to_owned(), pass.shows(pass.shown.color), current).warnings(&self.warnings).show(ui);
        if shown.restore
            && let Mixed::Same(current) = pass.compare
        {
            got.clicks.push(current);
        }
        if let Some(warning) = shown.warning.and_then(|at| self.warnings.get(at)) {
            got.clicks.push(PickedColor::rgb(warning.nearest()));
        }
    }

    /// One field group: a grid of the radio cell and the channel cell (letter, field, unit), one row per channel of
    /// each of `models`, an empty row between two models, and in the first group the hex row: "#" in the radio column
    /// and the hex field, six digits wide, from the letter column on.
    fn field_group(&self, ui: &mut Ui, pass: &Pass<'a>, models: &[&'a dyn ColorModel], choice: ModelChoice<'_>, hex: bool, got: &mut Collected) {
        Grid::new(TWO_COLUMNS).fit(GridFit::Fixed).gap(Gap::Row).show(ui, |ui, _| {
            for (at, model) in models.iter().enumerate() {
                // An empty row stands between two models, as Photoshop's gap between HSB and RGB.
                if at > 0 {
                    empty_cell(ui);
                    empty_cell(ui);
                }
                self.model_rows(ui, pass, *model, choice, got);
            }
            if hex {
                Text::new(Label::from(HexField::HASH)).shade(Shade::Muted).show(ui);
                // Six digits wide from the letter column on; the content-sized row keeps it from stretching to the cell.
                Row::new().show(ui, |ui, _| self.hex_field(ui, pass, choice.words, Some(Sizing::Content), got));
            }
        });
    }

    /// The rows of one model's channels, two cells each: the radio that chooses the area mode (empty where the channel
    /// offers none), then the channel's number field with its letter before it and its unit after it. The radio's
    /// accessible name is the channel's name.
    fn model_rows(&self, ui: &mut Ui, pass: &Pass<'a>, model: &'a dyn ColorModel, choice: ModelChoice<'_>, got: &mut Collected) {
        let ModelChoice { visible, offered, words } = choice;
        let values = pass.shown.values_in(model);
        for (i, channel) in model.channels().iter().enumerate() {
            let name = channel_name(model, i, visible);
            match offered.iter().find(|mode| mode.model().id() == model.id() && mode.strip() == i).copied() {
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
                None => empty_cell(ui),
            }
            let letter = words.grid_label.fill(&[("label", channel.label)]);
            let letter: &str = &letter;
            let rule = ChannelRule::of(channel);
            let value = f64::from(values.values[i]);
            // The unit is the field's kind (typed units, decimals) and its text stands after the field.
            let (kind, unit) = match ChannelUnit::of(channel.unit) {
                ChannelUnit::None => (None, None),
                ChannelUnit::Percent => (Some(UnitKind::Percent), Some(Env::of(ui).units().symbol(UnitKind::Percent))),
                ChannelUnit::Degrees => (Some(UnitKind::Degrees), Some(Env::of(ui).units().symbol(UnitKind::Degrees))),
                ChannelUnit::Text(text) => (None, Some(text.to_owned())),
            };
            // The letter is the field's glyph label, so the letters of a group share a column and the fields (one width
            // through the number fields' value column) start at one x, their units after them; the content-sized row
            // keeps the field from stretching to the cell, which the hex field spans.
            Row::new().show(ui, |ui, _| {
                let side = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text(Label::from(letter.to_owned()))));
                let edit = Field::new(name.clone()).side(side).sizing(Sizing::Content).show(ui, |ui, _| {
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
                    field.show(ui).edit
                });
                if let Some(unit) = &unit {
                    Text::new(Label::from(unit.clone())).shade(Shade::Muted).show(ui);
                }
                if let Some(edit) = edit.inner {
                    got.adopt(edit, pass.shown, |v, mine| {
                        if let Some(v) = v {
                            *mine = mine.with_channel(model, i, *v, &pass.rules);
                        }
                    });
                }
            });
        }
    }
}
