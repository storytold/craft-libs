//! [`VanishingPointBar`]: Vanishing Point's options bar.

use std::ops::RangeInclusive;

use egui::{Response, Ui};

use craft_behavior::EditEvents;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing};
use craft_ui::display::Divider;
use craft_ui::form::Field;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::text::NumberField;

use super::words::VanishingPointWords;
use super::{PasteImage, VanishingPointSettings, VanishingTool};

/// Diameter, in pixels.
const DIAMETER: RangeInclusive<f64> = 1.0..=5000.0;
/// Hardness and Opacity, in percent.
const PERCENT: RangeInclusive<f64> = 0.0..=100.0;
/// Grid Size, in pixels (PhotoCraft's range).
const GRID_SIZE: RangeInclusive<f64> = 2.0..=2000.0;
/// The paste's Width, in percent of the plane's width (PhotoCraft's range).
const PASTE_WIDTH: RangeInclusive<f64> = 1.0..=1000.0;

/// What [`VanishingPointBar::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct VanishingPointBarOutput {
    /// The bar's response.
    pub response: Response,
    /// The settings the user changed this pass, in order, each with every setting as it is then: a number field's gesture (`Begin`,
    /// `Update`, `Commit`) or a checkbox's `Set`.
    pub edit: EditEvents<VanishingPointSettings>,
    /// The paste's Width as a fraction of the plane's width, edited this pass: a session change, one undo step when
    /// its gesture commits.
    pub paste_width: EditEvents<f64>,
}

impl Outcome for VanishingPointBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The options bar, full width under the title: for Stamp Diameter (1..=5000 px), Hardness, Opacity (0..=100 %),
/// Aligned; for every tool Grid Size (px); with an image on the clipboard Paste and Width (percent of the plane); New
/// Layer. PhotoCraft's tool pills and its check and ban buttons go: the strip holds the tools and the side column's
/// footer OK and Cancel.
///
/// Width shows while Paste is on. New Layer shows with an image on the clipboard, with strokes
/// ([`has_strokes`](Self::has_strokes)) or with the Stamp tool, as PhotoCraft's bar does.
#[derive(craft_ui::contract::Pattern)]
pub struct VanishingPointBar<'a> {
    tool: VanishingTool,
    settings: &'a VanishingPointSettings,
    paste: PasteImage,
    has_clip: bool,
    has_strokes: bool,
}

impl<'a> VanishingPointBar<'a> {
    /// The bar for `tool` over `settings`; `paste` is the pasted image's placement and `has_clip` whether the
    /// clipboard holds an image.
    pub fn new(tool: VanishingTool, settings: &'a VanishingPointSettings, paste: PasteImage, has_clip: bool) -> Self {
        Self { tool, settings, paste, has_clip, has_strokes: false }
    }

    /// Whether the session has clone strokes (default no): they show New Layer.
    #[must_use]
    pub fn has_strokes(mut self, has_strokes: bool) -> Self {
        self.has_strokes = has_strokes;
        self
    }

    /// Shows the bar.
    pub fn show(self, ui: &mut Ui) -> VanishingPointBarOutput {
        // pc: crates/ui-egui/src/dialogs/vp_ui.rs:1230 `options_bar`.
        let env = Env::of(ui);
        let words = env.words::<VanishingPointWords>();
        let Self { tool, settings, paste, has_clip, has_strokes } = self;
        let mut edit = EditEvents::<VanishingPointSettings>::new();
        let mut paste_width = EditEvents::new();
        let shown = Bar::new(words.options.clone(), BarKind::Options).show(ui, |ui, _| {
            (edit, paste_width) = (EditEvents::new(), EditEvents::new());
            let set = |change: fn(&mut VanishingPointSettings, f64)| {
                move |value: f64| {
                    let mut next = *settings;
                    change(&mut next, value);
                    next
                }
            };
            if tool == VanishingTool::Stamp {
                type StampCell<'a> = (&'static str, &'a Label, f64, RangeInclusive<f64>, UnitKind, fn(&mut VanishingPointSettings, f64));
                let cells: [StampCell<'_>; 3] = [
                    ("diameter", &words.diameter, settings.stamp.diameter, DIAMETER, UnitKind::Pixels, |s, v| s.stamp.diameter = v),
                    ("hardness", &words.hardness, settings.stamp.hardness, PERCENT, UnitKind::Percent, |s, v| s.stamp.hardness = v),
                    ("opacity", &words.opacity, settings.stamp.opacity, PERCENT, UnitKind::Percent, |s, v| s.stamp.opacity = v),
                ];
                for (key, caption, value, range, kind, change) in cells {
                    edit.extend(number(ui, key, caption, value, range, kind).map(set(change)));
                }
                let aligned = Checkbox::new(Bind::given(settings.stamp.aligned)).label(words.aligned.clone()).show(ui);
                edit.extend(
                    aligned.events.map(|on| VanishingPointSettings { stamp: super::StampSettings { aligned: on, ..settings.stamp }, ..*settings }),
                );
                Divider::vertical().show(ui);
            }
            edit.extend(number(ui, "grid-size", &words.grid_size, settings.grid_size, GRID_SIZE, UnitKind::Pixels).map(set(|s, v| s.grid_size = v)));
            if has_clip {
                Divider::vertical().show(ui);
                let pasted = Checkbox::new(Bind::given(settings.paste)).label(words.paste.clone()).show(ui);
                edit.extend(pasted.events.map(|on| VanishingPointSettings { paste: on, ..*settings }));
                if settings.paste {
                    let percent = paste.width * 100.0;
                    paste_width = number(ui, "paste-width", &words.width, percent, PASTE_WIDTH, UnitKind::Percent).map(|v| v / 100.0);
                }
            }
            if has_clip || has_strokes || tool == VanishingTool::Stamp {
                Divider::vertical().show(ui);
                let layer = Checkbox::new(Bind::given(settings.new_layer)).label(words.new_layer.clone()).show(ui);
                edit.extend(layer.events.map(|on| VanishingPointSettings { new_layer: on, ..*settings }));
            }
        });
        VanishingPointBarOutput { response: shown.response.clone(), edit, paste_width }
    }
}

/// One captioned number cell: the edit of `value`, if the user made one this pass.
fn number(ui: &mut Ui, key: &'static str, caption: &Label, value: f64, range: RangeInclusive<f64>, kind: UnitKind) -> EditEvents<f64> {
    let mut edited = EditEvents::new();
    Field::new(caption.clone()).id_salt(key).side(SideLabel::new().scrub(true)).sizing(Sizing::Content).show(ui, |ui, _| {
        let out = NumberField::new(caption.clone(), Bind::given(Some(value))).range(range.clone()).unit_kind(kind).scrub_by_label().show(ui);
        edited = out.edit.events.map(|v| v.unwrap_or(value));
    });
    edited
}
