//! Stories of the dialog shells.

use super::modal::ModalDragStrip;
use super::{BlockingOverlay, Dialog, DialogFrame, DialogHost, StackDialog};
use crate::buttons::Button;
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _, clicked};
use crate::runtime::focus::Initial;
use crate::sections::{DialogRole, FooterButton};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;
use std::sync::{Arc, Mutex, PoisonError};

/// What the story dialogs hold: two check boxes whose values the story keeps between passes.
#[doc(hidden)]
pub fn body(ui: &mut Ui, checks: &mut [bool; 2]) {
    let [snap, rulers] = checks;
    Checkbox::new(Bind::bound(snap)).label("Snap to grid").show(ui);
    Checkbox::new(Bind::bound(rulers)).label("Show rulers").show(ui);
}

/// A story's dialog host: the runs of a story share it, and it equals only itself, so story values hold it.
#[derive(Clone, Default)]
struct StoryHost(Arc<Mutex<DialogHost>>);

impl PartialEq for StoryHost {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A story of a hosted dialog: `open()` on the story's [`DialogHost`], which runs every pass. The story's `open` value
/// is the dialog's open state, so the card holds the dialog behind its button as it holds any story dialog: the
/// dialog opens while the value is on, and the value goes off when the dialog closes.
#[doc(hidden)]
pub fn hosted<D: StackDialog>(ui: &mut Ui, data: &mut StoryData, open: impl FnOnce() -> D) -> StoryOutput {
    hosted_with(ui, data, open, |_| {})
}

/// [`hosted`], with `pass` answering what the open dialog asked for (a preview, a path) before the host runs it.
#[doc(hidden)]
pub fn hosted_with<D: StackDialog>(ui: &mut Ui, data: &mut StoryData, open: impl FnOnce() -> D, pass: impl FnOnce(&mut DialogHost)) -> StoryOutput {
    let host = data.value("host", StoryHost::default()).clone();
    let mut host = host.0.lock().unwrap_or_else(PoisonError::into_inner);
    let wanted = *data.value("open", true);
    match (wanted, host.is_open::<D>()) {
        (true, false) => {
            host.open(open());
        }
        (false, true) => {
            host.cancel::<D>(ui.ctx());
        }
        _ => {}
    }
    pass(&mut host);
    host.run(ui);
    *data.value("open", true) = host.is_open::<D>();
    StoryOutput::default()
}

/// The opener of the Grid options dialogs.
const GRID_OPTIONS: &str = "Grid Options…";

/// The open state a story's dialog binds, behind the button that opens it: dialogs start closed, and the button opens
/// the real one.
fn opener<'d>(ui: &mut Ui, data: &'d mut StoryData, label: &'static str) -> &'d mut bool {
    let open = data.value("open", false);
    if clicked(&Button::new(label).show(ui)) {
        *open = true;
    }
    open
}

/// The drive of the shells with a close action: the opener's click where there is one, then a click on the close
/// action.
#[doc(hidden)]
pub fn close_box(fixture: &mut Fixture) {
    if let Some(open) = fixture.by_label(GRID_OPTIONS) {
        fixture.click(&open);
        fixture.settle();
    }
    if let Some(close) = fixture.by_label("Close") {
        fixture.click(&close);
    }
}

#[story(Dialog<'static>, covers = [Default], drive = Drive::Custom(close_box))]
fn dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let shown: StoryOutput =
        Dialog::new("Grid options").opened(Bind::bound(opener(ui, data, GRID_OPTIONS))).show(ui, |ui, _| body(ui, &mut checks)).into();
    *data.value("checks", [true, false]) = checks;
    shown
}

#[story(Dialog<'static>, covers = [], drive = Drive::Never)]
fn dialog_with_footer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let shown: StoryOutput = Dialog::new("Grid options")
        .opened(Bind::bound(opener(ui, data, GRID_OPTIONS)))
        .footer(|ui, _| {
            Checkbox::new(Bind::given(false)).label("Don't ask again").show(ui);
        })
        .show(ui, |ui, _| body(ui, &mut checks))
        .into();
    *data.value("checks", [true, false]) = checks;
    shown
}

