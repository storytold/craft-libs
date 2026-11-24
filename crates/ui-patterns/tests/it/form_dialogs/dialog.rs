//! `ParamDialog` and `HostedParamDialog`: the dialog's own undo history, the Preview box's hold-to-compare, Enter in
//! a multi-line row and on a focused Cancel, Alt+Reset to `reset_to`, the held OK, the host's notes, OK state and row
//! labels and shown states, reopening under the same key, the
//! request slots and the rows an edit changed, disabled options and the remembered values a row refuses.

use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Outcome as _, Props as _, Role, SaveKey, Show as _, clicked};
use craft_ui::dialog::{DialogHost, DialogResult, HeldAnswer, HeldOk, dialog_id};
use craft_ui::patterns::form_dialogs::{
    DialogFields, FieldValue, FormButtons, FormDialog, HostedParamDialog, PagerStep, PanePager, Param, ParamDialog, ParamKind, ParamLayout,
    ParamResult, PreviewState,
};
use craft_ui::patterns::presets::{PresetFns, PresetRow, PresetSource};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::memory::{Lifetime, saved_value};
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::testing::relations::{above, left_of};
use craft_ui::testing::{Fixture, trial};
use egui::load::SizedTexture;
use egui::{Key, Modifiers, TextureId, Vec2, vec2};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

const SPEC: &str = r#"{"radius":0..100=10,"smooth":bool}"#;

// A slider drag is one step and a check box click another: Undo takes them back one by one, Redo brings the drag
// back.
#[trial]
fn undo_steps_the_dialog_values_and_a_drag_is_one_step() {
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        ParamDialog::new("blur", "Blur", Param::parse_list(SPEC)).values(Bind::bound(&mut *shown.borrow_mut())).history(true).show(ui);
    });
    fixture.settle();
    let radius = || values.borrow().number("radius");
    let smooth = || values.borrow().bool("smooth").unwrap_or(false);
    let opened = radius();

    let slider = fixture.by_role(Role::SLIDER).into_iter().next().expect("the radius row has a slider");
    fixture.drag(&slider, vec2(60.0, 0.0)).settle();
    let dragged = radius();
    assert_ne!(dragged, opened, "the drag moved the radius");
    let check = fixture.by_label("Smooth").expect("the smooth row has a check box");
    fixture.click(&check).settle();
    assert!(smooth());

    fixture.key(Key::Z, Modifiers::COMMAND).settle();
    assert_eq!((radius(), smooth()), (dragged, false), "the first Undo takes back the click");
    fixture.key(Key::Z, Modifiers::COMMAND).settle();
    assert_eq!(radius(), opened, "the second Undo takes back the whole drag");
    fixture.key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT).settle();
    assert_eq!(radius(), dragged, "Redo brings the drag back");
}

// Holding the Preview box compares and leaves the preview on; the release turns it off.
#[trial]
fn holding_the_preview_box_compares_and_the_release_toggles_it() {
    let seen = Rc::new(Cell::new((false, false)));
    let out = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let picture = SizedTexture::new(TextureId::default(), vec2(64.0, 48.0));
        let shown = ParamDialog::new("blur", "Blur", Param::parse_list(SPEC)).previews(true).preview(PreviewState::Ready(picture)).show(ui);
        out.set((shown.preview_on, shown.comparing));
    });
    fixture.settle();
    assert_eq!(seen.get(), (true, false));
    let preview = fixture
        .by_role(Role::CHECK_BOX)
        .into_iter()
        .find(|target| fixture.tree().name_of(target.node) == Some("Preview"))
        .expect("the footer has the Preview box");
    fixture.press(&preview);
    fixture.idle(1);
    assert_eq!(seen.get(), (true, true), "held: the preview stays on and the pane compares");
    fixture.release(&preview).settle();
    assert_eq!(seen.get(), (false, false), "the release turns the preview off");
}

/// The rects of the radius slider, its number field, the Preview box and OK in a previewing blur dialog laid out as
/// `layout`.
fn laid_out(layout: ParamLayout) -> [egui::Rect; 4] {
    let mut fixture = Fixture::new(move |ui| {
        let picture = SizedTexture::new(TextureId::default(), vec2(64.0, 48.0));
        ParamDialog::new("blur", "Blur", Param::parse_list(SPEC)).previews(true).preview(PreviewState::Ready(picture)).layout(layout).show(ui);
    });
    fixture.settle();
    let slider = fixture.by_role(Role::SLIDER).into_iter().next().expect("the radius row has a slider");
    let field = fixture.by_role(Role::SPIN_BUTTON).into_iter().next().expect("the radius row has a number field");
    let preview = fixture
        .by_role(Role::CHECK_BOX)
        .into_iter()
        .find(|target| fixture.tree().name_of(target.node) == Some("Preview"))
        .expect("the footer has the Preview box");
    let ok = fixture.by_label("OK").expect("the footer has OK");
    [slider.rect, field.rect, preview.rect, ok.rect]
}

// Stacked puts each slider under its number field and the buttons across the foot, the Preview box before them;
// Below stands the buttons beside the body.
#[trial]
fn the_stacked_layout_stacks_sliders_and_puts_the_buttons_across_the_foot() {
    let [slider, field, preview, ok] = laid_out(ParamLayout::Stacked);
    assert!(above(field, slider), "the number field stands over the slider");
    assert!(above(slider, ok) && above(slider, preview), "the footer is under the rows");
    assert!(left_of(preview, ok), "the Preview box comes before OK on the footer line");
    let [slider, _, preview, ok] = laid_out(ParamLayout::Below);
    assert!(left_of(slider, ok), "the buttons stand beside the body");
    assert!(above(ok, preview), "the Preview box is under the buttons");
}

/// The notes and whether the dialog answered OK after typing "a", Enter, "b" in a Notes row of `lines` lines.
fn enter_in_notes(lines: Option<usize>) -> (String, bool) {
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let accepted = Rc::new(Cell::new(false));
    let (shown, ok) = (values.clone(), accepted.clone());
    let mut fixture = Fixture::new(move |ui| {
        let notes = Param::new("notes", "Notes", ParamKind::text(""));
        let params = vec![lines.map_or(notes.clone(), |lines| notes.multiline(lines))];
        let out = ParamDialog::new("info", "Info", params).values(Bind::bound(&mut *shown.borrow_mut())).show(ui);
        if out.result.is_some() {
            ok.set(true);
        }
    });
    fixture.settle();
    let field = fixture.by_label("Notes").expect("the notes row has a field");
    fixture.click(&field).settle();
    fixture.type_text("a");
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    if !accepted.get() {
        fixture.type_text("b").settle();
    }
    let notes = values.borrow().text("notes").unwrap_or_default().to_owned();
    (notes, accepted.get())
}

// Enter in a multi-line row starts a new line and leaves the dialog open; in a single-line row it is the OK.
#[trial]
fn enter_in_a_multi_line_row_inserts_a_line_and_the_dialog_stays_open() {
    let (notes, accepted) = enter_in_notes(Some(3));
    assert_eq!((notes.as_str(), accepted), ("a\nb", false));
    let (_, accepted) = enter_in_notes(None);
    assert!(accepted, "Enter in a single-line row answers OK");
}

/// A Blur dialog over the host's values.
fn blur(fields: &mut DialogFields) -> ParamDialog<'_> {
    ParamDialog::new("blur", "Blur", Param::parse_list(SPEC)).values(Bind::bound(fields))
}

/// A fixture that runs `host` every pass. While `twice` is on, egui runs each frame twice and gives the frame's input
/// to its first pass only.
fn host_fixture(host: &Rc<RefCell<DialogHost>>, twice: &Rc<Cell<bool>>) -> Fixture {
    let (shown, twice) = (Rc::clone(host), Rc::clone(twice));
    let mut fixture = Fixture::new(move |ui| {
        shown.borrow_mut().run(ui);
        if twice.get() && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the frame runs twice");
        }
    });
    fixture.room(Vec2::new(1200.0, 900.0));
    fixture
}

/// `act` on the open `HostedParamDialog`; `None` when none is open.
fn on_dialog<T>(host: &Rc<RefCell<DialogHost>>, act: impl FnOnce(&mut HostedParamDialog) -> T) -> Option<T> {
    host.borrow_mut().get_mut::<HostedParamDialog>().map(act)
}

