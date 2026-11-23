//! The pen's Path Options: the rubber band, the path colour and thickness ([`PathOptionsSettings`]), and the gear
//! popover that edits them ([`PathOptionsPopover`]).

use std::ops::RangeInclusive;
use std::sync::Arc;

use craft_behavior::{EditEvent, IconId, Mixed};
use craft_theme::Srgba;
use craft_ui::buttons::IconButton;
use craft_ui::choice::{Checkbox, Swatch};
use craft_ui::contract::{Bind, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::popover::Popover;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::text::NumberField;
use egui::{Response, Ui};

craft_ui::words! {
    /// The words of [`PathOptionsPopover`], English by default (Photoshop's Pen gear popover).
    pub struct PathOptionsWords {
        /// The gear button's accessible name and tooltip, and the popover's title.
        title: "pathOptions.title" = "Path Options";
        /// The thickness number field.
        thickness: "pathOptions.thickness" = "Thickness";
        /// The colour swatch button, and the colour picker's title.
        color: "pathOptions.color" = "Color";
        /// The button that clears the colour.
        default_color: "pathOptions.defaultColor" = "Default";
        /// The rubber band checkbox.
        rubber_band: "pathOptions.rubberBand" = "Rubber Band";
    }
}

/// The key of the colour request the popover files: the host opens its colour picker for it and answers with the
/// colour.
pub const PATH_OPTIONS_COLOR_KEY: &str = "pathOptions.color";

/// The pen's Path Options, persisted by the app with its preferences (Photoshop's Pen gear popover). Serialised in
/// camelCase; unknown fields are ignored and missing ones take their defaults.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct PathOptionsSettings {
    /// The Pen's rubber band from the last anchor to the pointer. Off by default, as Photoshop. The Curvature mode
    /// always draws its preview curve.
    pub rubber_band: bool,
    /// The path colour as sRGB bytes; `None` uses the overlay line colour. Paths with a colour of their own (a layer
    /// colour) keep it.
    pub color: Option<[u8; 3]>,
    /// The path line's width in pixels, within [`THICKNESS_RANGE`](Self::THICKNESS_RANGE); `None` uses the theme's
    /// medium line.
    pub thickness: Option<f32>,
}

impl PathOptionsSettings {
    /// The thicknesses the popover offers, in pixels.
    pub const THICKNESS_RANGE: RangeInclusive<f32> = 0.5..=8.0;
    /// What one arrow press or scrub step of the thickness field changes.
    const THICKNESS_STEP: f64 = 0.5;
}

/// What the popover reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PathOptionsOutput {
    /// The gear button.
    pub response: Response,
    /// The settings changed in this pass, as a `Set` of the whole value. They are preferences, so the host stores the
    /// latest value and makes no undo step; a binding that writes through already holds it.
    pub edit: Option<EditEvent<PathOptionsSettings>>,
    /// The colour swatch was activated: the host opens its colour picker keyed [`PATH_OPTIONS_COLOR_KEY`] and, on OK,
    /// stores the colour in [`PathOptionsSettings::color`] itself.
    pub pick_color: Option<ColorRequest>,
}

impl Outcome for PathOptionsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Pen's gear button in the options bar and its Path Options popover (Photoshop's): a Thickness number field
/// (pixels, [`THICKNESS_RANGE`](PathOptionsSettings::THICKNESS_RANGE), empty while the theme's line is used), a Color
/// swatch with a Default button that clears it, and a Rubber Band checkbox.
///
/// - The gear is an icon button named by [`PathOptionsWords::title`]; a click or Enter toggles the popover below it.
///   Focus moves into the popover; Tab goes Thickness, Color, Default, Rubber Band; Escape closes it and returns
///   focus to the gear; a press outside closes it.
/// - The popover reports every change at once; there is no OK button.
#[must_use = "show the popover with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct PathOptionsPopover<'a> {
    label: Label,
    settings: Bind<'a, PathOptionsSettings>,
    default_color: Option<Srgba>,
}

impl<'a> PathOptionsPopover<'a> {
    /// The gear and popover for the host's `settings`, named `label` for assistive technology.
    pub fn new(label: impl Into<Label>, settings: Bind<'a, PathOptionsSettings>) -> Self {
        Self { label: label.into(), settings, default_color: None }
    }

    /// The colour the swatch shows while the settings hold none (the overlay line colour the pen draws in). Without
    /// it the swatch shows a neutral well.
    pub fn default_color(mut self, color: Srgba) -> Self {
        self.default_color = Some(color);
        self
    }

    /// Adds the gear button to `ui` and, while `open`, the popover under it. The host owns `open`.
    pub fn show(mut self, ui: &mut Ui, open: &mut bool) -> PathOptionsOutput {
        let words = Env::of(ui).words::<PathOptionsWords>();
        let gear = IconButton::new(IconId::new("settings")).label(words.title.clone()).show(ui).response;
        let (mut edit, mut pick_color) = (None, None);
        let (settings, default_color) = (&mut self.settings, self.default_color);
        let _shown = Popover::new(self.label.clone(), Bind::bound(open)).opened_by(&gear).show(ui, |ui, cx| {
            let before = match settings.reborrow().read(cx) {
                Some(Mixed::Same(held)) => held,
                _ => PathOptionsSettings::default(),
            };
            let mut now = before;
            FieldGroup::new(words.title.clone()).show(ui, |ui, _| {
                Field::new(words.thickness.clone()).show(ui, |ui, _| {
                    let range = f64::from(*PathOptionsSettings::THICKNESS_RANGE.start())..=f64::from(*PathOptionsSettings::THICKNESS_RANGE.end());
                    let field = NumberField::new(words.thickness.clone(), Bind::given(before.thickness.map(f64::from)))
                        .range(range)
                        .step(PathOptionsSettings::THICKNESS_STEP)
                        .automatic(true)
                        .unit_kind(UnitKind::Pixels)
                        .show(ui);
                    if let Some(thickness) = field.value().copied() {
                        now.thickness = thickness.map(|value| value as f32);
                    }
                });
                Field::new(words.color.clone()).show(ui, |ui, _| {
                    let shown = before.color.map(|[r, g, b]| Srgba::from(craft_theme::Rgba::rgb(r, g, b))).or(default_color);
                    let swatch = Swatch::new(words.color.clone(), shown.map_or(Mixed::Mixed, Mixed::Same)).picker(PATH_OPTIONS_COLOR_KEY).show(ui);
                    pick_color = swatch.pick_color;
                    let clear = IconButton::new(IconId::new("undo")).label(words.default_color.clone()).enabled(before.color.is_some()).show(ui);
                    if craft_ui::contract::clicked(&clear) {
                        now.color = None;
                    }
                });
                let rubber = Checkbox::new(Bind::given(before.rubber_band)).label(words.rubber_band.clone()).show(ui);
                if let Some(rubber_band) = rubber.value().copied() {
                    now.rubber_band = rubber_band;
                }
            });
            if now != before {
                let event = EditEvent::set(Mixed::Same(before), now, Arc::from(words.title.as_str()), None);
                let _ = settings.reborrow().write(cx, &event);
                edit = Some(event);
            }
        });
        PathOptionsOutput { response: gear, edit, pick_color }
    }
}
