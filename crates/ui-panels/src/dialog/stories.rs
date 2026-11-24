//! Stories of the dialog shells.

use super::modal::ModalDragStrip;
use super::{BlockingOverlay, ButtonPlace, Dialog, DialogFrame, DialogHost, JobProgress, StackDialog};
use crate::buttons::Button;
use crate::choice::Checkbox;
use crate::contract::{Bind, FooterSlot, Props as _, Show as _, ShowContainer as _, clicked};
use crate::display::Text;
use crate::layout::GridFit;
use crate::layout_components::{Grid, Row};
use crate::runtime::env::Label;
use crate::runtime::focus::Initial;
use crate::sections::{DialogRole, FooterButton};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::ColumnWidth;
use egui::Ui;

/// What the story dialogs hold: two check boxes whose values the story keeps between passes.
#[doc(hidden)]
pub fn body(ui: &mut Ui, checks: &mut [bool; 2]) {
    let [snap, rulers] = checks;
    Checkbox::new(Bind::bound(snap)).label("Snap to grid").show(ui);
    Checkbox::new(Bind::bound(rulers)).label("Show rulers").show(ui);
}

/// A story of a hosted dialog: `open()` on the story's [`DialogHost`] (a story note), which runs every pass. The story's `open` value
/// is the dialog's open state, so the card holds the dialog behind its button as it holds any story dialog: the
/// dialog opens while the value is on, and the value goes off when the dialog closes. `knobs` holds the story values
/// `open` reads; when they change, `open()` replaces the open dialog, so a knob reaches the dialog on screen. The
/// story reports the open dialog's [`story_facts`](StackDialog::story_facts).
#[doc(hidden)]
pub fn hosted<D: StackDialog, K: PartialEq + 'static>(ui: &mut Ui, data: &mut StoryData, knobs: K, open: impl FnOnce() -> D) -> StoryOutput {
    hosted_with(ui, data, knobs, open, |_| {})
}

/// [`hosted`], with `pass` answering what the open dialog asked for (a preview, a path) before the host runs it.
#[doc(hidden)]
pub fn hosted_with<D: StackDialog, K: PartialEq + 'static>(
    ui: &mut Ui,
    data: &mut StoryData,
    knobs: K,
    open: impl FnOnce() -> D,
    pass: impl FnOnce(&mut DialogHost),
) -> StoryOutput {
    let wanted = *data.value("open", true);
    let stale = data.note::<Option<K>>("knobs", None).as_ref() != Some(&knobs);
    let host = data.note("host", DialogHost::new());
    let opened = match (wanted, host.is_open::<D>()) {
        (true, is_open) if stale || !is_open => {
            host.open(open());
            true
        }
        (false, true) => {
            host.cancel::<D>(ui.ctx());
            false
        }
        _ => false,
    };
    pass(host);
    host.run(ui);
    let is_open = host.is_open::<D>();
    let facts = host.get::<D>().map(StackDialog::story_facts).unwrap_or_default();
    if opened {
        *data.note::<Option<K>>("knobs", None) = Some(knobs);
    }
    *data.value("open", true) = is_open;
    facts
}

/// The opener of the Grid options dialogs.
const GRID_OPTIONS: &str = "Grid Options…";

/// The opener of the dialog with a title note.
const COMMENT_PROPERTIES: &str = "Comment Properties…";