// A step dialog opened with recorded fields lists them, and a host set of one, before its first pass: an Actions step
// reads and fills its dialog in the call that opens it.
#[trial]
fn a_hosted_param_dialog_lists_its_opening_fields_and_takes_a_set_before_its_first_pass() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let fixture = Fixture::new(|_| {});
    let fields = DialogFields::from_pairs([("radius".to_owned(), FieldValue::Number(2.0))]);
    let id = host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "blur", fields, blur));
    let listed =
        |host: &Rc<RefCell<DialogHost>>| host.borrow().open_dialogs().into_iter().find(|dialog| dialog.id == id).and_then(|dialog| dialog.fields);
    let fields = listed(&host).expect("the dialog lists its fields");
    assert_eq!((fields["radius"].as_f64(), fields["smooth"].clone()), (Some(2.0), json!(false)), "the opening field and the defaults: {fields}");
    assert_eq!(host.borrow_mut().set_json_field(fixture.ctx(), id, "radius", json!(4)), Ok(()));
    assert_eq!(listed(&host).and_then(|fields| fields["radius"].as_f64()), Some(4.0), "the set lists before any pass");
}

// A hosted dialog that previews is previewing from the call that opens it, before any pass and without a picture
// (a canvas-only preview); its Preview box turns it off. A dialog that does not preview has no box.
#[trial]
fn a_hosted_dialog_that_previews_is_previewing_before_its_first_pass() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "blur", DialogFields::new(), |fields| blur(fields).previews(true)));
    assert_eq!(on_dialog(&host, |dialog| dialog.is_previewing()), Some(true), "previewing before the first pass");
    fixture.idle(4);
    let preview = fixture.by_label("Preview").expect("the Preview box without a picture");
    fixture.click(&preview).idle(4);
    assert_eq!(on_dialog(&host, |dialog| dialog.is_previewing()), Some(false), "the box turns the preview off");

    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = host_fixture(&host, &twice);
    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "blur", DialogFields::new(), blur));
    fixture.idle(4);
    assert_eq!(on_dialog(&host, |dialog| dialog.is_previewing()), Some(false), "a dialog that does not preview");
    assert!(fixture.by_label("Preview").is_none(), "has no Preview box");
}

/// Why the host refuses the held OK.
const REFUSED: &str = "The radius is too large for this layer.";

// OK of a held dialog files its answer once, in a frame that runs twice too, and waits with OK disabled; a refusal
// shows under the rows until a value changes; `confirm` files like the button, and the accepted answer closes the
// dialog with the held values.
#[trial]
fn a_held_ok_refused_keeps_the_dialog_open_and_an_accepted_one_closes() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "blur", DialogFields::new(), blur).held_ok());
    fixture.idle(10);

    twice.set(true);
    fixture.key(Key::Enter, Modifiers::NONE);
    assert_eq!(fixture.passes(), 2, "the frame ran a second pass");
    twice.set(false);
    fixture.idle(4);
    let filed = on_dialog(&host, HostedParamDialog::take_ok).flatten().expect("Enter files the held OK");
    assert_eq!(filed.id, dialog_id("blur"), "the answer names the dialog that filed it");
    assert_eq!(on_dialog(&host, HostedParamDialog::take_ok).flatten(), None, "the OK is filed once");
    assert!(host.borrow().is_open::<HostedParamDialog>(), "a held OK keeps the dialog open");
    let ok = fixture.by_label("OK").expect("the OK button");
    assert!(fixture.tree().is_disabled_key(ok.node), "OK waits disabled for the host's answer");

    assert_eq!(host.borrow().held_state(dialog_id("blur")), Some(HeldOk::Waiting), "the filed OK waits for the host");
    on_dialog(&host, |dialog| dialog.answer_ok(Err(Label::from(REFUSED))));
    assert_eq!(host.borrow().held_state(dialog_id("blur")), Some(HeldOk::Refused(Label::from(REFUSED))));
    fixture.idle(4);
    assert!(fixture.by_label(REFUSED).is_some(), "the refusal shows under the rows");
    let ok = fixture.by_label("OK").expect("the OK button");
    assert!(!fixture.tree().is_disabled_key(ok.node), "a refusal enables OK again");
    let smooth = fixture.by_label("Smooth").expect("the smooth row has a check box");
    fixture.click(&smooth).idle(4);
    assert!(fixture.by_label(REFUSED).is_none(), "a changed value clears the refusal");

    let ctx = fixture.ctx().clone();
    assert!(!host.borrow_mut().confirm::<HostedParamDialog>(&ctx), "confirm on a held dialog answers nothing yet");
    let filed = on_dialog(&host, HostedParamDialog::take_ok).flatten().expect("confirm files the held OK as the button does");
    assert_eq!(filed.fields.bool("smooth"), Some(true), "the held answer has the values confirmed");
    on_dialog(&host, |dialog| dialog.answer_ok(Ok(())));
    assert_eq!(host.borrow().held_state(dialog_id("blur")), Some(HeldOk::Accepted), "the accepted OK closes at the next pass");
    fixture.idle(4);
    let answered = host.borrow_mut().take_keyed::<HostedParamDialog>().map(|(result, _)| result).and_then(DialogResult::ok);
    assert_eq!(answered.map(|result| result.fields), Some(filed.fields), "the accepted OK closes the dialog with the held values");

    // A form that stays open after an accepted answer (Symbol Insert): the accepted OK hands the values out once, the
    // held state returns to idle and the dialog is still open with OK enabled.
    let held = Rc::new(RefCell::new(HeldAnswer::<bool>::new()));
    let (state, accepted, closed) = (Rc::clone(&held), Rc::new(Cell::new(0)), Rc::new(Cell::new(false)));
    let (count, ended) = (Rc::clone(&accepted), Rc::clone(&closed));
    let mut fixture = Fixture::new(move |ui| {
        let mut held = state.borrow_mut();
        let shown = FormDialog::own("Insert").held(&mut held).accept_stays_open(true).show(ui, |ui, _, value: &mut bool| {
            Checkbox::new(Bind::bound(value)).label("Wide").show(ui);
            true
        });
        if shown.accepted.is_some() {
            count.set(count.get() + 1);
        }
        if shown.closed.is_some() {
            ended.set(true);
        }
    });
    fixture.idle(10);
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.idle(4);
    assert!(held.borrow().is_waiting(), "Enter files the values and waits");
    let ok = fixture.by_label("OK").expect("the OK button");
    assert!(fixture.tree().is_disabled_key(ok.node), "OK waits disabled while the answer is out");
    assert!(held.borrow_mut().answer(Ok(())), "an answer was filed");
    fixture.idle(4);
    assert_eq!(accepted.get(), 1, "the accepted answer comes out once");
    assert!(!closed.get(), "an accepted answer leaves the dialog open");
    assert_eq!(held.borrow().state(), None, "the held state is idle again");
    let ok = fixture.by_label("OK").expect("the dialog is still open");
    assert!(!fixture.tree().is_disabled_key(ok.node), "OK is enabled again");
}

// The body's activation is the OK press: under `held`, a double click on a cell that calls `FormCx::accept` files the
// values once, in a frame that runs twice too, and the dialog stays open waiting with OK disabled.
#[trial]
fn a_body_accept_files_a_held_ok_once_and_waits() {
    let (held, twice, filed, closed) =
        (Rc::new(RefCell::new(HeldAnswer::<String>::new())), Rc::new(Cell::new(false)), Rc::new(RefCell::new(Vec::new())), Rc::new(Cell::new(false)));
    let (state, again, files, ended) = (Rc::clone(&held), Rc::clone(&twice), Rc::clone(&filed), Rc::clone(&closed));
    let mut fixture = Fixture::new(move |ui| {
        let mut held = state.borrow_mut();
        let shown = FormDialog::own("Insert Symbol").held(&mut held).show(ui, |ui, cx, value: &mut String| {
            let cell = Button::new("Sun").show(ui);
            if clicked(&cell) {
                *value = "Sun".to_owned();
            }
            if cell.clicks == 2 {
                cx.accept();
            }
            !value.is_empty()
        });
        if let Some(values) = held.take() {
            files.borrow_mut().push(values);
        }
        if shown.closed.is_some() {
            ended.set(true);
        }
        if again.get() && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the frame runs twice");
        }
    });
    fixture.idle(10);
    let cell = fixture.by_label("Sun").expect("the body's cell");
    fixture.click(&cell);
    assert!(filed.borrow().is_empty(), "a single click files nothing");
    twice.set(true);
    fixture.click(&cell);
    assert_eq!(fixture.passes(), 2, "the frame ran a second pass");
    twice.set(false);
    fixture.idle(4);
    assert_eq!(*filed.borrow(), ["Sun"], "the double click files the values once");
    assert!(held.borrow().is_waiting(), "the filed values wait for the app's answer");
    assert!(!closed.get(), "a held accept keeps the dialog open");
    let ok = fixture.by_label("OK").expect("the OK button");
    assert!(fixture.tree().is_disabled_key(ok.node), "OK waits disabled for the answer");
}

