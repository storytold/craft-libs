//! [`PinWarpBar`]: Puppet Warp's options bar.

use craft_behavior::{EditEvent, IconId};
use craft_theme::variant::Tone;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::buttons::{Button, IconButton};
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::display::{Divider, Text};
use craft_ui::form::Field;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, UnitKind};
use craft_ui::text::NumberField;
use egui::Ui;

use super::{Pin, PinDensity, PinEdit, PinMode, PinWarpOptions, PinWarpSession};

craft_ui::words! {
    /// The words of [`PinWarpBar`] (PhotoCraft's Puppet Warp bar).
    pub struct PinWarpBarWords {
        name: "pinWarpBar.name" = "Puppet Warp options";
        mode: "pinWarpBar.mode" = "Mode";
        rigid: "pinWarpBar.rigid" = "Rigid";
        normal: "pinWarpBar.normal" = "Normal";
        distort: "pinWarpBar.distort" = "Distort";
        density: "pinWarpBar.density" = "Density";
        fewer_points: "pinWarpBar.fewerPoints" = "Fewer Points";
        more_points: "pinWarpBar.morePoints" = "More Points";
        expansion: "pinWarpBar.expansion" = "Expansion";
        show_mesh: "pinWarpBar.showMesh" = "Show Mesh";
        pin_depth: "pinWarpBar.pinDepth" = "Pin Depth";
        forward: "pinWarpBar.forward" = "Bring the selected pin forward";
        backward: "pinWarpBar.backward" = "Send the selected pin backward";
        rotate: "pinWarpBar.rotate" = "Rotate";
        auto: "pinWarpBar.auto" = "Auto";
        fixed: "pinWarpBar.fixed" = "Fixed";
        rotation: "pinWarpBar.rotation" = "Rotation";
        remove_all: "pinWarpBar.removeAll" = "Remove All Pins";
        commit: "pinWarpBar.commit" = "Commit Puppet Warp";
        cancel: "pinWarpBar.cancel" = "Cancel Puppet Warp";
    }
}

/// The least and greatest Expansion, in pixels.
const EXPANSION: std::ops::RangeInclusive<f64> = -50.0..=100.0;
/// The least and greatest fixed rotation, in degrees.
const ROTATION: std::ops::RangeInclusive<f64> = -360.0..=360.0;

/// Puppet Warp's options bar, in Tab order: Mode select (Rigid, Normal, Distort), Density select (Fewer Points,
/// Normal, More Points), Expansion field (−50 to 100 px), the Show Mesh check; the refused-remesh notice; with a
/// selected pin, Pin Depth + and − buttons, the Rotate select (Auto, Fixed) and, when Fixed, its degrees field
/// (−360 to 360); Remove All Pins; then cancel and commit at the reading end. With several pins selected the depth
/// and rotation act on the first. The bar never changes the session: it reports edits for
/// [`PinWarpSession::apply`].
#[derive(craft_ui::contract::Pattern)]
pub struct PinWarpBar<'a> {
    session: &'a PinWarpSession,
    notice: Option<&'a str>,
}

/// What the bar reported this pass.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PinWarpBarOutput {
    /// The edits in order; the app applies each with [`PinWarpSession::apply`].
    pub edits: Vec<PinEdit>,
    /// Commit pressed.
    pub apply: bool,
    /// Cancel pressed.
    pub discard: bool,
}

impl<'a> PinWarpBar<'a> {
    /// The bar of `session`.
    pub fn new(session: &'a PinWarpSession) -> Self {
        Self { session, notice: None }
    }

    /// A refused remesh's text from [`PinWarpSession::apply`], shown after the options until the app clears it.
    #[must_use]
    pub fn notice(mut self, text: Option<&'a str>) -> Self {
        self.notice = text;
        self
    }

    /// Shows the bar.
    pub fn show(self, ui: &mut Ui) -> PinWarpBarOutput {
        // pc: crates/ui-egui/src/tools/puppet_ui.rs:395 (`options_bar`).
        let words = Env::of(ui).words::<PinWarpBarWords>();
        let mut edits = Vec::new();
        let shown = Bar::new(words.name.clone(), BarKind::Options)
            .commit_cancel(words.commit.clone(), words.cancel.clone())
            .show(ui, |ui, _| body(ui, self.session, &words, self.notice, &mut edits));
        PinWarpBarOutput { edits, apply: shown.committed, discard: shown.cancelled }
    }
}

/// A control with its caption beside it. `add` may run twice (the measuring run, then the real one).
fn captioned<R>(ui: &mut Ui, caption: &craft_ui::runtime::env::Label, mut add: impl FnMut(&mut Ui) -> R) -> Option<R> {
    Field::new(caption.clone()).show(ui, |ui, _| add(ui)).inner
}

