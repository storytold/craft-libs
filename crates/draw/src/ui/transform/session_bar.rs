//! `TransformBar`: the options bar of an open Free Transform session.

#[cfg(feature = "screen-testing")]
mod stories;

use super::session::{TransformEdit, TransformPhase, TransformSession, TransformSettings, TransformWarp};
use super::transform_fields::{BarGroups, TransformFields, TransformRow, size_unit_of};
use craft_behavior::{EditEvent, IconId, Mixed, RulerUnits};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::buttons::IconButton;
use craft_ui::canvas::RulerText;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, Outcome, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::display::Divider;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use egui::{Response, Ui};

craft_ui::words! {
    /// The words of [`TransformBar`]. The commit and cancel names carry the phase: "Commit transform" in the Box phase,
    /// "Commit warp" in the Warp phase; their tooltips add the key.
    pub struct TransformBarWords {
        interpolation: "transformBar.interpolation" = "Interpolation";
        bicubic: "transformBar.bicubic" = "Bicubic";
        bilinear: "transformBar.bilinear" = "Bilinear";
        nearest_neighbor: "transformBar.nearestNeighbor" = "Nearest Neighbor";
        show_reference_point: "transformBar.showReferencePoint" = "Show Reference Point";
        commit_transform: "transformBar.commitTransform" = "Commit transform";
        cancel_transform: "transformBar.cancelTransform" = "Cancel transform";
        commit_warp: "transformBar.commitWarp" = "Commit warp";
        cancel_warp: "transformBar.cancelWarp" = "Cancel warp";
        reset_warp: "transformBar.resetWarp" = "Reset warp";
        switch_warp: "transformBar.switchWarp" = "Switch between free transform and warp modes";
    }
}

/// A group of the bar the app draws itself: imaging's warp fields, Content-Aware Scale's Amount and Protect fields.
pub struct BarSlot<'a>(Box<dyn FnMut(&mut Ui) + 'a>);

impl<'a> BarSlot<'a> {
    /// The slot that runs `show` in the bar's row.
    pub fn new(show: impl FnMut(&mut Ui) + 'a) -> Self {
        BarSlot(Box::new(show))
    }

    fn show(&mut self, ui: &mut Ui) {
        (self.0)(ui);
    }
}

/// What the bar reported this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct TransformBarOutput<W> {
    /// The bar's response.
    pub response: Response,
    /// The edits in the order they happened; the app applies each with [`TransformSession::apply`].
    pub edits: Vec<TransformEdit<W>>,
    /// The commit button was pressed or Enter ran it: the app calls [`TransformSession::commit`] after the edits.
    pub apply: bool,
    /// The cancel button was pressed or Escape ran it: the app calls [`TransformSession::discard`] after the edits.
    pub discard: bool,
    /// Reset warp was pressed (Warp phase): the app answers with its identity warp for the box,
    /// `TransformEdit::Warp(identity)`, one undo step.
    pub reset_warp: bool,
    /// The unit picked in the W and H menu this pass, also written to the settings.
    pub unit_picked: Option<UnitKind>,
}

impl<W> Outcome for TransformBarOutput<W> {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The options bar of an open [`TransformSession`]: its own options `Bar` with the commit and cancel buttons of the
/// phase.
///
/// - Box phase, in Tab order: the [`TransformFields`] strip (reference point, X, Y, W, link, H, angle, H and V skew,
///   interpolation), the Show Reference Point check, the app's [`extra`](Self::extra) group, the Free Transform / Warp
///   switch (an icon button, pressed in the Warp phase), then cancel and commit at the reading end.
/// - Warp phase: the app's [`warp_fields`](Self::warp_fields) group in place of the fields, Reset warp and the
///   switch, then cancel and commit.
/// - [`groups`](Self::groups) narrows the strip; [`mode_switch`](Self::mode_switch) hides the switch (Content-Aware
///   Scale).
/// - Every control is an ordinary focusable control. The bar never changes the session: it reports
///   [`TransformEdit`]s, and writes the persisted choices (Show Reference Point, the aspect link, the interpolation,
///   the W and H unit) to its settings binding as well.
#[must_use = "show the bar with `show`"]
#[derive(craft_ui::contract::Pattern)]
#[pattern(probe(TransformBar<'static, ()>))]
pub struct TransformBar<'a, W> {
    label: Label,
    session: &'a TransformSession<W>,
    settings: Bind<'a, TransformSettings>,
    groups: BarGroups,
    mode_switch: bool,
    warp_fields: Option<BarSlot<'a>>,
    extra: Option<BarSlot<'a>>,
    units: (RulerText, RulerText),
    size_units: Option<&'a [Entry<UnitKind>]>,
}

impl<'a, W: TransformWarp> TransformBar<'a, W> {
    /// The bar named `label` of `session`, reading and writing the persisted `settings`.
    pub fn new(label: impl Into<Label>, session: &'a TransformSession<W>, settings: Bind<'a, TransformSettings>) -> Self {
        let pixels = RulerText::new(RulerUnits::pixels(), UnitKind::Pixels);
        Self {
            label: label.into(),
            session,
            settings,
            groups: BarGroups::ALL,
            mode_switch: true,
            warp_fields: None,
            extra: None,
            units: (pixels, pixels),
            size_units: None,
        }
    }

