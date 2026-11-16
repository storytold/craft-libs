//! `ParamDialog` and `HostedParamDialog`: the dialog's own undo history, the Preview box's hold-to-compare, Enter in
//! a multi-line row and on a focused Cancel, Alt+Reset to `reset_to`, the held OK, reopening under the same key, the
//! request slots, disabled options and the remembered values a row refuses.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Outcome as _, Role, SaveKey, Show as _, clicked};
use craft_ui::dialog::{DialogHost, DialogResult, HeldOk, dialog_id};
use craft_ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft_ui::patterns::presets::PresetRow;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::testing::{Fixture, trial};
use egui::load::SizedTexture;
use egui::{Key, Modifiers, TextureId, Vec2, vec2};
use serde_json::json;
use std::borrow::Cow;
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
        let shown = ParamDialog::new("blur", "Blur", Param::parse_list(SPEC)).previews(true).preview(picture).show(ui);
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

/// A dialog with a files row and two footer buttons that keep it open.
fn stack_dialog(fields: &mut DialogFields) -> ParamDialog<'_> {
    ParamDialog::new("stack", "Load Files into Stack", vec![Param::files("sources", "Source Files").folder_mode(true)])
        .values(Bind::bound(fields))
        .extra_buttons(vec![("load".to_owned(), Label::from("Load…")), ("save".to_owned(), Label::from("Save…"))])
}

/// The `{files, folder, useFolder}` value of a files row that lists `folder`.
fn folder_value(folder: &str) -> serde_json::Value {
    json!({ "files": [], "folder": folder, "useFolder": true })
}

// A footer button that keeps the dialog open is reported once by its id, in a frame that runs twice too; a folder
// listing is asked for once; `cancel_requests` drops both.
#[trial]
fn a_hosted_dialogs_new_requests_come_once_and_a_cancel_drops_them() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let mut fixture = host_fixture(&host, &twice);
    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "stack", DialogFields::new(), stack_dialog));
    fixture.idle(10);

    let save = fixture.by_label("Save…").expect("the Save… button");
    twice.set(true);
    fixture.click(&save);
    twice.set(false);
    fixture.idle(4);
    assert_eq!(on_dialog(&host, HostedParamDialog::take_footer_action).flatten().as_deref(), Some("save"), "the press is reported by its id");
    assert_eq!(on_dialog(&host, HostedParamDialog::take_footer_action).flatten(), None, "the press is reported once");
    assert!(host.borrow().is_open::<HostedParamDialog>(), "the button keeps the dialog open");

    let ctx = fixture.ctx().clone();
    host.borrow_mut().set_json_field(&ctx, dialog_id("stack"), "sources", folder_value("/photos/shoot")).expect("the files row takes a folder");
    fixture.idle(4);
    let listing = on_dialog(&host, HostedParamDialog::take_listing).flatten().expect("a new folder asks for its files");
    assert_eq!((listing.key.as_str(), listing.folder), ("sources", PathBuf::from("/photos/shoot")));
    assert_eq!(on_dialog(&host, HostedParamDialog::take_listing).flatten(), None, "the listing is asked for once");

    let load = fixture.by_label("Load…").expect("the Load… button");
    fixture.click(&load).idle(2);
    host.borrow_mut().set_json_field(&ctx, dialog_id("stack"), "sources", folder_value("/photos/other")).expect("the files row takes a folder");
    fixture.idle(4);
    on_dialog(&host, HostedParamDialog::cancel_requests);
    assert_eq!(on_dialog(&host, HostedParamDialog::take_footer_action).flatten(), None, "a cancel drops the footer press");
    assert_eq!(on_dialog(&host, HostedParamDialog::take_listing).flatten(), None, "a cancel drops the listing request");
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
    let ids: &[&str] = if narrow { &["dots"] } else { &["dots", "stripes"] };
    let patterns: Vec<PresetRow<String>> = ids.iter().map(|id| PresetRow::item((*id).to_owned(), *id, 0)).collect();
    ParamDialog::new("filter", "Filter", filter_params(narrow))
        .values(Bind::bound(fields))
        .remember(SaveKey::new("trial.filter"))
        .patterns(Cow::Owned(patterns))
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
    on_dialog(&host, |dialog| dialog.set_preview(Some(picture)));
    fixture.idle(4);
    press(&mut fixture);
    assert_eq!(presses.get(), 1, "with the pane shown the press reaches the canvas");

    host.borrow_mut().open(HostedParamDialog::new(fixture.ctx(), "blur", DialogFields::new(), locating(id, false)));
    on_dialog(&host, |dialog| dialog.set_preview(Some(picture)));
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