/// The options: Mode, Density, Expansion, Show Mesh, the notice and the selected pin's controls, then Remove All
/// Pins.
fn body(ui: &mut Ui, session: &PinWarpSession, words: &PinWarpBarWords, notice: Option<&str>, edits: &mut Vec<PinEdit>) {
    let options = *session.options();
    let pins = session.pins();
    let picked = session.selected().first().copied().filter(|&i| i < pins.len());
    let modes = [
        Entry::new(PinMode::Rigid, words.rigid.clone()),
        Entry::new(PinMode::Normal, words.normal.clone()),
        Entry::new(PinMode::Distort, words.distort.clone()),
    ];
    captioned(ui, &words.mode, |ui| {
        let picked = Select::new(words.mode.clone(), Bind::given(options.mode), &modes).show(ui);
        if let Some(&mode) = picked.value().filter(|&&mode| mode != options.mode) {
            edits.push(PinEdit::Options(PinWarpOptions { mode, ..options }));
        }
    });
    let densities = [
        Entry::new(PinDensity::Fewer, words.fewer_points.clone()),
        Entry::new(PinDensity::Normal, words.normal.clone()),
        Entry::new(PinDensity::More, words.more_points.clone()),
    ];
    captioned(ui, &words.density, |ui| {
        let picked = Select::new(words.density.clone(), Bind::given(options.density), &densities).show(ui);
        if let Some(&density) = picked.value().filter(|&&density| density != options.density) {
            edits.push(PinEdit::Options(PinWarpOptions { density, ..options }));
        }
    });
    captioned(ui, &words.expansion, |ui| {
        let field = NumberField::new(words.expansion.clone(), Bind::given(Some(options.expansion)))
            .range(EXPANSION)
            .unit_kind(UnitKind::Pixels)
            .commit_decimals(0)
            .show(ui);
        // A remesh per update would rebuild the mesh while the value is still being typed or scrubbed: only the
        // value that ends the edit goes out.
        for event in &field.events {
            if let EditEvent::Commit { after: Some(expansion), .. } | EditEvent::Set { after: Some(expansion), .. } = event
                && *expansion != options.expansion
            {
                edits.push(PinEdit::Options(PinWarpOptions { expansion: *expansion, ..options }));
            }
        }
    });
    let shown = Checkbox::new(Bind::given(session.show_mesh())).label(words.show_mesh.clone()).show(ui);
    if let Some(&show) = shown.value().filter(|&&show| show != session.show_mesh()) {
        edits.push(PinEdit::ShowMesh(show));
    }
    if let Some(text) = notice {
        Text::new(text.to_owned()).tone(Tone::Bad).show(ui);
    }
    Divider::vertical().show(ui);
    if let Some(i) = picked {
        let pin = pins[i];
        captioned(ui, &words.pin_depth, |ui| {
            for (icon, name, by, salt) in [("plus", &words.forward, 1, "forward"), ("minus", &words.backward, -1, "backward")] {
                if clicked(&IconButton::new(IconId::new(icon)).id_salt(salt).label(name.clone()).show(ui)) {
                    edits.push(PinEdit::replace(pins, with_pin(pins, i, Pin { depth: pin.depth + by, ..pin }), words.pin_depth.as_str()));
                }
            }
        });
        let rotations = [Entry::new(false, words.auto.clone()), Entry::new(true, words.fixed.clone())];
        captioned(ui, &words.rotate, |ui| {
            let picked = Select::new(words.rotate.clone(), Bind::given(pin.angle.is_some()), &rotations).show(ui);
            if let Some(&fixed) = picked.value().filter(|&&fixed| fixed != pin.angle.is_some()) {
                let angle = fixed.then_some(0.0);
                edits.push(PinEdit::replace(pins, with_pin(pins, i, Pin { angle, ..pin }), words.rotate.as_str()));
            }
        });
        if let Some(angle) = pin.angle {
            captioned(ui, &words.rotation, |ui| {
                let field = NumberField::new(words.rotation.clone(), Bind::given(Some(angle))).range(ROTATION).unit_kind(UnitKind::Degrees).show(ui);
                let turned = |angle: Option<f64>| with_pin(pins, i, Pin { angle: angle.or(pin.angle), ..pin });
                edits.extend(field.events.iter().cloned().map(|event| PinEdit::Pins(event.map(turned))));
            });
        }
        Divider::vertical().show(ui);
    }
    if clicked(&Button::new(words.remove_all.clone()).show(ui)) {
        edits.push(PinEdit::replace(pins, Vec::new(), words.remove_all.as_str()));
    }
}

/// `pins` with pin `i` replaced.
fn with_pin(pins: &[Pin], i: usize, pin: Pin) -> Vec<Pin> {
    let mut all = pins.to_vec();
    all[i] = pin;
    all
}
