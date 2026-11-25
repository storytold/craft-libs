//! The FillStroke layout of the Color panel: the proxy wells with their warnings, the rows the host's model rule
//! shows, the None, Black and White wells with the hex field, the Spectrum ramp and the Recent Colors. It reads the
//! same `Body` as the Photoshop layout and shares its rows, wells, warnings and ramp helpers.

use craft_behavior::{EditEvent, Mixed};
use craft_theme::Srgba;
use craft_ui::choice::{Swatch, SwatchSpacing};
use craft_ui::contract::{Bind, Gap, LabelScheme, Props as _, Scroll, Show as _, ShowContainer as _, Sizing};
use craft_ui::display::Text;
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::runtime::env::Label;
use craft_ui::text::{HexField, TextField};
use egui::Ui;

use super::{Body, ColorPanelMode, ColorPanelShown, PanelTarget, TintRow};
use crate::ui::{ChannelRow, ChannelRule, ChannelUnit, ColorRamp, PickedColor, RampKind, SwatchPalette};

impl Body<'_> {
    /// The layout. The options (rows, quick wells, recent colours) go when the state hides them; the proxy and the
    /// spectrum stay.
    pub(super) fn fill_stroke(mut self, ui: &mut Ui) -> Self {
        let hidden = self.now.hide_options;
        Stack::new().fill().scroll(Scroll::vertical()).show(ui, |ui, _| {
            self.top(ui, hidden);
            if !hidden {
                self.quick_wells(ui);
            }
            self.spectrum(ui);
            if !hidden && self.now.recent_colors {
                self.recent(ui);
            }
        });
        self
    }

    /// The mode whose rows show: the host's, else the state's.
    fn shown_mode(&self) -> ColorPanelMode {
        match self.inputs.shown {
            ColorPanelShown::Mode(mode) => *mode,
            _ => self.now.mode,
        }
    }

    /// The proxy with its warnings beside it, then the rows. The warnings sit in the proxy's own height, so the
    /// layout under them holds still when one appears.
    fn top(&mut self, ui: &mut Ui, hidden: bool) {
        let enabled = self.enabled();
        let shown = self.inputs.shown;
        let web = matches!(shown, ColorPanelShown::Mode(ColorPanelMode::WebColorSliders));
        Row::new().fill_width().gap(Gap::Item).show(ui, |ui, _| {
            Row::new().gap(Gap::Tight).show(ui, |ui, _| {
                self.wells(ui);
                if enabled && !matches!(shown, ColorPanelShown::Empty) {
                    self.warnings(ui, web);
                }
            });
            if !hidden {
                Stack::new().fill_width().enabled(enabled).show(ui, |ui, _| match shown {
                    ColorPanelShown::Mode(mode) if mode.is_sliders() => self.mode_rows(ui, *mode),
                    ColorPanelShown::Tint(tint) => self.tint_row(ui, tint),
                    _ => {}
                });
            }
        });
    }

    /// A swatch-linked colour: its chip (tinted, named with its kind), its name and the T row.
    fn tint_row(&mut self, ui: &mut Ui, tint: &TintRow) {
        let strength = tint.tint / 100.0;
        let mix = |channel: f32| 1.0 + (channel - 1.0) * strength;
        let shown = Srgba::new(mix(tint.base.r), mix(tint.base.g), mix(tint.base.b), tint.base.a);
        let kinds: Vec<&str> = [(tint.global, self.words.global.as_str()), (tint.spot, self.words.spot.as_str())]
            .into_iter()
            .filter(|(on, _)| *on)
            .map(|(_, word)| word)
            .collect();
        let mut events = Vec::new();
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            Row::new().gap(Gap::Item).show(ui, |ui, _| {
                let _ = Swatch::new(tint.name.clone(), shown).well().description(kinds.join(", ")).show(ui);
                Text::new(tint.name.clone()).show(ui);
            });
            FieldGroup::new(self.words.tint.clone()).labels(LabelScheme::Inspector).gap(Gap::Tight).show(ui, |ui, _| {
                let row = ChannelRow::new(self.words.tint.clone(), Bind::given(f64::from(tint.tint)), ChannelRule::ALPHA.clone())
                    .short("T")
                    .unit(ChannelUnit::Percent)
                    .track(vec![Srgba::rgb(1.0, 1.0, 1.0), tint.base])
                    .show(ui);
                events.extend(row.into_iter().flat_map(|row| row.events));
            });
        });
        // A pass has one tint edit: the last of its events.
        self.tint = events.into_iter().last().map(|event| event.map(|value| value as f32));
    }

    /// The None, Black and White wells and the hex field. Alt on a well sets the colour not edited.
    fn quick_wells(&mut self, ui: &mut Ui) {
        let (words, enabled) = (self.words, self.enabled());
        let wells =
            [(&words.none, Srgba::new(0.0, 0.0, 0.0, 0.0)), (&words.black, Srgba::rgb(0.0, 0.0, 0.0)), (&words.white, Srgba::rgb(1.0, 1.0, 1.0))];
        let mut hit = None;
        Row::new().fill_width().gap(Gap::Item).show(ui, |ui, _| {
            for (at, (label, color)) in wells.iter().enumerate() {
                let out = Swatch::new((*label).clone(), *color).well().show(ui);
                if out.clicked {
                    hit = Some((at, out.activation.is_some_and(|activation| activation.mods.alt)));
                }
            }
            Stack::new().fill_width().enabled(enabled).show(ui, |ui, _| self.hex(ui));
        });
        if let Some((at, other)) = hit {
            let target = if other { self.target.other() } else { self.target };
            match wells.get(at) {
                Some((_, color)) if at > 0 => self.set_color(target, *color),
                _ => self.none = Some(target),
            }
        }
    }

    /// The hex field: an entry sets the colour with the edited colour's alpha.
    fn hex(&mut self, ui: &mut Ui) {
        let color = self.color();
        let words = self.words;
        let edit = Field::new(words.hex_color.clone())
            .side(HexField::hash_label())
            .show(ui, |ui, _| TextField::hex(words.hex_color.clone(), Bind::given(color.to_rgba())).show(ui))
            .inner;
        for event in edit.into_iter().flat_map(|edit| edit.events) {
            self.push(self.target, event.map(|rgba| PickedColor::rgb(Srgba::from(rgba).with_alpha(color.a))));
        }
    }

    /// The Spectrum ramp (grey in Grayscale) with a tooltip naming the shown mode. A Cmd or Ctrl click recolours and
    /// picks nothing; a Shift click moves to the next mode.
    fn spectrum(&mut self, ui: &mut Ui) {
        let mode = self.shown_mode();
        let kind = if mode == ColorPanelMode::GrayscaleSlider { RampKind::Grayscale } else { RampKind::Spectrum };
        let name = mode.label(self.words);
        let (gamut, revision) = self.ramp_gamut();
        let ramp = ColorRamp::new(self.words.color_ramp.clone(), kind, Bind::kept())
            .kind_name(name.clone())
            .tooltip(name)
            .recolor_clicks(true)
            .gamut(gamut)
            .gamut_revision(revision)
            .web_safe(self.now.web_safe_ramp)
            .sizing(Sizing::Fill)
            .show(ui);
        self.ramp_events(ramp.edit.events);
        self.recolor = ramp.recolor;
        if ramp.cycle {
            self.now.mode = mode.fill_stroke_next();
        }
    }

    /// The Recent Colors as a spaced palette; a pick sets the colour.
    // PLAN: Alt on a recent colour or a warning chip should set the other colour; `SwatchPaletteOutput` and
    // `ColorWarningChipOutput` report no modifiers (color-w1 for the palette, the owner of warning.rs for the chip).
    fn recent(&mut self, ui: &mut Ui) {
        if self.inputs.recent.is_empty() {
            return;
        }
        let swatches: Vec<(Label, Srgba)> = self.inputs.recent.iter().map(|color| (Label::from(color.to_hex()), *color)).collect();
        let mut current = self.color();
        let mut palette = SwatchPalette::new(self.words.recent_colors.clone(), Bind::bound(&mut current), &swatches).spacing(SwatchSpacing::Spaced);
        if let Some(setup) = self.inputs.recent_setup {
            palette = setup(palette);
        }
        if let Some(event) = palette.show(ui).edit
            && let Some(color) = event.value()
        {
            self.set_color(self.target, *color);
        }
    }

    /// Sets `target`'s colour to `color`, keeping its alpha.
    fn set_color(&mut self, target: PanelTarget, color: Srgba) {
        let before = self.colors[target.index()];
        let name = self.words.title.as_str();
        self.push(target, EditEvent::set(Mixed::Same(PickedColor::rgb(before)), PickedColor::rgb(color.with_alpha(before.a)), name.into(), None));
    }
}
