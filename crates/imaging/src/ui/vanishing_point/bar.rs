//! [`VanishingPointBar`]: Vanishing Point's options bar.

use std::ops::RangeInclusive;

use egui::{Response, Ui};

use craft_behavior::EditEvents;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft_ui::display::Divider;
use craft_ui::form::Field;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::text::NumberField;

use super::words::VanishingPointWords;
use super::{PasteImage, StampSettings, VanishingPointSettings, VanishingTool};

/// The paste's Width, in percent of the plane's width (PhotoCraft's range).
pub(super) const PASTE_WIDTH: RangeInclusive<f64> = 1.0..=1000.0;

/// The paste's Angle, in degrees.
pub(super) const PASTE_ANGLE: RangeInclusive<f64> = -180.0..=180.0;

/// What [`VanishingPointBar::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct VanishingPointBarOutput {
    /// The bar's response.
    pub response: Response,
    /// The settings the user changed this pass, in order, each with every setting as it is then: a number field's gesture (`Begin`,
    /// `Update`, `Commit`) or a checkbox's `Set`.
    pub edit: EditEvents<VanishingPointSettings>,
    /// The selected paste as its Width, Angle or Flip fields left it this pass: a session change, one undo step when
    /// its gesture commits.
    pub paste: EditEvents<PasteImage>,
}

impl Outcome for VanishingPointBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The options bar, full width under the title: for Stamp Diameter (1..=2000 px), Hardness, Opacity (0..=100 %),
/// Aligned; for every tool Grid Size (px); with a paste selected (not under Stamp) its Width (percent of the plane),
/// Angle (degrees), Flip Horizontal and Flip Vertical; New Layer. PhotoCraft's tool pills and its check and ban
/// buttons go: the strip holds the tools and the side column's footer OK and Cancel.
///
/// New Layer shows with an image on the clipboard, with strokes ([`has_strokes`](Self::has_strokes)) or with the Stamp
/// tool, as PhotoCraft's bar does.
#[derive(craft_ui::contract::Pattern)]
pub struct VanishingPointBar<'a> {
    tool: VanishingTool,
    settings: &'a VanishingPointSettings,
    paste: Option<PasteImage>,
    has_clip: bool,
    has_strokes: bool,
}

impl<'a> VanishingPointBar<'a> {
    /// The bar for `tool` over `settings`; `paste` is the selected paste, if any, and `has_clip` whether the clipboard
    /// holds an image.
    pub fn new(tool: VanishingTool, settings: &'a VanishingPointSettings, paste: Option<PasteImage>, has_clip: bool) -> Self {
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
        let mut pasted = EditEvents::new();
        let shown = Bar::new(words.options.clone(), BarKind::Options).show(ui, |ui, _| {
            (edit, pasted) = (EditEvents::new(), EditEvents::new());
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
                    ("diameter", &words.diameter, settings.stamp.diameter, StampSettings::DIAMETER, UnitKind::Pixels, |s, v| s.stamp.diameter = v),
                    ("hardness", &words.hardness, settings.stamp.hardness, StampSettings::PERCENT, UnitKind::Percent, |s, v| s.stamp.hardness = v),
                    ("opacity", &words.opacity, settings.stamp.opacity, StampSettings::PERCENT, UnitKind::Percent, |s, v| s.stamp.opacity = v),
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
            edit.extend(
                number(ui, "grid-size", &words.grid_size, settings.grid_size, VanishingPointSettings::GRID_SIZE, UnitKind::Pixels)
                    .map(set(|s, v| s.grid_size = v)),
            );
            if let Some(p) = paste.filter(|_| tool != VanishingTool::Stamp) {
                Divider::vertical().show(ui);
                pasted.extend(
                    number(ui, "paste-width", &words.width, p.width * 100.0, PASTE_WIDTH, UnitKind::Percent)
                        .map(move |v| PasteImage { width: v / 100.0, ..p }),
                );
                pasted.extend(
                    number(ui, "paste-angle", &words.angle, p.angle, PASTE_ANGLE, UnitKind::Degrees).map(move |v| PasteImage { angle: v, ..p }),
                );
                let flip_h = Checkbox::new(Bind::given(p.flip_h)).label(words.flip_h.clone()).show(ui);
                pasted.extend(flip_h.events.map(move |on| PasteImage { flip_h: on, ..p }));
                let flip_v = Checkbox::new(Bind::given(p.flip_v)).label(words.flip_v.clone()).show(ui);
                pasted.extend(flip_v.events.map(move |on| PasteImage { flip_v: on, ..p }));
            }
            if has_clip || has_strokes || tool == VanishingTool::Stamp {
                Divider::vertical().show(ui);
                let layer = Checkbox::new(Bind::given(settings.new_layer)).label(words.new_layer.clone()).show(ui);
                edit.extend(layer.events.map(|on| VanishingPointSettings { new_layer: on, ..*settings }));
            }
        });
        VanishingPointBarOutput { response: shown.response.clone(), edit, paste: pasted }
    }
}

/// One captioned number cell: the edit of `value`, if the user made one this pass.
fn number(ui: &mut Ui, key: &'static str, caption: &Label, value: f64, range: RangeInclusive<f64>, kind: UnitKind) -> EditEvents<f64> {
    let mut edited = EditEvents::new();
    Field::new(caption.clone()).id_salt(key).side(SideLabel::new().scrub(true)).show(ui, |ui, _| {
        let out = NumberField::new(caption.clone(), Bind::given(Some(value))).range(range.clone()).unit_kind(kind).scrub_by_label().show(ui);
        edited = out.edit.events.map(|v| v.unwrap_or(value));
    });
    edited
}