// A form without OK shows none, and Enter picks its one Accept-role extra, which keeps its index after Close.
#[trial]
fn a_form_without_ok_shows_none_and_enter_picks_its_accept_extra() {
    let (picked, closed) = (Rc::new(RefCell::new(Vec::new())), Rc::new(Cell::new(false)));
    let (picks, ended) = (Rc::clone(&picked), Rc::clone(&closed));
    let mut fixture = Fixture::new(move |ui| {
        let shown = FormDialog::own("Mark Entry")
            .buttons(FormButtons::Close)
            .extra_buttons([FooterButton::new("Mark All", DialogRole::Other), FooterButton::new("Mark", DialogRole::Accept)])
            .show(ui, |ui, _, value: &mut bool| {
                Checkbox::new(Bind::bound(value)).label("Bold").show(ui);
                true
            });
        picks.borrow_mut().extend(shown.picked);
        if shown.closed.is_some() {
            ended.set(true);
        }
    });
    fixture.idle(10);
    assert!(fixture.by_label("OK").is_none(), "a form without OK shows none");
    assert!(fixture.by_label("Close").is_some(), "its one closing button is Close");
    // Focus on the check box, so Enter goes to the footer's default button rather than a focused one.
    let bold = fixture.by_label("Bold").expect("the body's check box");
    fixture.click(&bold).idle(2);
    fixture.key(Key::Enter, Modifiers::NONE).idle(4);
    assert_eq!(*picked.borrow(), [3], "Enter picks Mark, the second extra");
    assert!(!closed.get(), "the extra keeps the dialog open");
}

/// A note the host's job gives the dialog.
const CHECKING: &str = "Checking pages…";

/// Why the host holds OK back.
const NOT_READY: &str = "The pages are still being checked.";

// The notes and the OK state the host sets between passes show in the next pass: the note under the rows and OK
// disabled, with `confirm` refused too; `Ok` and an empty list put OK and the rows back in the pass after.
#[trial]
fn notes_and_the_ok_state_the_host_sets_show_in_the_next_pass() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "blur", DialogFields::new(), blur));
    fixture.idle(4);
    on_dialog(&host, |dialog| {
        dialog.set_notes(vec![Label::from(CHECKING)]);
        dialog.set_ready(Err(Label::from(NOT_READY)));
    });
    fixture.idle(1);
    assert!(fixture.by_label(CHECKING).is_some(), "the note shows in the next pass");
    let ok = fixture.by_label("OK").expect("the OK button");
    assert!(fixture.tree().is_disabled_key(ok.node), "OK is disabled in the next pass");
    let ctx = fixture.ctx().clone();
    assert!(!host.borrow_mut().confirm::<HostedParamDialog>(&ctx), "confirm answers nothing while the host holds OK");
    assert!(host.borrow().is_open::<HostedParamDialog>(), "the dialog stays open");

    on_dialog(&host, |dialog| {
        dialog.set_notes(Vec::new());
        dialog.set_ready(Ok(()));
    });
    fixture.idle(1);
    assert!(fixture.by_label(CHECKING).is_none(), "an empty list shows no note");
    let ok = fixture.by_label("OK").expect("the OK button");
    assert!(!fixture.tree().is_disabled_key(ok.node), "`Ok` leaves OK to the build again");
}

/// A row label the host's job words.
const PAGES_3_TO_5: &str = "Smooth pages 3–5";

// A row label and a hidden row the host sets between passes show in the next pass, over what `build` gives: the row
// takes the host's words, and a hidden row is gone and refuses a set through automation until the host shows it.
#[trial]
fn a_row_label_and_shown_state_the_host_sets_show_in_the_next_pass() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "blur", DialogFields::new(), blur));
    fixture.idle(4);
    on_dialog(&host, |dialog| dialog.set_param_label("smooth", PAGES_3_TO_5));
    fixture.idle(1);
    assert!(fixture.by_label(PAGES_3_TO_5).is_some(), "the row shows the host's label");
    assert!(fixture.by_label("Smooth").is_none(), "the build's label is replaced");

    on_dialog(&host, |dialog| dialog.set_param_shown("smooth", false));
    fixture.idle(1);
    assert!(fixture.by_label(PAGES_3_TO_5).is_none(), "the hidden row is gone");
    let ctx = fixture.ctx().clone();
    assert!(host.borrow_mut().set_json_field(&ctx, dialog_id("blur"), "smooth", json!(true)).is_err(), "a hidden row refuses a set");
    assert!(fixture.by_label("Radius").is_some(), "the other rows stay");

    on_dialog(&host, |dialog| dialog.set_param_shown("smooth", true));
    fixture.idle(1);
    assert!(fixture.by_label(PAGES_3_TO_5).is_some(), "the row shows again");
    assert_eq!(host.borrow_mut().set_json_field(&ctx, dialog_id("blur"), "smooth", json!(true)), Ok(()));
}

// A dialog the app opens again under the same key, in the pass it takes the first one's Cancel, shows and answers.
#[trial]
fn a_hosted_dialog_reopened_under_the_same_key_opens() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let reopen = Rc::new(Cell::new(true));
    let answered = Rc::new(RefCell::new(None::<DialogResult<ParamResult>>));
    let (shown, again, out) = (Rc::clone(&host), Rc::clone(&reopen), Rc::clone(&answered));
    let mut fixture = Fixture::new(move |ui| {
        let mut host = shown.borrow_mut();
        host.run(ui);
        if let Some(result) = host.take_keyed::<HostedParamDialog>().map(|(result, _)| result) {
            if again.replace(false) {
                host.open(HostedParamDialog::new(ui.ctx(), "blur", DialogFields::new(), blur));
            } else {
                *out.borrow_mut() = Some(result);
            }
        }
    });
    fixture.room(Vec2::new(1200.0, 900.0));
    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "blur", DialogFields::new(), blur));
    fixture.idle(10);
    fixture.key(Key::Escape, Modifiers::NONE).idle(10);
    assert!(!reopen.get(), "Escape closed the first dialog");
    assert!(fixture.by_label("Blur").is_some(), "the dialog opened again under the same key shows");
    fixture.key(Key::Enter, Modifiers::NONE).idle(4);
    assert!(matches!(*answered.borrow(), Some(DialogResult::Ok(_))), "the reopened dialog answers OK");
}

/// A previewing dialog with a files row, a custom row whose button asks for the `reload` action and two footer
/// buttons that keep it open.
fn stack_dialog(fields: &mut DialogFields) -> ParamDialog<'_> {
    let reload = Param::custom("reload", "Reload Sources", |ui, _, cx| {
        let shown = Button::new("Reload Sources").show(ui);
        if clicked(&shown) {
            cx.request_action("reload");
        }
        shown.response().clone()
    });
    ParamDialog::new("stack", "Load Files into Stack", vec![Param::files("sources", "Source Files").folder_mode(true), reload])
        .values(Bind::bound(fields))
        .extra_buttons(vec![
            ("load".to_owned(), FooterButton::new(Label::from("Load…"), DialogRole::Other)),
            ("save".to_owned(), FooterButton::new(Label::from("Save…"), DialogRole::Other)),
        ])
        .previews(true)
}

/// The `{files, folder, useFolder}` value of a files row that lists `folder`.
fn folder_value(folder: &str) -> serde_json::Value {
    json!({ "files": [], "folder": folder, "useFolder": true })
}