/// The open state a story's dialog binds, behind the button that opens it: dialogs start closed, and the button opens
/// the real one.
pub(super) fn opener<'d>(ui: &mut Ui, data: &'d mut StoryData, label: &'static str) -> &'d mut bool {
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

/// Dialog with a title note.
#[story(Dialog<'static>, covers = [], drive = Drive::Never)]
fn dialog_with_title_note(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let shown: StoryOutput = Dialog::new("Comment Properties")
        .title_note("3 comments selected")
        .opened(Bind::bound(opener(ui, data, COMMENT_PROPERTIES)))
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
        .button_place(ButtonPlace::Side)
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

/// The two columns of the body-buttons story: the grid choices, and the column the buttons head.
const TWO_COLUMNS: &[ColumnWidth] = &[ColumnWidth::Content, ColumnWidth::Content];

/// A dialog whose body keeps the cell its buttons stand in (`ButtonPlace::Body`): the buttons head the second column,
/// over its controls, as Photoshop's Color Picker puts OK and Cancel over its second field group.
#[story(FooterSlot, covers = [Default], drive = Drive::Never)]
fn dialog_with_body_buttons(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let [snap, rulers] = &mut checks;
    let shown: StoryOutput = Dialog::new("Grid options")
        .opened(Bind::bound(opener(ui, data, GRID_OPTIONS)))
        .button_place(ButtonPlace::Body)
        .buttons([
            FooterButton::new("OK", DialogRole::Accept),
            FooterButton::new("Cancel", DialogRole::Cancel),
            FooterButton::new("Add to Swatches", DialogRole::Other).stays_open(true),
        ])
        .show(ui, |ui, _| {
            Grid::new(TWO_COLUMNS).fit(GridFit::Fixed).show(ui, |ui, _| {
                Text::new("Lines every 10 mm, in 4 subdivisions.").show(ui);
                let _ = FooterSlot::new().show(ui);
                // A labelled control in a grid takes a cell for its label: each check box is a row, one cell.
                Row::new().show(ui, |ui, _| Checkbox::new(Bind::bound(&mut *snap)).label("Snap to grid").show(ui));
                Row::new().show(ui, |ui, _| Checkbox::new(Bind::bound(&mut *rulers)).label("Show rulers").show(ui));
            });
        })
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

#[story(Dialog<'static>, covers = [], drive = Drive::Never)]
fn dialog_with_refusal(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut checks = *data.value("checks", [true, false]);
    let refused = *data.value("refused", true);
    let shown: StoryOutput = Dialog::new("Grid options")
        .opened(Bind::bound(opener(ui, data, GRID_OPTIONS)))
        .buttons([FooterButton::new("OK", DialogRole::Accept), FooterButton::new("Cancel", DialogRole::Cancel)])
        .refusal(move || refused.then(|| Label::from("The grid is finer than the document allows.")))
        .show(ui, |ui, _| body(ui, &mut checks))
        .into();
    *data.value("checks", [true, false]) = checks;
    shown
}

#[story(Dialog<'static>, covers = [], drive = Drive::Never)]
fn dialog_nudge(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    Dialog::new("Grid options")
        .opened(Bind::bound(opener(ui, data, GRID_OPTIONS)))
        .buttons([FooterButton::new("OK", DialogRole::Accept), FooterButton::new("Cancel", DialogRole::Cancel)])
        .show(ui, |ui, _| {
            Text::new("Press beside this dialog: it shakes, or its edge flashes twice with reduced motion.").show(ui);
        })
        .into()
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

/// A job window behind its opener: the window shows while the story's value is on and the value goes off with an answer.
fn job_window(ui: &mut Ui, data: &mut StoryData, window: JobProgress) -> StoryOutput {
    let open = opener(ui, data, "Show progress…");
    if *open && window.show(ui).answer.is_some() {
        *open = false;
    }
    StoryOutput::default()
}

#[story(JobProgress, covers = [], drive = Drive::Never)]
fn job_progress_counted(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let window = JobProgress::new("Exporting", "Exporting page 3 of 24").fraction(Some(0.125)).counts("3 of 24 pages").can_hide(true);
    job_window(ui, data, window)
}

#[story(JobProgress, covers = [], drive = Drive::Never)]
fn job_progress_indeterminate(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    job_window(ui, data, JobProgress::new("Exporting", "Preparing the export…").can_hide(true))
}

#[story(JobProgress, covers = [], drive = Drive::Never)]
fn job_progress_queued(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let window = JobProgress::new("Exporting", "Exporting page 3 of 24")
        .fraction(Some(0.5))
        .counts("12 of 24 pages")
        .queued("2 more exports are waiting")
        .can_hide(true);
    job_window(ui, data, window)
}

#[story(JobProgress, covers = [], drive = Drive::Never)]
fn job_progress_cancel_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    job_window(ui, data, JobProgress::new("Exporting", "Exporting page 3 of 24").fraction(Some(0.75)).counts("18 of 24 pages"))
}

#[story(JobProgress, covers = [], drive = Drive::Never)]
fn job_progress_long_title(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let window = JobProgress::new(
        "Exporting “Annual report with every regional appendix and the translated summaries” as a print-ready PDF",
        "Embedding fonts and flattening transparency on page 3 of 24",
    )
    .fraction(Some(0.125))
    .counts("3 of 24 pages")
    .can_hide(true);
    job_window(ui, data, window)
}

#[story(ModalDragStrip, covers = [Default], drive = Drive::Never)]
fn modal_drag_strip(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = ui.max_rect();
    ModalDragStrip::new(room).show(ui, |_, _| {}).into()
}
