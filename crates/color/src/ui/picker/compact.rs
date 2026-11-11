//! The picker's body: the Compact layout, and the parts both layouts show (the area and strip, the hex field and the
//! screen-pick button). The Full layout is in `full.rs`.

use craft_behavior::Mixed;
use craft_theme::{ColorModel, Srgba};
use craft_ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft_ui::form::FieldGroup;
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::focus::{self, FocusTarget};
use craft_ui::text::TextField;
use egui::Ui;

use super::channel::{ChannelRow, ChannelRule, ChannelUnit, channel_name};
use super::screen_pick::ScreenPickButton;
use super::{Collected, ColorPickerLayout, ColorPickerWords, ColorWarning, Pass};
use crate::ui::channels::WEB_LEVELS;
use crate::ui::{AreaMode, ColorArea, ColorStrip};

/// What the body shows.
pub(super) struct Body<'a> {
    pub layout: ColorPickerLayout,
    pub models: Vec<&'a dyn ColorModel>,
    pub warnings: Vec<ColorWarning>,
    pub compare: Option<Mixed<Srgba>>,
    pub alpha: bool,
    pub dynamic_tracks: bool,
    pub eyedropper: bool,
    pub eyedropper_busy: bool,
    /// Whether the host holds Only Web Colors, so the Full layout shows its check box.
    pub has_web_check: bool,
}

impl<'a> Body<'a> {
    /// Fills the container's body; what the children report is left in `got`.
    pub(super) fn run(&self, ui: &mut Ui, pass: &Pass<'a>, got: &mut Collected) {
        match self.layout {
            ColorPickerLayout::Compact => self.compact(ui, pass, got),
            ColorPickerLayout::Full => self.full(ui, pass, got),
        }
    }

    /// The area and strip over the channel rows of one model, the model switch, the hex row and the opacity row.
    fn compact(&self, ui: &mut Ui, pass: &Pass<'a>, got: &mut Collected) {
        let words = Env::of(ui).words::<ColorPickerWords>();
        let model = self.models.iter().copied().find(|m| Some(m.id()) == pass.model);
        let visible: Vec<&dyn ColorModel> = model.into_iter().chain([pass.mode.model()]).collect();
        self.area_and_strip(ui, pass, &visible, got);
        if let Some(model) = model {
            FieldGroup::new(words.channels.as_str().to_owned()).show(ui, |ui, _| {
                let values = pass.shown.values_in(model);
                for (i, channel) in model.channels().iter().enumerate() {
                    let mut row = ChannelRow::new(
                        channel_name(model, i, &visible),
                        Bind::given(pass.shows(f64::from(values.values[i]))),
                        ChannelRule::of(channel),
                    )
                    .unit(ChannelUnit::of(channel.unit));
                    if self.dynamic_tracks {
                        row = row.track(model.track(i, &values));
                    }
                    if let Some(edit) = row.show(ui) {
                        got.adopt(edit, pass.shown, |v, mine| *mine = mine.with_channel(model, i, *v, &pass.rules));
                    }
                }
                self.alpha_row(ui, pass, &words, got);
            });
            if self.models.len() > 1 {
                let entries: Vec<Entry<&'static str>> = self.models.iter().map(|m| Entry::new(m.id(), Label::from(m.name()))).collect();
                let label = words.color_model.fill(&[("model", model.name())]);
                let out = Select::new(label, Bind::given(model.id()), &entries).show(ui);
                if let Some(picked) = out.value() {
                    got.model_pick = Some(*picked);
                }
            }
        }
        Row::new().show(ui, |ui, _| {
            self.hex_field(ui, pass, &words, got);
            self.sample_button(ui, &words, got);
        });
    }

    /// The area and the strip beside it, for the mode of the pass.
    pub(super) fn area_and_strip(&self, ui: &mut Ui, pass: &Pass<'a>, visible: &[&dyn ColorModel], got: &mut Collected) {
        let mode: AreaMode = pass.mode;
        let model = mode.model();
        let [across, down] = mode.area();
        let values = pass.shown.values_in(model);
        let levels = pass.rules.web.then_some(WEB_LEVELS);
        Row::new().show(ui, |ui, _| {
            let mut area = ColorArea::new(Label::from(mode.name()), mode, Bind::given(values))
                .names([channel_name(model, across, visible), channel_name(model, down, visible)]);
            let mut strip = ColorStrip::new(channel_name(model, mode.strip(), visible), mode, Bind::given(values));
            if let Some(levels) = levels {
                area = area.quantize(levels);
                strip = strip.quantize(levels);
            }
            let edit = area.show(ui);
            got.adopt(edit, pass.shown, |v, mine| *mine = mine.with_values(v, model, &pass.rules));
            let edit = strip.show(ui);
            got.adopt(edit, pass.shown, |v, mine| *mine = mine.with_values(v, model, &pass.rules));
        });
    }

    /// The "Opacity" row, 0 to 100 % over the checkerboard, when the picker edits alpha.
    pub(super) fn alpha_row(&self, ui: &mut Ui, pass: &Pass<'a>, words: &ColorPickerWords, got: &mut Collected) {
        if !self.alpha {
            return;
        }
        let opaque = pass.shown.color.with_alpha(1.0);
        let row = ChannelRow::new(words.opacity.as_str().to_owned(), Bind::given(pass.shows(pass.shown.alpha_percent())), ChannelRule::ALPHA)
            .unit(ChannelUnit::Percent)
            .track(vec![opaque.with_alpha(0.0), opaque])
            .checker(true);
        if let Some(edit) = row.show(ui) {
            got.adopt(edit, pass.shown, |v, mine| *mine = mine.with_alpha(*v, &pass.rules));
        }
    }

    /// The hex field. The Full layout's first show focuses it.
    pub(super) fn hex_field(&self, ui: &mut Ui, pass: &Pass<'a>, words: &ColorPickerWords, got: &mut Collected) {
        let edit = TextField::hex(words.hex.as_str().to_owned(), Bind::given(pass.shows(pass.shown.color.to_rgba())))
            .empty(pass.shown.color.to_rgba())
            .show(ui);
        if pass.focus_hex {
            focus::request(ui.ctx(), FocusTarget::Id(edit.response.id));
        }
        got.adopt(edit, pass.shown, |v, mine| *mine = mine.with_hex(*v, &pass.rules));
    }

    /// The screen-pick button; its press is the output's `screen_pick`.
    pub(super) fn sample_button(&self, ui: &mut Ui, words: &ColorPickerWords, got: &mut Collected) {
        if !self.eyedropper {
            return;
        }
        let label = words.sample_screen.as_str().to_owned();
        let out = ScreenPickButton::new().label(label.clone()).tooltip(label).enabled(!self.eyedropper_busy).show(ui);
        got.screen_pick = out.gesture;
    }
}