// A footer button and a custom row's button that keep the dialog open are reported once by their id on the one
// action channel, and a press on the preview pager once as its step, in a frame that runs twice too; a folder listing
// is asked for once; `cancel_requests` drops them all.
#[trial]
fn a_hosted_dialogs_new_requests_come_once_and_a_cancel_drops_them() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "stack", DialogFields::new(), stack_dialog));
    let picture = SizedTexture::new(TextureId::default(), vec2(64.0, 48.0));
    let pager = PanePager { shown: 1, count: 3, back: false, next: true, ..PanePager::default() };
    on_dialog(&host, |dialog| {
        dialog.set_preview(Some(PreviewState::Ready(picture)));
        dialog.set_preview_pager(Some(pager));
    });
    fixture.idle(10);
    let press_twice = |fixture: &mut Fixture, label: &str| {
        let target = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} button"));
        twice.set(true);
        fixture.click(&target);
        assert_eq!(fixture.passes(), 2, "the frame ran a second pass");
        twice.set(false);
        fixture.idle(4);
    };

    press_twice(&mut fixture, "Save…");
    assert_eq!(on_dialog(&host, HostedParamDialog::take_action).flatten().as_deref(), Some("save"), "the footer press is reported by its id");
    assert_eq!(on_dialog(&host, HostedParamDialog::take_action).flatten(), None, "the footer press is reported once");
    assert!(host.borrow().is_open::<HostedParamDialog>(), "the button keeps the dialog open");
    press_twice(&mut fixture, "Reload Sources");
    assert_eq!(on_dialog(&host, HostedParamDialog::take_action).flatten().as_deref(), Some("reload"), "the row's press is reported by its id");
    assert_eq!(on_dialog(&host, HostedParamDialog::take_action).flatten(), None, "the row's press is reported once");
    press_twice(&mut fixture, "Next");
    assert_eq!(on_dialog(&host, HostedParamDialog::take_preview_step).flatten(), Some(PagerStep::Next), "the pager press is its step");
    assert_eq!(on_dialog(&host, HostedParamDialog::take_preview_step).flatten(), None, "the step is reported once");

    let ctx = fixture.ctx().clone();
    host.borrow_mut().set_json_field(&ctx, dialog_id("stack"), "sources", folder_value("/photos/shoot")).expect("the files row takes a folder");
    fixture.idle(4);
    let listing = on_dialog(&host, HostedParamDialog::take_listing).flatten().expect("a new folder asks for its files");
    assert_eq!((listing.key.as_str(), listing.folder), ("sources", PathBuf::from("/photos/shoot")));
    assert_eq!(on_dialog(&host, HostedParamDialog::take_listing).flatten(), None, "the listing is asked for once");

    let load = fixture.by_label("Load…").expect("the Load… button");
    fixture.click(&load).idle(2);
    let next = fixture.by_label("Next").expect("the pager's Next button");
    fixture.click(&next).idle(2);
    host.borrow_mut().set_json_field(&ctx, dialog_id("stack"), "sources", folder_value("/photos/other")).expect("the files row takes a folder");
    fixture.idle(4);
    on_dialog(&host, HostedParamDialog::cancel_requests);
    assert_eq!(on_dialog(&host, HostedParamDialog::take_action).flatten(), None, "a cancel drops the footer press");
    assert_eq!(on_dialog(&host, HostedParamDialog::take_preview_step).flatten(), None, "a cancel drops the pager step");
    assert_eq!(on_dialog(&host, HostedParamDialog::take_listing).flatten(), None, "a cancel drops the listing request");
}

/// A dialog with a custom row whose button asks the host to run the action `pen`.
fn pen_dialog(fields: &mut DialogFields) -> ParamDialog<'_> {
    let pen = Param::custom("pen", "Pen", |ui, _, cx| {
        let pressed = Button::new("Pen…").show(ui);
        if clicked(&pressed) {
            cx.request_action("pen");
        }
        pressed.response().clone()
    });
    ParamDialog::new("pens", "Pens", vec![pen]).values(Bind::bound(fields))
}

// A custom row's action request reaches the host once, in a frame that runs twice too.
#[trial]
fn a_custom_rows_action_request_reaches_the_host_once() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "pens", DialogFields::new(), pen_dialog));
    fixture.idle(10);
    let pen = fixture.by_label("Pen").expect("the row's button, named by the row");
    twice.set(true);
    fixture.click(&pen);
    twice.set(false);
    fixture.idle(4);
    assert_eq!(on_dialog(&host, HostedParamDialog::take_actions), Some(vec!["pen".to_owned()]), "the press asks for the action");
    assert_eq!(on_dialog(&host, HostedParamDialog::take_actions), Some(Vec::new()), "the request is taken once");
}

// An edit of a row reports its key once, in a frame that runs twice too, and a set through automation reports its
// key; passes with no edit report none.
#[trial]
fn a_hosted_dialog_reports_the_rows_an_edit_changed_once() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "blur", DialogFields::new(), blur));
    fixture.idle(10);
    let edited = || on_dialog(&host, HostedParamDialog::take_edited).unwrap_or_default();
    assert_eq!(edited(), Vec::<String>::new(), "opening and passes with no edit report none");

    let smooth = fixture.by_label("Smooth").expect("the smooth row has a check box");
    twice.set(true);
    fixture.click(&smooth);
    assert_eq!(fixture.passes(), 2, "the frame ran a second pass");
    twice.set(false);
    fixture.idle(4);
    assert_eq!(edited(), ["smooth"], "the edit reports its row");
    fixture.idle(4);
    assert_eq!(edited(), Vec::<String>::new(), "the edit is reported once");

    let ctx = fixture.ctx().clone();
    host.borrow_mut().set_json_field(&ctx, dialog_id("blur"), "radius", json!(40)).expect("the radius row takes a number");
    assert_eq!(edited(), ["radius"], "a set through automation reports its row");
}

/// A dialog with a radio row whose second option is disabled.
fn brush_tip(fields: &mut DialogFields) -> ParamDialog<'_> {
    let options = vec![("round".to_owned(), Label::from("Round")), ("square".to_owned(), Label::from("Square"))];
    let shape = Param::new("shape", "Shape", ParamKind::radio(options, Some("round".to_owned()))).disabled_options(["square"]);
    ParamDialog::new("brush", "Brush Tip", vec![shape]).values(Bind::bound(fields))
}

// A disabled radio option is a disabled node, a click on it changes nothing, and the control surface is refused it.
#[trial]
fn a_disabled_radio_option_is_greyed_and_refused() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "brush", DialogFields::new(), brush_tip));
    fixture.idle(10);
    let shape = || on_dialog(&host, |dialog| dialog.fields().text("shape").map(str::to_owned)).flatten();

    let square = fixture.by_label("Square").expect("the disabled option shows");
    assert!(fixture.tree().is_disabled_key(square.node), "the disabled option is a disabled node");
    fixture.click(&square).idle(4);
    assert_eq!(shape().as_deref(), Some("round"), "a click on the disabled option changes nothing");

    let ctx = fixture.ctx().clone();
    let refused = host.borrow_mut().set_json_field(&ctx, dialog_id("brush"), "shape", json!("square"));
    assert!(refused.as_ref().is_err_and(|message| message.contains("`shape` can't take")), "the disabled option is refused: {refused:?}");
    assert_eq!(shape().as_deref(), Some("round"), "the refused value is not set");
}

/// The filter's parameters; `narrow` is the next version, whose rows take less.
fn filter_params(narrow: bool) -> Vec<Param> {
    let spec = if narrow {
        r#"{"radius":0..50=10,"method":"spin|zoom","smooth":bool}"#
    } else {
        r#"{"radius":0..100=10,"method":"spin|zoom|twist","smooth":bool}"#
    };
    let edges = vec![("wrap".to_owned(), Label::from("Wrap")), ("repeat".to_owned(), Label::from("Repeat"))];
    let edge = Param::new("edge", "Edge", ParamKind::radio(edges, Some("wrap".to_owned())));
    let mut params = Param::parse_list(spec);
    params.push(if narrow { edge.disabled_options(["repeat"]) } else { edge });
    params.push(Param::new("pattern", "Pattern", ParamKind::pattern(Some("dots".to_owned()))));
    params
}

/// A remembering filter dialog; `narrow` offers fewer patterns.
fn filter(fields: &mut DialogFields, narrow: bool) -> ParamDialog<'_> {
    let ids: &'static [&'static str] = if narrow { &["dots"] } else { &["dots", "stripes"] };
    ParamDialog::new("filter", "Filter", filter_params(narrow))
        .values(Bind::bound(fields))
        .remember(SaveKey::new("trial.filter"))
        .patterns(PresetSource::new(ids, PresetFns::new(|ids, _| ids.iter().map(|id| PresetRow::item((*id).to_owned(), *id, 0)).collect())))
}

