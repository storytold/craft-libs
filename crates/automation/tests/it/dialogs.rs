//! `ui.dialog.*` on typed dialogs through one generic path.

use std::sync::mpsc::{Receiver, channel};

use craft_automation::control::{ControlChannel, ControlRequest, ControlResponse};
use craft_automation::dialogs::{MAX_DIALOG_FIELDS, dialog_method, dialog_reply, inspect};
use craft_ui::contract::{Bind, SaveKey};
use craft_ui::dialog::{DialogHost, DialogId, DialogResult, PromptChoice, PromptKind, StandardPrompt};
use craft_ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft_ui::runtime::env::{Label, Word};
use craft_ui::testing::trial;
use egui::Context;
use serde_json::{Value, json};

fn call(host: &mut DialogHost, ctx: &Context, method: &str, params: Value) -> Result<Value, String> {
    dialog_method(host, ctx, method, &params).expect("a ui.dialog method")
}

/// A parameter dialog and a prompt are listed with their fields, set by key and answered by id, with no code that
/// knows their types; a value the row can't hold and an unknown choice are refused.
#[trial]
fn dialog_methods_read_set_and_answer_typed_dialogs() {
    let ctx = Context::default();
    let mut host = DialogHost::new();
    host.open(HostedParamDialog::new(&ctx, "gaussianBlur", DialogFields::from_pairs([("radius", 1.0)]), |fields| {
        ParamDialog::new("gaussianBlur", "Gaussian Blur", Param::parse_list(r#"{"radius":0.1..1000=1,"method":"spin|zoom"}"#))
            .values(Bind::bound(fields))
    }));
    let listed = inspect(&host);
    assert_eq!(listed[0]["key"], "gaussianBlur");
    assert_eq!(listed[0]["fields"], json!({"radius": 1.0, "method": "spin"}), "the dialog lists its defaults from its open");
    let blur = listed[0]["id"].clone();

    assert_eq!(call(&mut host, &ctx, "ui.dialog.set", json!({"dialog": blur, "field": "radius", "value": 4.5})), Ok(Value::Null));
    assert!(call(&mut host, &ctx, "ui.dialog.set", json!({"dialog": blur, "field": "radius", "value": 5000})).is_err_and(|e| e.contains("radius")));
    assert!(
        call(&mut host, &ctx, "ui.dialog.set", json!({"dialog": blur, "field": "angle", "value": 1})).is_err_and(|e| e.contains("unknown field"))
    );
    assert_eq!(inspect(&host)[0]["fields"]["radius"], 4.5);
    assert_eq!(call(&mut host, &ctx, "ui.dialog.confirm", json!({"dialog": blur})), Ok(Value::Null));
    let Some(DialogResult::Ok(result)) = host.take_keyed::<HostedParamDialog>().map(|(result, _)| result) else {
        panic!("the blur dialog answers OK")
    };
    assert_eq!((result.fields.number("radius"), result.fields.text("method")), (Some(4.5), Some("spin")));
    assert_eq!(inspect(&host), json!([]));

    host.open(StandardPrompt::new(PromptKind::TrashMask));
    let prompt = inspect(&host)[0]["id"].clone();
    assert_eq!(inspect(&host)[0]["fields"]["choice"], "proceed");
    assert!(call(&mut host, &ctx, "ui.dialog.confirm", json!({"dialog": prompt, "choice": "maybe"})).is_err());
    assert_eq!(call(&mut host, &ctx, "ui.dialog.confirm", json!({"dialog": prompt, "choice": "alternate"})), Ok(Value::Null));
    let Some(DialogResult::Ok(answer)) = host.take::<StandardPrompt>() else { panic!("the prompt answers") };
    assert_eq!(answer.choice, PromptChoice::Alternate);

    // Only answers with an enabled button: a delete question has no Alternate, and Apply is off while the mask
    // can't be applied, so the default answer is refused too. The id may also come as a number.
    host.open(StandardPrompt::confirm_delete(["Layer 1"], 1));
    let delete: u64 = inspect(&host)[0]["id"].as_str().and_then(|id| id.parse().ok()).expect("a decimal id");
    assert!(call(&mut host, &ctx, "ui.dialog.set", json!({"dialog": delete, "field": "choice", "value": "alternate"})).is_err());
    assert!(host.cancel::<StandardPrompt>(&ctx));
    host.take::<StandardPrompt>();
    host.open(StandardPrompt::new(PromptKind::TrashMask).can_apply(false));
    let trash = inspect(&host)[0]["id"].clone();
    assert!(call(&mut host, &ctx, "ui.dialog.confirm", json!({"dialog": trash})).is_err());
    assert!(call(&mut host, &ctx, "ui.dialog.confirm", json!({"dialog": trash, "choice": "proceed"})).is_err());
    assert!(host.take::<StandardPrompt>().is_none());
    assert!(host.cancel::<StandardPrompt>(&ctx));
    host.take::<StandardPrompt>();

    host.open(StandardPrompt::new(PromptKind::TrashMask));
    let prompt = inspect(&host)[0]["id"].clone();
    assert_eq!(call(&mut host, &ctx, "ui.dialog.cancel", json!({"dialog": prompt})), Ok(Value::Null));
    let Some(DialogResult::Ok(answer)) = host.take::<StandardPrompt>() else { panic!("the prompt answers its Cancel, as Escape does") };
    assert_eq!(answer.choice, PromptChoice::Cancel);
    assert!(call(&mut host, &ctx, "ui.dialog.cancel", json!({"dialog": prompt})).is_err());
}

const REMEMBERED: SaveKey = SaveKey::new("craft_automation.blur");

/// A hosted parameter dialog's automation follows its rows as a user meets them: OK is refused while disabled, a
/// disabled row takes no value, a value is read as its row's kind (three integers for a three-cell grid), and from its
/// open, before any pass, the dialog lists and answers with the remembered values over the defaults.
#[trial]
fn param_dialog_automation_follows_its_rows() {
    let ctx = Context::default();
    let mut host = DialogHost::new();
    let blur = |ok: bool| {
        HostedParamDialog::new(&ctx, "blur", DialogFields::new(), move |fields| {
            let mut params = Param::parse_list(r#"{"radius":0.1..1000=1,"method":"spin|zoom"}"#);
            params.push(Param::new("amount", "Amount", ParamKind::range(0.0, 100.0, 10.0, Default::default())).enabled_when("method", "zoom"));
            params.push(Param::new("cells", "Cells", ParamKind::grid(3, 0)));
            ParamDialog::new("blur", "Blur", params).values(Bind::bound(fields)).remember(REMEMBERED).ok_enabled(ok)
        })
    };

    host.open(blur(false));
    let id = inspect(&host)[0]["id"].clone();
    assert!(call(&mut host, &ctx, "ui.dialog.confirm", json!({"dialog": id})).is_err());
    assert!(host.take_keyed::<HostedParamDialog>().map(|(result, _)| result).is_none());
    assert!(host.cancel::<HostedParamDialog>(&ctx));

    host.open(blur(true));
    let id = inspect(&host)[0]["id"].clone();
    let set =
        |host: &mut DialogHost, field: &str, value: Value| call(host, &ctx, "ui.dialog.set", json!({"dialog": id, "field": field, "value": value}));
    assert!(set(&mut host, "amount", json!(50)).is_err(), "Amount is disabled while the method is spin");
    assert_eq!(set(&mut host, "method", json!("zoom")), Ok(Value::Null));
    assert_eq!(set(&mut host, "amount", json!(50)), Ok(Value::Null));
    assert_eq!(set(&mut host, "cells", json!([1, 2, 3])), Ok(Value::Null));
    assert_eq!(set(&mut host, "radius", json!(7)), Ok(Value::Null));
    assert_eq!(inspect(&host)[0]["fields"]["cells"], json!([1, 2, 3]));
    assert_eq!(call(&mut host, &ctx, "ui.dialog.confirm", json!({"dialog": id})), Ok(Value::Null));
    let Some(DialogResult::Ok(result)) = host.take_keyed::<HostedParamDialog>().map(|(result, _)| result) else {
        panic!("the blur dialog answers OK")
    };
    assert_eq!(result.fields.grid("cells"), Some(&[1, 2, 3][..]));
    result.remember(&ctx);

    host.open(blur(true));
    let listed = inspect(&host);
    let id = listed[0]["id"].clone();
    let fields = &listed[0]["fields"];
    assert_eq!((fields["radius"].as_f64(), &fields["method"], &fields["cells"]), (Some(7.0), &json!("zoom"), &json!([0, 1, 0])), "{fields}");
    assert_eq!(call(&mut host, &ctx, "ui.dialog.confirm", json!({"dialog": id})), Ok(Value::Null));
    let Some(DialogResult::Ok(reopened)) = host.take_keyed::<HostedParamDialog>().map(|(result, _)| result) else {
        panic!("the reopened dialog answers OK")
    };
    assert_eq!((reopened.fields.number("radius"), reopened.fields.text("method")), (Some(7.0), Some("zoom")));
}

const TAKEN: Word = Word::keyed("craft_automation.nameTaken", "A preset with that name exists");

/// The open parameter dialog `id` names.
fn held(host: &mut DialogHost, id: DialogId) -> &mut HostedParamDialog {
    host.get_dyn_mut(id).and_then(|dialog| dialog.downcast_mut::<HostedParamDialog>()).expect("the held dialog is open")
}

/// A `ui.dialog.confirm` that files a held OK replies with the app's answer: nothing while the app has not answered,
/// an error with the refusal's English text once it refuses, success once it accepts, and an error when the dialog
/// closes first, also when it opened again since. A second confirm while the answer waits is refused, and one after a
/// refusal no poll has read answers the earlier confirm with it. A dialog without a held OK replies at once.
#[trial]
fn a_confirm_of_a_held_ok_replies_with_the_apps_answer() {
    let ctx = Context::default();
    let mut host = DialogHost::new();
    let (_requests, rx) = channel();
    let mut control = ControlChannel::new(rx);
    let dialog = |ctx: &Context, held: bool| {
        let dialog = HostedParamDialog::new(ctx, "blur", DialogFields::new(), |fields| {
            ParamDialog::new("blur", "Blur", Param::parse_list(r#"{"radius":0.1..1000=1}"#)).values(Bind::bound(fields))
        });
        if held { dialog.held_ok() } else { dialog }
    };
    let confirm = |host: &mut DialogHost, control: &mut ControlChannel, id: DialogId| -> Receiver<ControlResponse> {
        let (request, reply) = ControlRequest::new("ui.dialog.confirm", json!({"dialog": id.value()}));
        let outcome = dialog_reply(host, &ctx, &request.method, &request.params).expect("a ui.dialog method");
        control.settle(request, outcome, None, 0.0);
        reply
    };

    let id = host.open(dialog(&ctx, true));
    let reply = confirm(&mut host, &mut control, id);
    control.poll_held(&host);
    assert!(reply.try_recv().is_err(), "the reply waits for the app's answer");
    assert_eq!(control.waiting(), 1);
    let again = confirm(&mut host, &mut control, id);
    assert!(again.try_recv().is_ok_and(|reply| reply["ok"] == false), "a second OK while the first waits is refused");
    assert_eq!(control.waiting(), 1);
    assert!(held(&mut host, id).take_ok().is_some(), "OK filed the answer for the app");
    held(&mut host, id).answer_ok(Err(Label::from(TAKEN)));
    control.poll_held(&host);
    assert_eq!(reply.try_recv(), Ok(json!({"ok": false, "error": "A preset with that name exists"})));
    assert_eq!(control.waiting(), 0);

    // A confirm after a refusal no poll has read yet answers the earlier confirm with that refusal.
    let first = confirm(&mut host, &mut control, id);
    assert!(held(&mut host, id).take_ok().is_some(), "OK files the answer again after a refusal");
    held(&mut host, id).answer_ok(Err(Label::from(TAKEN)));
    let reply = confirm(&mut host, &mut control, id);
    assert_eq!(first.try_recv(), Ok(json!({"ok": false, "error": "A preset with that name exists"})));
    assert_eq!(control.waiting(), 1);
    assert!(held(&mut host, id).take_ok().is_some());
    held(&mut host, id).answer_ok(Ok(()));
    control.poll_held(&host);
    assert_eq!(reply.try_recv(), Ok(json!({"ok": true, "result": null})));
    assert!(host.confirm_by_id(&ctx, id), "the accepted dialog closes");
    assert!(matches!(host.take_keyed::<HostedParamDialog>(), Some((DialogResult::Ok(_), _))));

    let id = host.open(dialog(&ctx, true));
    let reply = confirm(&mut host, &mut control, id);
    assert!(host.cancel_by_id(&ctx, id));
    control.poll_held(&host);
    let closed = reply.try_recv().expect("a closed dialog answers the held confirm");
    assert_eq!(closed["ok"], false);
    assert!(closed["error"].as_str().is_some_and(|error| error.contains("closed")), "{closed}");
    host.take_keyed::<HostedParamDialog>();

    // The same dialog opened again is a new open: its answer is not the closed one's.
    let id = host.open(dialog(&ctx, true));
    let reply = confirm(&mut host, &mut control, id);
    assert!(host.cancel_by_id(&ctx, id));
    assert_eq!(host.open(dialog(&ctx, true)), id, "a reopened dialog has the same id");
    assert_eq!(control.waiting(), 1);
    control.poll_held(&host);
    let closed = reply.try_recv().expect("the closed open's confirm is answered");
    assert!(closed["error"].as_str().is_some_and(|error| error.contains("closed")), "{closed}");
    assert!(host.cancel_by_id(&ctx, id));
    host.take_keyed::<HostedParamDialog>();

    let id = host.open(dialog(&ctx, false));
    let reply = confirm(&mut host, &mut control, id);
    assert_eq!(reply.try_recv(), Ok(json!({"ok": true, "result": null})), "a dialog without a held OK replies at once");
    assert_eq!(control.waiting(), 0);
}

/// A `fields` list sets every field or none: a refused field in the middle leaves the dialog as it was and gives that
/// refusal; a list over the limit, an item without a string `field` and `field` beside `fields` are refused unread.
#[trial]
fn a_field_list_sets_every_field_or_none_and_has_a_limit() {
    let ctx = Context::default();
    let mut host = DialogHost::new();
    host.open(HostedParamDialog::new(&ctx, "blur", DialogFields::new(), |fields| {
        ParamDialog::new("blur", "Blur", Param::parse_list(r#"{"radius":0.1..1000=1,"method":"spin|zoom"}"#)).values(Bind::bound(fields))
    }));
    let id = inspect(&host)[0]["id"].clone();
    let fields = |host: &DialogHost| inspect(host)[0]["fields"].clone();
    let set = |host: &mut DialogHost, params: Value| call(host, &ctx, "ui.dialog.set", params);

    let listed = json!([{"field": "radius", "value": 4.5}, {"field": "method", "value": "zoom"}]);
    assert_eq!(set(&mut host, json!({"dialog": id, "fields": listed})), Ok(Value::Null));
    let settled = fields(&host);
    assert_eq!((settled["radius"].as_f64(), &settled["method"]), (Some(4.5), &json!("zoom")));

    let refused = json!([{"field": "radius", "value": 9}, {"field": "radius", "value": 5000}, {"field": "method", "value": "spin"}]);
    assert!(set(&mut host, json!({"dialog": id, "fields": refused})).is_err_and(|e| e.contains("radius")));
    assert_eq!(fields(&host), settled, "a refusal in the middle sets nothing");

    let too_many: Vec<Value> = (0..=MAX_DIALOG_FIELDS).map(|_| json!({"field": "radius", "value": 2})).collect();
    assert!(set(&mut host, json!({"dialog": id, "fields": too_many})).is_err());
    assert_eq!(fields(&host), settled, "a list over the limit sets nothing");

    let unnamed = json!([{"field": "radius", "value": 9}, {"value": 1}]);
    assert!(set(&mut host, json!({"dialog": id, "fields": unnamed})).is_err());
    assert_eq!(fields(&host), settled, "an item without a field sets nothing");

    let both = json!({"dialog": id, "field": "radius", "value": 9, "fields": [{"field": "radius", "value": 8}]});
    assert!(set(&mut host, both).is_err());
    assert_eq!(fields(&host), settled, "`field` beside `fields` sets nothing");
}

/// `"top"` names the top dialog of the stack: a child over its parent takes the field, the parent keeps its own; with
/// nothing open the request says so.
#[trial]
fn top_names_the_top_hosted_dialog() {
    let ctx = Context::default();
    let mut host = DialogHost::new();
    assert!(
        call(&mut host, &ctx, "ui.dialog.set", json!({"dialog": "top", "field": "choice", "value": "cancel"}))
            .is_err_and(|e| e.contains("no hosted dialog is open"))
    );
    let parent = host.open(StandardPrompt::new(PromptKind::TrashMask));
    host.open_child(StandardPrompt::confirm_delete(["Layer 1"], 1).instance("child"), parent);
    assert_eq!(call(&mut host, &ctx, "ui.dialog.set", json!({"dialog": "top", "field": "choice", "value": "cancel"})), Ok(Value::Null));
    let listed = inspect(&host);
    assert_eq!(listed[0]["id"], parent.value().to_string());
    assert_eq!((listed[0]["fields"]["choice"].clone(), listed[1]["fields"]["choice"].clone()), (json!("proceed"), json!("cancel")));
}

/// `ui.dialog.cancel` on a prompt with a Cancel button answers that Cancel as Escape does, with the prompt's tag; a
/// prompt without one closes from outside.
#[trial]
fn cancel_on_a_standard_prompt_answers_its_cancel_with_the_tag() {
    let ctx = Context::default();
    let mut host = DialogHost::new();
    let prompt = host.open(StandardPrompt::new(PromptKind::TrashMask).tag(7_u32));
    assert_eq!(call(&mut host, &ctx, "ui.dialog.cancel", json!({"dialog": prompt.value()})), Ok(Value::Null));
    let Some(DialogResult::Ok(answer)) = host.take::<StandardPrompt>() else { panic!("the prompt answers its Cancel") };
    assert_eq!((answer.choice, answer.tag::<u32>()), (PromptChoice::Cancel, Some(&7)));

    let prompt = host.open(StandardPrompt::custom("Rasterize the type layer?").button(PromptChoice::Proceed, "Rasterize").tag(8_u32));
    assert_eq!(call(&mut host, &ctx, "ui.dialog.cancel", json!({"dialog": prompt.value()})), Ok(Value::Null));
    assert!(matches!(host.take::<StandardPrompt>(), Some(DialogResult::Cancel)), "a prompt without Cancel closes from outside");
    assert!(host.is_empty());
}

/// The Actions library questions: Reset answers Append (the Alternate) with its tag; Clear All and Replace have no
/// Append, so a confirm choosing it is refused.
#[trial]
fn actions_library_prompts_offer_append_only_on_reset() {
    use craft_automation::ui::actions::{ActionsWords, LibraryPrompt};
    use craft_ui::runtime::words::CommonWords;
    let ctx = Context::default();
    let mut host = DialogHost::new();
    let (words, common) = (ActionsWords::default(), CommonWords::default());
    let reset = host.open(LibraryPrompt::Reset.prompt(&words, &common));
    assert_eq!(call(&mut host, &ctx, "ui.dialog.confirm", json!({"dialog": reset.value(), "choice": "alternate"})), Ok(Value::Null));
    let Some(DialogResult::Ok(answer)) = host.take::<StandardPrompt>() else { panic!("Reset answers Append") };
    assert_eq!((answer.choice, answer.tag::<LibraryPrompt>()), (PromptChoice::Alternate, Some(&LibraryPrompt::Reset)));

    for kind in [LibraryPrompt::ClearAll, LibraryPrompt::replace(3, "set.atn")] {
        let prompt = host.open(kind.prompt(&words, &common));
        assert!(call(&mut host, &ctx, "ui.dialog.confirm", json!({"dialog": prompt.value(), "choice": "alternate"})).is_err());
        assert_eq!(call(&mut host, &ctx, "ui.dialog.confirm", json!({"dialog": prompt.value(), "choice": "proceed"})), Ok(Value::Null));
        let Some(DialogResult::Ok(answer)) = host.take::<StandardPrompt>() else { panic!("OK answers Proceed") };
        assert_eq!((answer.choice, answer.tag::<LibraryPrompt>()), (PromptChoice::Proceed, Some(&kind)));
    }
}