    /// Which groups of the fields show; all by default.
    pub fn groups(mut self, groups: BarGroups) -> Self {
        self.groups = groups;
        self
    }

    /// Whether the Free Transform / Warp switch shows; true by default.
    pub fn mode_switch(mut self, shown: bool) -> Self {
        self.mode_switch = shown;
        self
    }

    /// The Warp phase's group, shown in place of the fields.
    pub fn warp_fields(mut self, slot: BarSlot<'a>) -> Self {
        self.warp_fields = Some(slot);
        self
    }

    /// The app's own group after the Show Reference Point check.
    pub fn extra(mut self, slot: BarSlot<'a>) -> Self {
        self.extra = Some(slot);
        self
    }

    /// The units of X and Y, which also give the document's resolution for lengths in W and H.
    pub fn units(mut self, x: RulerText, y: RulerText) -> Self {
        self.units = (x, y);
        self
    }

    /// The units of the W and H menu; without them W and H show their unit but have no menu.
    pub fn size_units(mut self, units: &'a [Entry<UnitKind>]) -> Self {
        self.size_units = Some(units);
        self
    }

    /// Shows the bar.
    pub fn show(self, ui: &mut Ui) -> TransformBarOutput<W> {
        let TransformBar { label, session, mut settings, groups, mode_switch, mut warp_fields, mut extra, units, size_units } = self;
        let env = Env::of(ui);
        let words = env.words::<TransformBarWords>();
        let points = env.units().points_per_inch();
        let warping = session.phase() == TransformPhase::Warp;
        let (commit, cancel) = if warping { (&words.commit_warp, &words.cancel_warp) } else { (&words.commit_transform, &words.cancel_transform) };
        let name: std::sync::Arc<str> = label.as_str().into();
        let mut edits: Vec<TransformEdit<W>> = Vec::new();
        let mut reset_warp = false;
        let mut unit_picked = None;
        let shown = Bar::new(label.clone(), BarKind::Options).commit_cancel(commit.clone(), cancel.clone()).show(ui, |ui, cx| {
            let current = match settings.reborrow().read(cx) {
                Some(Mixed::Same(held)) => held,
                _ => TransformSettings::default(),
            };
            let mut next = current.clone();
            if warping {
                if let Some(slot) = warp_fields.as_mut() {
                    slot.show(ui);
                }
            } else {
                let unit = current.size_unit.unwrap_or(UnitKind::Percent);
                let row = TransformRow::new(*session.state(), session.skew_v());
                let entries: Option<Vec<Entry<UnitKind>>> =
                    size_units.map(|rows| rows.iter().map(|entry| entry.clone().checked(entry.value() == Some(&unit))).collect());
                let mut fields = TransformFields::new(label.clone(), Bind::given(row), session.rect(), Bind::given(current.aspect_linked))
                    .bar()
                    .groups(groups)
                    .units(units.0, units.1)
                    .size_unit(size_unit_of(unit, units.0, points))
                    .interpolation(Bind::given(session.interpolation()));
                if let Some(entries) = &entries {
                    fields = fields.size_units(entries);
                }
                Self::fields(ui, fields, &mut edits, &mut next, &mut unit_picked);
                if groups.reference {
                    let check = Checkbox::new(Bind::given(session.reference_point())).label(words.show_reference_point.clone()).show(ui);
                    if let Some(on) = check.value().copied() {
                        edits.push(TransformEdit::ReferencePoint(on));
                        next.show_reference_point = on;
                    }
                }
                if let Some(slot) = extra.as_mut() {
                    slot.show(ui);
                }
            }
            if warping {
                let reset = IconButton::new(IconId::new("undo")).label(words.reset_warp.clone()).show(ui);
                reset_warp = clicked(&reset);
            }
            if mode_switch {
                Divider::vertical().show(ui);
                let switch = IconButton::new(IconId::new("grid")).label(words.switch_warp.clone()).selected(warping).show(ui);
                if clicked(&switch) {
                    edits.push(if warping { TransformEdit::LeaveWarp } else { TransformEdit::WarpRequested });
                }
            }
            if next != current {
                let _ = settings.reborrow().write(cx, &EditEvent::set(Mixed::Same(current), next, name.clone(), None));
            }
        });
        TransformBarOutput {
            response: shown.output.response.clone(),
            edits,
            apply: shown.output.committed,
            discard: shown.output.cancelled,
            reset_warp,
            unit_picked,
        }
    }

    /// Shows `fields` and collects what they report: the row's edit, the aspect link, the interpolation and the unit.
    fn fields(
        ui: &mut Ui,
        fields: TransformFields<'_>,
        edits: &mut Vec<TransformEdit<W>>,
        settings: &mut TransformSettings,
        unit_picked: &mut Option<UnitKind>,
    ) {
        let shown = fields.show(ui);
        edits.extend(shown.events.into_iter().map(TransformEdit::Fields));
        if let Some(on) = shown.linked {
            settings.aspect_linked = on;
        }
        if let Some(choice) = shown.interpolation {
            edits.push(TransformEdit::Interpolation(choice));
            settings.interpolation = choice;
        }
        if let Some(unit) = shown.unit_picked {
            settings.size_unit = Some(unit);
            *unit_picked = Some(unit);
        }
    }
}