fn wide_filter(fields: &mut DialogFields) -> ParamDialog<'_> {
    filter(fields, false)
}

fn narrow_filter(fields: &mut DialogFields) -> ParamDialog<'_> {
    filter(fields, true)
}

// The open path keeps a remembered value only where its row takes it: a value past a range that shrank, an option no
// longer offered, a disabled option and a pattern the dialog no longer offers open as the row's default, and a value
// the row still takes opens as remembered.
#[trial]
fn a_remembered_value_its_row_refuses_opens_as_the_default() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        let mut host = shown.borrow_mut();
        host.run(ui);
        // The app remembers an answer where it takes it, after its command ran.
        if let Some(DialogResult::Ok(result)) = host.take_keyed::<HostedParamDialog>().map(|(result, _)| result) {
            result.remember(ui.ctx());
        }
    });
    fixture.room(Vec2::new(1200.0, 900.0));
    let ctx = fixture.ctx().clone();

    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "filter", DialogFields::new(), wide_filter));
    fixture.idle(4);
    let values =
        [("radius", json!(80.0)), ("method", json!("twist")), ("edge", json!("repeat")), ("pattern", json!("stripes")), ("smooth", json!(true))];
    for (field, value) in values {
        host.borrow_mut().set_json_field(&ctx, dialog_id("filter"), field, value).expect("the first version takes each value");
    }
    assert!(host.borrow_mut().confirm::<HostedParamDialog>(&ctx), "OK answers");
    fixture.idle(2);

    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "filter", DialogFields::new(), narrow_filter));
    fixture.idle(4);
    let fields = host.borrow().get::<HostedParamDialog>().map(|dialog| dialog.fields().clone()).expect("the next version is open");
    assert_eq!(fields.bool("smooth"), Some(true), "a value its row still takes opens as remembered");
    assert_eq!(fields.number("radius"), Some(10.0), "a value past the shrunk range opens as the default");
    assert_eq!(fields.text("method"), Some("spin"), "an option no longer offered opens as the default");
    assert_eq!(fields.text("edge"), Some("wrap"), "a disabled option opens as the default");
    assert_eq!(fields.text("pattern"), Some("dots"), "a pattern no longer offered opens as the default");
}

// With Cancel focused, Enter cancels: the footer's Enter goes to the focused button, not to OK.
#[trial]
fn enter_on_a_focused_cancel_cancels_the_form() {
    let seen = Rc::new(Cell::new(None::<(CloseReason, bool)>));
    let out = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let shown = ParamDialog::new("blur", "Blur", Param::parse_list(SPEC)).show(ui);
        if let Some(closed) = shown.closed {
            out.set(Some((closed, shown.result.is_some())));
        }
    });
    fixture.settle();
    let cancel = fixture.by_label("Cancel").expect("the Cancel button");
    let on_cancel = |fixture: &Fixture| craft_ui::testing::access::focused_key(fixture.tree()) == cancel.node;
    for _ in 0..16 {
        if on_cancel(&fixture) {
            break;
        }
        fixture.key(Key::Tab, Modifiers::NONE);
    }
    assert!(on_cancel(&fixture), "Tab reaches Cancel");
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    let closed = seen.get();
    assert!(matches!(closed, Some((reason, false)) if !matches!(reason, CloseReason::Activated | CloseReason::Enter)), "Enter cancels: {closed:?}");
}

// One Escape in a number field holding typed text cancels the dialog, as in Word and the old apps: the draft does
// not take the Escape first, and the dialog answers no OK, so the typing goes with it. A field with nothing typed
// leaves the Escape to the dialog too, also where Escape otherwise takes a field's focus (`escape_leaves_fields`).
#[trial]
fn one_escape_over_a_typed_draft_cancels_the_dialog() {
    for leaves in [false, true] {
        for typed in [true, false] {
            let seen = Rc::new(Cell::new(None::<(CloseReason, bool)>));
            let out = seen.clone();
            let mut fixture = Fixture::new(move |ui| {
                let dialog = ParamDialog::new("blur", "Blur", Param::parse_list(SPEC)).show(ui);
                if let Some(closed) = dialog.closed {
                    out.set(Some((closed, dialog.result.is_some())));
                }
            });
            let mut root = craft_ui::runtime::env::RootEnv::new();
            root.escape_leaves_fields = leaves;
            fixture.root(root).settle();
            let field = fixture.by_role(Role::SPIN_BUTTON).into_iter().next().expect("the radius row has a number field");
            fixture.click(&field).settle();
            if typed {
                fixture.key(Key::A, Modifiers::COMMAND);
                fixture.type_text("55").settle();
                let draft =
                    fixture.by_role(Role::SPIN_BUTTON).into_iter().next().and_then(|field| fixture.tree().value_of(field.node).map(str::to_owned));
                assert_eq!(draft.as_deref(), Some("55"), "the field shows the typing as a draft");
            }
            fixture.key(Key::Escape, Modifiers::NONE).settle();
            assert!(
                matches!(seen.get(), Some((CloseReason::Escape, false))),
                "leaves {leaves}, typed {typed}: one Escape cancels the dialog: {:?}",
                seen.get()
            );
        }
    }
}

// Alt turns Cancel into Reset, which restores the opening values with the `reset_to` ones over them: a key `reset_to`
// lacks returns to its opening value, and a key without a row added since the open goes.
#[trial]
fn alt_reset_restores_the_opening_values_with_the_reset_to_values_over_them() {
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        ParamDialog::new("blur", "Blur", Param::parse_list(SPEC))
            .values(Bind::bound(&mut *shown.borrow_mut()))
            .reset_to(DialogFields::from_pairs([("radius", 50.0)]))
            .show(ui);
    });
    fixture.settle();
    let opening = values.borrow().bool("smooth");
    let smooth = fixture.by_label("Smooth").expect("the smooth row has a check box");
    fixture.click(&smooth).settle();
    assert_ne!(values.borrow().bool("smooth"), opening);
    values.borrow_mut().set("points", FieldValue::Int(3));
    fixture.settle();

    fixture.hold(Modifiers::ALT).idle(2);
    let reset = fixture.by_label("Reset").expect("Alt turns Cancel into Reset");
    fixture.click(&reset).idle(2);
    fixture.hold(Modifiers::NONE).settle();
    assert_eq!(values.borrow().number("radius"), Some(50.0), "Reset sets the reset_to value");
    assert_eq!(values.borrow().bool("smooth"), opening, "a key reset_to lacks returns to its opening value");
    assert!(!values.borrow().contains("points"), "a key without a row set since the open goes");
    assert!(fixture.by_label("Blur").is_some(), "Reset keeps the dialog open");
}

/// A blur dialog that previews, over `canvas`, whose pane centres on clicks when `centres`.
fn locating(canvas: egui::Id, centres: bool) -> impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static {
    move |fields| blur(fields).previews(true).canvas(canvas).click_centres(centres)
}

// The image under a dialog whose preview pane centres on clicks is the pane's locator: a primary press on the canvas
// passes the modal layer while the pane shows. Before the pane has a picture, or with a pane that does not centre on
// clicks, the press stays blocked.
#[trial]
fn a_preview_that_centres_on_clicks_passes_a_press_on_the_canvas_under_it() {
    let (host, canvas, presses) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(None)), Rc::new(Cell::new(0)));
    let (shown, seen, counted) = (Rc::clone(&host), Rc::clone(&canvas), Rc::clone(&presses));
    let mut fixture = Fixture::new(move |ui| {
        let surface = Button::new("Canvas").show(ui);
        seen.set(Some((surface.response().id, surface.response().rect.center())));
        if clicked(&surface) {
            counted.set(counted.get() + 1);
        }
        shown.borrow_mut().run(ui);
    });
    fixture.room(Vec2::new(1200.0, 900.0));
    fixture.settle();
    let (id, at) = canvas.get().expect("the canvas showed");
    let picture = SizedTexture::new(TextureId::default(), vec2(64.0, 48.0));
    // The modal layer takes the canvas out of the access tree: the press goes to the canvas's point.
    let press = |fixture: &mut Fixture| {
        fixture.click_at(at).settle();
    };

    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "blur", DialogFields::new(), locating(id, true)));
    fixture.idle(4);
    press(&mut fixture);
    assert_eq!(presses.get(), 0, "no pane yet: the modal layer blocks the press");
    on_dialog(&host, |dialog| dialog.set_preview(Some(PreviewState::Ready(picture))));
    fixture.idle(4);
    press(&mut fixture);
    assert_eq!(presses.get(), 1, "with the pane shown the press reaches the canvas");

    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "blur", DialogFields::new(), locating(id, false)));
    on_dialog(&host, |dialog| dialog.set_preview(Some(PreviewState::Ready(picture))));
    fixture.idle(4);
    press(&mut fixture);
    assert_eq!(presses.get(), 1, "a pane that does not centre on clicks leaves the press blocked");
}