#[story(Dialog<'static>, covers = [], drive = Drive::Never)]
fn dialog_with_side_buttons(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let shown: StoryOutput = Dialog::new("Grid options")
        .opened(Bind::bound(opener(ui, data, GRID_OPTIONS)))
        .side_buttons(true)
        .buttons([
            FooterButton::new("OK", DialogRole::Accept),
            FooterButton::new("Cancel", DialogRole::Cancel),
            FooterButton::new("Add to Swatches", DialogRole::Other).stays_open(true),
        ])
        .footer(|ui, _| {
            Checkbox::new(Bind::given(true)).label("Preview").show(ui);
        })
        .show(ui, |ui, _| body(ui, &mut checks))
        .into();
    *data.value("checks", [true, false]) = checks;
    shown
}

#[story(Dialog<'static>, covers = [], drive = Drive::Never)]
fn dialog_resizable_with_initial_focus(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let shown: StoryOutput = Dialog::new("Grid options")
        .opened(Bind::bound(opener(ui, data, GRID_OPTIONS)))
        .initial_focus(Initial::First)
        .resizable(true)
        .show(ui, |ui, _| body(ui, &mut checks))
        .into();
    *data.value("checks", [true, false]) = checks;
    shown
}

#[story(Dialog<'static>, covers = [], drive = Drive::Never)]
fn dialog_prompt(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let shown: StoryOutput = Dialog::prompt("Discard changes?")
        .opened(Bind::bound(opener(ui, data, "Close Document…")))
        .buttons([FooterButton::new("Discard", DialogRole::Destructive).letter('D'), FooterButton::new("Keep editing", DialogRole::Cancel)])
        .show(ui, |ui, _| body(ui, &mut checks))
        .into();
    *data.value("checks", [true, false]) = checks;
    shown
}

#[story(Dialog<'static>, covers = [], drive = Drive::Never)]
fn dialog_modeless(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let resizable = *data.value("resizable", false);
    let shown: StoryOutput = Dialog::new("Grid options")
        .opened(Bind::bound(opener(ui, data, GRID_OPTIONS)))
        .modeless()
        .resizable(resizable)
        .show(ui, |ui, _| body(ui, &mut checks))
        .into();
    *data.value("checks", [true, false]) = checks;
    shown
}

#[story(DialogFrame<'static>, covers = [Default], drive = Drive::Custom(close_box))]
fn dialog_frame(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let closable = *data.value("close box", true);
    let roomy = *data.value("roomy", false);
    let shown: StoryOutput = DialogFrame::new("Grid options").closable(closable).roomy(roomy).show(ui, |ui, _| body(ui, &mut checks)).into();
    *data.value("checks", [true, false]) = checks;
    shown
}

#[story(DialogFrame<'static>, covers = [], drive = Drive::Never)]
fn dialog_frame_with_footer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let shown: StoryOutput = DialogFrame::new("Grid options")
        .roomy(true)
        .footer(|ui, _| {
            Checkbox::new(Bind::given(false)).label("Don't ask again").show(ui);
        })
        .show(ui, |ui, _| body(ui, &mut checks))
        .into();
    *data.value("checks", [true, false]) = checks;
    shown
}

/// The drive of the overlay story: the opener's click.
#[doc(hidden)]
pub fn open_export(fixture: &mut Fixture) {
    if let Some(open) = fixture.by_label(EXPORT) {
        fixture.click(&open);
    }
}

/// The opener of the overlay story.
const EXPORT: &str = "Export…";

#[story(BlockingOverlay<'static>, covers = [Default], drive = Drive::Custom(open_export))]
fn blocking_overlay(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let scrim = *data.value("scrim", true);
    let card = *data.value("wait card", true);
    let overlay = BlockingOverlay::new("Exporting").scrim(scrim);
    let overlay = if card { overlay.message("Exporting 24 pages…").cancel("Cancel") } else { overlay };
    // Escape closes it too, so the story's overlay without a card can close.
    overlay.cancellable(true).opened(Bind::bound(opener(ui, data, EXPORT))).show(ui, |_, _| {}).into()
}

#[story(ModalDragStrip, covers = [Default], drive = Drive::Never)]
fn modal_drag_strip(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = ui.max_rect();
    ModalDragStrip::new(room).show(ui, |_, _| {}).into()
}