/// A dialog with one colour row.
fn tint(fields: &mut DialogFields) -> ParamDialog<'_> {
    ParamDialog::new("tint", "Tint", vec![Param::new("tint", "Tint", ParamKind::color(None))]).values(Bind::bound(fields))
}

/// Whether any rectangle of `shapes` is filled with `color`.
fn paints(shapes: &[egui::Shape], color: egui::Color32) -> bool {
    shapes.iter().any(|shape| match shape {
        egui::Shape::Rect(rect) => rect.fill == color,
        egui::Shape::Vec(inner) => paints(inner, color),
        _ => false,
    })
}

// A colour the host previews (a child picker's live colour) shows in its row and leaves the dialog's values alone;
// ending the preview shows the stored colour again.
#[trial]
fn a_previewed_colour_shows_in_its_row_and_leaves_the_fields() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    let (red, blue) = (craft_theme::Srgba::new(1.0, 0.0, 0.0, 1.0), craft_theme::Srgba::new(0.0, 0.0, 1.0, 1.0));
    host.borrow_mut().open(HostedParamDialog::new(
        fixture.ctx(),
        "tint",
        DialogFields::from_pairs([("tint", FieldValue::Color([0.0, 0.0, 1.0]))]),
        tint,
    ));
    fixture.idle(10);
    let before = on_dialog(&host, |dialog| dialog.fields().clone()).expect("the dialog is open");
    assert!(paints(&fixture.frame_shapes(), blue.into()), "the swatch shows the stored colour");

    on_dialog(&host, |dialog| dialog.preview_color("tint", Some(red)));
    fixture.idle(4);
    assert!(paints(&fixture.frame_shapes(), red.into()), "the swatch shows the previewed colour");
    assert_eq!(on_dialog(&host, |dialog| dialog.fields().clone()), Some(before), "the preview leaves the fields");

    on_dialog(&host, |dialog| dialog.preview_color("tint", None));
    fixture.idle(4);
    assert!(paints(&fixture.frame_shapes(), blue.into()), "ending the preview shows the stored colour again");
}

// A field the host sets before the dialog's first pass is a change from the values the dialog opened with: Reset
// restores the opening value.
#[trial]
fn a_field_set_before_the_first_show_resets_to_the_opening_value() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "blur", DialogFields::new(), blur));
    let ctx = fixture.ctx().clone();
    host.borrow_mut().set_json_field(&ctx, dialog_id("blur"), "radius", json!(60.0)).expect("the radius row takes a number");
    fixture.idle(10);
    assert_eq!(on_dialog(&host, |dialog| dialog.fields().number("radius")).flatten(), Some(60.0), "the set value shows");

    fixture.hold(Modifiers::ALT).idle(2);
    let reset = fixture.by_label("Reset").expect("Alt turns Cancel into Reset, enabled while the values differ");
    fixture.click(&reset).idle(2);
    fixture.hold(Modifiers::NONE).settle();
    assert_eq!(on_dialog(&host, |dialog| dialog.fields().number("radius")).flatten(), Some(10.0), "Reset restores the opening value");
}

/// A Sharpen dialog over the host's values.
fn sharpen(fields: &mut DialogFields) -> ParamDialog<'_> {
    ParamDialog::new("sharpen", "Sharpen", Param::parse_list(SPEC)).values(Bind::bound(fields))
}

// Hosted dialogs of two commands stay open together and each answers by its key; the same command opened again
// replaces only its own dialog.
#[trial]
fn hosted_dialogs_of_two_commands_stay_open_together() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    let ctx = fixture.ctx().clone();
    host.borrow_mut().open(HostedParamDialog::new(&ctx, "blur", DialogFields::new(), blur));
    host.borrow_mut().open(HostedParamDialog::new(&ctx, "sharpen", DialogFields::new(), sharpen));
    fixture.idle(4);
    assert_eq!(host.borrow().stack().ids().count(), 2, "the second command's dialog leaves the first open");

    host.borrow_mut().open(HostedParamDialog::new(&ctx, "blur", DialogFields::new(), blur));
    fixture.idle(4);
    let replaced = host.borrow_mut().take_keyed::<HostedParamDialog>();
    assert!(matches!(replaced, Some((DialogResult::Cancel, Some(ref key))) if key == "blur"), "the same command replaces its own dialog");
    assert!(host.borrow().get_by_id::<HostedParamDialog>(dialog_id("sharpen")).is_some(), "the other command's dialog stays open");

    for key in ["blur", "sharpen"] {
        assert!(host.borrow_mut().confirm_by_id(&ctx, dialog_id(key)), "{key}: OK answers");
        let answered = host.borrow_mut().take_keyed::<HostedParamDialog>();
        assert!(
            matches!(answered, Some((DialogResult::Ok(ref result), Some(ref named))) if named == key && result.key == key),
            "{key}: its OK comes back by its key"
        );
    }
}

/// Global Light (#14980): an angle row over -360 to 360 in whole degrees and a range row 0 to 90.
fn light(fields: &mut DialogFields) -> ParamDialog<'_> {
    let params = vec![
        Param::new("angle", "Angle", ParamKind::angle_in(-360.0, 360.0, 120.0, 0)),
        Param::new("altitude", "Altitude", ParamKind::range(0.0, 90.0, 30.0, craft_ui::value_drag::SliderScale::Linear)).decimals(0),
        Param::new("size", "Size", ParamKind::int(Some(1), Some(300_000), 100)),
    ];
    ParamDialog::new("light", "Global Light", params).values(Bind::bound(fields)).alert_title("PhotoCraft")
}

// A value typed past a row's range is clamped and the hosted dialog keeps one range alert until the host takes it; a
// value in range, a step key and text that is no number raise none. 180 stays 180, and a typed fraction rounds to the
// row's whole degrees.
#[trial]
fn a_typed_value_past_a_rows_range_is_clamped_and_the_host_takes_one_range_alert() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "light", DialogFields::new(), light));
    fixture.idle(6);
    let enter = |fixture: &mut Fixture, label: &str, text: &str| {
        let field = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} field"));
        fixture.click(&field).settle();
        fixture.key(Key::A, Modifiers::COMMAND).type_text(text);
        fixture.key(Key::Tab, Modifiers::NONE).settle();
    };
    let number = |key: &str| on_dialog(&host, |dialog| dialog.fields().number(key)).flatten();
    let alert = || on_dialog(&host, |dialog| dialog.take_range_alert().is_some()).unwrap_or(false);

    enter(&mut fixture, "Angle in degrees", "400");
    assert_eq!(number("angle"), Some(360.0), "400 is clamped to 360");
    assert!(alert(), "the clamp raises the range alert");
    assert!(!alert(), "the alert is taken once");
    enter(&mut fixture, "Angle in degrees", "180");
    assert_eq!(number("angle"), Some(180.0), "180 stays 180");
    enter(&mut fixture, "Angle in degrees", "12.6");
    assert_eq!(number("angle"), Some(13.0), "a fraction rounds to whole degrees");
    assert!(!alert(), "values in range raise no alert");

    let field = fixture.by_label("Angle in degrees").expect("the angle field");
    fixture.click(&field).settle();
    fixture.key(Key::ArrowUp, Modifiers::NONE).settle();
    assert_eq!(number("angle"), Some(14.0), "Up steps one degree");
    fixture.key(Key::ArrowUp, Modifiers::SHIFT).settle();
    assert_eq!(number("angle"), Some(20.0), "Shift+Up steps to the next ten");
    enter(&mut fixture, "Angle in degrees", "355");
    fixture.click(&field).settle();
    fixture.key(Key::ArrowUp, Modifiers::SHIFT).settle();
    assert_eq!(number("angle"), Some(360.0), "a step stops at the end");
    assert!(!alert(), "a step raises no alert");

    enter(&mut fixture, "Altitude", "95");
    assert_eq!(number("altitude"), Some(90.0), "the range row clamps 95 to 90");
    assert!(alert(), "the range row raises the alert too");

    enter(&mut fixture, "Size", "0");
    assert_eq!(on_dialog(&host, |dialog| dialog.fields().int("size")).flatten(), Some(1), "an int row clamps 0 to 1");
    assert!(alert(), "the int row raises the alert");
    enter(&mut fixture, "Size", "abc");
    assert_eq!(on_dialog(&host, |dialog| dialog.fields().int("size")).flatten(), Some(1), "text that is no number leaves the value");
    assert!(!alert(), "and raises no alert");
}

/// Runs a one-row dialog for `row`; the values of its answer are filed in the returned cell.
fn single_row(row: Param) -> (Fixture, Rc<RefCell<Option<DialogFields>>>) {
    let answered = Rc::new(RefCell::new(None::<DialogFields>));
    let out = Rc::clone(&answered);
    let fixture = Fixture::new(move |ui| {
        if let Some(result) = ParamDialog::new("row", "Row", vec![row.clone()]).show(ui).result {
            *out.borrow_mut() = Some(result.fields);
        }
    });
    (fixture, answered)
}

// Text that does not parse in a number row holds OK: it is disabled and Enter answers nothing; text that parses
// answers OK with the new value.
#[trial]
fn a_row_whose_text_does_not_parse_holds_ok() {
    let (mut fixture, answered) = single_row(Param::new("size", "Size", ParamKind::int(Some(1), Some(100), 10)));
    fixture.settle();
    let field = fixture.by_label("Size").expect("the size row has a field");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND).type_text("abc");
    fixture.idle(2);
    let ok = fixture.by_label("OK").expect("the footer has OK");
    assert!(fixture.tree().is_disabled_key(ok.node), "OK waits while the text does not parse");
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert!(answered.borrow().is_none(), "Enter answers nothing while the text does not parse");

    fixture.key(Key::A, Modifiers::COMMAND).type_text("42");
    fixture.idle(2);
    let ok = fixture.by_label("OK").expect("the footer has OK");
    assert!(!fixture.tree().is_disabled_key(ok.node), "text that parses frees OK");
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(answered.borrow().as_ref().and_then(|fields| fields.int("size")), Some(42), "OK answers the typed value");
}

// Shift+Enter in a row's field confirms the dialog with the typed value.
#[trial]
fn shift_enter_in_a_row_answers_ok() {
    let (mut fixture, answered) = single_row(Param::new("notes", "Notes", ParamKind::text("")));
    fixture.settle();
    let field = fixture.by_label("Notes").expect("the notes row has a field");
    fixture.click(&field).settle();
    fixture.type_text("a");
    fixture.key(Key::Enter, Modifiers::SHIFT).settle();
    assert_eq!(answered.borrow().as_ref().and_then(|fields| fields.text("notes").map(str::to_owned)).as_deref(), Some("a"));
}

// Enter in an open select picks the highlighted option; it does not answer OK, and the dialog stays open with the pick.
#[trial]
fn an_open_select_takes_enter_and_the_dialog_stays_open() {
    let options = vec![("spin".to_owned(), Label::from("Spin")), ("zoom".to_owned(), Label::from("Zoom"))];
    let (mut fixture, answered) = single_row(Param::new("method", "Method", ParamKind::choice(options, None)));
    fixture.settle();
    let select = fixture.by_label("Method").expect("the method row has a select");
    fixture.click(&select).settle();
    fixture.key(Key::ArrowDown, Modifiers::NONE).settle();
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert!(answered.borrow().is_none(), "Enter in the open select does not answer OK");
    assert!(fixture.by_label("Row").is_some(), "the dialog stays open");
    let ok = fixture.by_label("OK").expect("the footer has OK");
    fixture.click(&ok).settle();
    assert_eq!(answered.borrow().as_ref().and_then(|fields| fields.text("method").map(str::to_owned)).as_deref(), Some("zoom"), "the pick stood");
}

// A dialog that previews lists its Preview box as the `preview` field, and the field sets it; one that does not
// preview refuses the field as unknown.
#[trial]
fn a_previewing_dialog_takes_preview_as_a_field() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    let ctx = fixture.ctx().clone();
    let id = host.borrow_mut().open(HostedParamDialog::new(&ctx, "blur", DialogFields::new(), |fields| blur(fields).previews(true)));
    fixture.idle(4);
    let preview = |host: &Rc<RefCell<DialogHost>>| {
        host.borrow()
            .open_dialogs()
            .into_iter()
            .find(|dialog| dialog.id == id)
            .and_then(|dialog| dialog.fields)
            .map(|fields| fields["preview"].clone())
    };
    assert_eq!(preview(&host), Some(json!(true)), "the box lists as on");
    assert_eq!(host.borrow_mut().set_json_field(&ctx, id, "preview", json!(false)), Ok(()));
    assert_eq!(on_dialog(&host, |dialog| dialog.is_previewing()), Some(false), "the field turned the preview off");
    assert_eq!(preview(&host), Some(json!(false)));

    let plain = host.borrow_mut().open(HostedParamDialog::new(&ctx, "sharpen", DialogFields::new(), sharpen));
    fixture.idle(4);
    let refused = host.borrow_mut().set_json_field(&ctx, plain, "preview", json!(true));
    assert!(refused.is_err(), "a dialog that does not preview has no preview field: {refused:?}");
}

// A refusal the host keeps turns OK off with its reason and survives a changed value until the host withdraws it.
#[trial]
fn a_host_refusal_keeps_ok_off_until_a_value_changes() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "blur", DialogFields::new(), blur));
    fixture.idle(6);
    let ok_off = |fixture: &Fixture| {
        let ok = fixture.by_label("OK").expect("the OK button");
        fixture.tree().is_disabled_key(ok.node)
    };
    on_dialog(&host, |dialog| dialog.refuse(Some(Label::from(REFUSED))));
    fixture.idle(4);
    assert!(ok_off(&fixture), "the refusal turns OK off");
    assert!(fixture.by_label(REFUSED).is_some(), "the reason shows under the rows");
    assert_eq!(host.borrow().held_state(dialog_id("blur")), Some(HeldOk::Refused(Label::from(REFUSED))));

    let smooth = fixture.by_label("Smooth").expect("the smooth row has a check box");
    fixture.click(&smooth).idle(4);
    assert!(ok_off(&fixture), "a changed value does not lift the refusal the host keeps");
    assert!(fixture.by_label(REFUSED).is_some(), "and the reason stays");

    on_dialog(&host, |dialog| dialog.refuse(None));
    fixture.idle(4);
    assert!(!ok_off(&fixture), "the host withdrew the refusal");
    assert!(fixture.by_label(REFUSED).is_none());
}

/// A dialog whose Show Grid row stands in the footer.
fn gridded(fields: &mut DialogFields) -> ParamDialog<'_> {
    ParamDialog::new("grid", "Grid", footer_params()).values(Bind::bound(fields))
}

/// The rows of [`gridded`].
fn footer_params() -> Vec<Param> {
    vec![
        Param::new("radius", "Radius", ParamKind::range(0.0, 100.0, 10.0, Default::default())),
        Param::new("grid", "Show Grid", ParamKind::bool(false)).in_footer("Hide Grid"),
    ]
}

// A footer row is a footer button: a press flips its value and the dialog stays open, its label follows, automation
// sets the value, the remembered values leave it out, and the rows draw no body row for it.
#[trial]
fn a_footer_row_flips_its_value_and_automation_sets_it() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    let ctx = fixture.ctx().clone();
    let id = host.borrow_mut().open(HostedParamDialog::new(&ctx, "grid", DialogFields::new(), gridded));
    fixture.idle(6);
    let grid = || on_dialog(&host, |dialog| dialog.fields().bool("grid")).flatten();
    assert_eq!(grid(), Some(false));
    assert!(!fixture.by_role(Role::CHECK_BOX).iter().any(|target| fixture.tree().name_of(target.node) == Some("Show Grid")), "no body row");

    let button = fixture.by_label("Show Grid").expect("the footer has the row's button");
    fixture.click(&button).idle(6);
    assert_eq!(grid(), Some(true), "the press flips the value");
    assert!(host.borrow().is_open::<HostedParamDialog>(), "the dialog stays open");
    assert!(fixture.by_label("Hide Grid").is_some(), "the label follows the value");

    assert_eq!(host.borrow_mut().set_json_field(&ctx, id, "grid", json!(false)), Ok(()));
    assert_eq!(grid(), Some(false), "automation sets it");
    let fields = on_dialog(&host, |dialog| dialog.fields().clone()).expect("open");
    assert!(!ParamDialog::remembered(&footer_params(), &fields).contains("grid"), "the footer key is not remembered");
}

// The presets row lists the presets; a pick writes the preset's values as one edit, and a value edited after it
// leaves the values matching no preset.
#[trial]
fn a_preset_row_fills_the_values_and_names_new_presets() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    let ctx = fixture.ctx().clone();
    let id = host.borrow_mut().open(HostedParamDialog::new(&ctx, "blur", DialogFields::new(), blur));
    let rows = vec![PresetRow::item("soft".to_owned(), "Soft", 0), PresetRow::item("hard".to_owned(), "Hard", 0)];
    let values =
        vec![("soft".to_owned(), DialogFields::from_pairs([("radius", 2.0)])), ("hard".to_owned(), DialogFields::from_pairs([("radius", 60.0)]))];
    on_dialog(&host, |dialog| dialog.set_presets(rows, values, vec!["soft".to_owned()]));
    fixture.idle(6);
    let preset = |host: &Rc<RefCell<DialogHost>>| {
        host.borrow()
            .open_dialogs()
            .into_iter()
            .find(|dialog| dialog.id == id)
            .and_then(|dialog| dialog.fields)
            .map(|fields| fields["preset"].clone())
    };
    let picker = fixture.by_label("Preset").expect("the dialog has a presets row");
    fixture.click(&picker).idle(4);
    let hard = fixture.by_label("Hard").expect("the picker lists the preset");
    fixture.click(&hard).idle(6);
    assert_eq!(on_dialog(&host, |dialog| dialog.fields().number("radius")).flatten(), Some(60.0), "the pick wrote the preset's values");
    assert_eq!(preset(&host), Some(json!("hard")), "the values are the preset's");

    assert_eq!(host.borrow_mut().set_json_field(&ctx, id, "radius", json!(5)), Ok(()));
    fixture.idle(4);
    assert_eq!(preset(&host), Some(json!(null)), "after an edit the values are no preset's");
    assert!(on_dialog(&host, HostedParamDialog::take_preset_events).is_some_and(|events| events.is_empty()), "a pick reports no edit of the presets");
}

/// A Blur dialog that keeps its values for the run.
fn run_only(fields: &mut DialogFields) -> ParamDialog<'_> {
    blur(fields).remember(SaveKey::new("trial.run-only")).remember_for(Lifetime::Run)
}

// A dialog that remembers for the run reopens with the values the app stored after its command, and nothing reaches
// the saved tier.
#[trial]
fn a_run_only_remember_reopens_with_its_values_and_stores_nothing() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    let ctx = fixture.ctx().clone();
    let id = host.borrow_mut().open(HostedParamDialog::new(&ctx, "blur", DialogFields::new(), run_only));
    fixture.idle(4);
    host.borrow_mut().set_json_field(&ctx, id, "radius", json!(33.0)).expect("the radius row takes a number");
    assert!(host.borrow_mut().confirm::<HostedParamDialog>(&ctx), "OK answers");
    let result = host.borrow_mut().take_keyed::<HostedParamDialog>().map(|(result, _)| result).and_then(DialogResult::ok).expect("the answer");
    result.remember(&ctx);
    fixture.idle(2);

    host.borrow_mut().open(HostedParamDialog::new(&ctx, "blur", DialogFields::new(), run_only));
    fixture.idle(4);
    assert_eq!(on_dialog(&host, |dialog| dialog.fields().number("radius")).flatten(), Some(33.0), "the run's values come back");
    assert_eq!(saved_value::<Option<DialogFields>>(&ctx, &SaveKey::new("trial.run-only")), None, "a run-only value never reaches the saved tier");
}

// A host's reset restores the opening values with `reset_to` over them, keeps the dialog open and answers false for an
// unknown id. Whether it can reset compares with that target: values that opened off it can be reset, and none at it.
#[trial]
fn a_hosted_dialogs_reset_through_the_host_restores_the_opening_values() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    let ctx = fixture.ctx().clone();
    let id = host.borrow_mut().open(HostedParamDialog::new(&ctx, "blur", DialogFields::new(), |fields| {
        blur(fields).reset_to(DialogFields::from_pairs([("smooth", true)]))
    }));
    fixture.idle(6);
    assert!(host.borrow().can_reset_by_id(id), "Smooth opened off its reset target");
    host.borrow_mut().set_json_field(&ctx, id, "radius", json!(60.0)).expect("the radius row takes a number");
    assert!(host.borrow().can_reset_by_id(id), "an edit can be reset");
    assert!(host.borrow_mut().reset_by_id(&ctx, id), "the reset happened");
    fixture.idle(4);
    let fields = on_dialog(&host, |dialog| dialog.fields().clone()).expect("the dialog stays open");
    assert_eq!((fields.number("radius"), fields.bool("smooth")), (Some(10.0), Some(true)), "the opening values with reset_to over them");
    assert!(!host.borrow().can_reset_by_id(id), "at the reset target there is nothing to reset");
    assert!(!host.borrow_mut().reset_by_id(&ctx, dialog_id("unknown")), "an unknown id answers false");
}

/// Escape with the focus in a form's text field, plain or masked (a password), cancels the dialog at once, typed text or
/// not, as Escape does from any other control of the dialog: the dialog's Cancel discards the field's draft with the
/// rest, so the draft does not hold Escape (production closed its Protect dialog on one Escape after typing).
#[trial]
fn escape_in_a_text_field_closes_the_form_dialog_typed_or_not() {
    for masked in [false, true] {
        for typed in ["", "secret"] {
            let open = Rc::new(Cell::new(true));
            let closed = Rc::new(Cell::new(false));
            let (shown, seen, value) = (Rc::clone(&open), Rc::clone(&closed), Rc::new(RefCell::new(String::new())));
            let mut fixture = Fixture::new(move |ui| {
                let mut is_open = shown.get();
                let mut text = value.borrow_mut();
                let out = craft_ui::patterns::form_dialogs::FormDialog::new("Protect", Bind::bound(&mut *text))
                    .opened(Bind::bound(&mut is_open))
                    .show(ui, |ui, _, text| {
                        let field = craft_ui::text::TextField::new("Type Password", Bind::bound(text));
                        let _ = if masked { field.masked() } else { field }.show(ui);
                        true
                    });
                shown.set(is_open);
                seen.set(seen.get() || out.closed.is_some());
            });
            fixture.idle(10);
            let field = fixture.by_label("Type Password").expect("the field is shown");
            fixture.click(&field).idle(2);
            fixture.type_text(typed).idle(2);
            fixture.key(Key::Escape, Modifiers::NONE).idle(6);
            assert!(closed.get() && !open.get(), "Escape in the field closes the dialog (masked {masked}, typed {typed:?})");
        }
    }
}

/// A modeless form keeps its other edits when Escape cancels an active text edit.
#[trial]
fn escape_in_a_modeless_form_restores_the_field_before_closing() {
    let open = Rc::new(Cell::new(true));
    let value = Rc::new(RefCell::new(String::from("Original")));
    let (shown, text) = (Rc::clone(&open), Rc::clone(&value));
    let mut fixture = Fixture::new(move |ui| {
        let mut is_open = shown.get();
        let mut text = text.borrow_mut();
        craft_ui::patterns::form_dialogs::FormDialog::new("Tool", Bind::bound(&mut *text))
            .opened(Bind::bound(&mut is_open))
            .shaped(|dialog| dialog.modeless())
            .show(ui, |ui, _, text| {
                craft_ui::text::TextField::new("Name", Bind::bound(text)).show(ui);
                true
            });
        shown.set(is_open);
    });
    fixture.idle(10);
    let field = fixture.by_label("Name").expect("the field is shown");
    fixture.click(&field).idle(2);
    fixture.type_text("x").idle(2);
    assert_ne!(&*value.borrow(), "Original", "typing changed the value");
    fixture.key(Key::Escape, Modifiers::NONE).idle(2);
    assert!(open.get(), "the first Escape leaves the window open");
    assert_eq!(&*value.borrow(), "Original", "the field's edit is restored");
    fixture.key(Key::Escape, Modifiers::NONE).idle(6);
    assert!(!open.get(), "the second Escape closes the window");
}
