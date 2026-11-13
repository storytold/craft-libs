//! `ui.dialog.*` on typed dialogs through one generic path.

use craft_automation::dialogs::{dialog_method, inspect, inspect_stack, names_open_dialog};
use craft_ui::contract::{Bind, SaveKey};
use craft_ui::dialog::{DialogHost, DialogResult, PromptChoice, PromptKind, StandardPrompt, StartupStage, dialog_id};
use craft_ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft_ui::testing::{Fixture, trial};
use egui::Context;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

fn call(host: &mut DialogHost, ctx: &Context, method: &str, params: Value) -> Result<Value, String> {
    dialog_method(host, ctx, method, &params).expect("a ui.dialog method")
}

/// A parameter dialog and a prompt are listed with their fields, set by key and answered by id, with no code that
/// knows their types; a value the row can't hold and an unknown choice are refused.
#[trial]
fn dialog_methods_read_set_and_answer_typed_dialogs() {
    let ctx = Context::default();
    let mut host = DialogHost::new();
    host.open(HostedParamDialog::new("gaussianBlur", DialogFields::from_pairs([("radius", 1.0)]), |fields| {
        ParamDialog::new("gaussianBlur", "Gaussian Blur", Param::parse_list(r#"{"radius":0.1..1000=1,"method":"spin|zoom"}"#))
            .values(Bind::bound(fields))
    }));
    let listed = inspect(&host);
    assert_eq!(listed[0]["key"], "gaussianBlur");
    assert_eq!(listed[0]["fields"], json!({"radius": 1.0}));
    let blur = listed[0]["id"].clone();

    assert_eq!(call(&mut host, &ctx, "ui.dialog.set", json!({"dialog": blur, "field": "radius", "value": 4.5})), Ok(Value::Null));
    assert!(call(&mut host, &ctx, "ui.dialog.set", json!({"dialog": blur, "field": "radius", "value": 5000})).is_err_and(|e| e.contains("radius")));
    assert!(
        call(&mut host, &ctx, "ui.dialog.set", json!({"dialog": blur, "field": "angle", "value": 1})).is_err_and(|e| e.contains("unknown field"))
    );
    assert_eq!(inspect(&host)[0]["fields"]["radius"], 4.5);
    assert_eq!(call(&mut host, &ctx, "ui.dialog.confirm", json!({"dialog": blur})), Ok(Value::Null));
    let Some(DialogResult::Ok(result)) = host.take::<HostedParamDialog>() else { panic!("the blur dialog answers OK") };
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
    // An app with dialogs of its own beside the host's learns which ones a request names.
    assert!(names_open_dialog(&host, &json!({"dialog": delete})) && names_open_dialog(&host, &json!({"dialog": delete.to_string()})));
    assert!(!names_open_dialog(&host, &json!({"dialog": 7})) && !names_open_dialog(&host, &json!({"dialog": "shell"})));
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
    assert!(matches!(host.take::<StandardPrompt>(), Some(DialogResult::Cancel)));
    assert!(call(&mut host, &ctx, "ui.dialog.cancel", json!({"dialog": prompt})).is_err());
}

const REMEMBERED: SaveKey = SaveKey::new("craft_automation.blur");

/// A hosted parameter dialog's automation follows its rows as a user meets them: OK is refused while disabled, a
/// disabled row takes no value, a value is read as its row's kind (three integers for a three-cell grid), and OK
/// before the first pass answers with the remembered values the first pass would have loaded.
#[trial]
fn param_dialog_automation_follows_its_rows() {
    let ctx = Context::default();
    let mut host = DialogHost::new();
    let blur = |ok: bool| {
        HostedParamDialog::new("blur", DialogFields::new(), move |fields| {
            let mut params = Param::parse_list(r#"{"radius":0.1..1000=1,"method":"spin|zoom"}"#);
            params.push(Param::new("amount", "Amount", ParamKind::range(0.0, 100.0, 10.0, Default::default())).enabled_when("method", "zoom"));
            params.push(Param::new("cells", "Cells", ParamKind::grid(3, 0)));
            ParamDialog::new("blur", "Blur", params).values(Bind::bound(fields)).remember(REMEMBERED).ok_enabled(ok)
        })
    };

    host.open(blur(false));
    let id = inspect(&host)[0]["id"].clone();
    assert!(call(&mut host, &ctx, "ui.dialog.confirm", json!({"dialog": id})).is_err());
    assert!(host.take::<HostedParamDialog>().is_none());
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
    let Some(DialogResult::Ok(result)) = host.take::<HostedParamDialog>() else { panic!("the blur dialog answers OK") };
    assert_eq!(result.fields.grid("cells"), Some(&[1, 2, 3][..]));
    ctx.run_ui(Default::default(), |ui| result.remember(ui)).textures_delta.clear();

    host.open(blur(true));
    let id = inspect(&host)[0]["id"].clone();
    assert_eq!(call(&mut host, &ctx, "ui.dialog.confirm", json!({"dialog": id})), Ok(Value::Null));
    let Some(DialogResult::Ok(reopened)) = host.take::<HostedParamDialog>() else { panic!("the reopened dialog answers OK") };
    assert_eq!((reopened.fields.number("radius"), reopened.fields.text("method")), (Some(7.0), Some("zoom")));
}

/// Dialogs an app shows itself take their place on the host's stack when they open: listed in one opening order
/// between the hosted ones, never answered by the host's methods, and closing the dialogs opened over them, the
/// host's with `Cancel`, when they close.
#[trial]
fn beside_dialogs_share_the_hosts_stack() {
    let ctx = Context::default();
    let mut host = DialogHost::new();
    let (first, second, third) = (dialog_id(("app", 1)), dialog_id(("app", 2)), dialog_id(("app", 3)));
    let own = |id| Some(json!({"id": if id == first { "first" } else { "second" }}));
    host.open_beside(first, None);
    let prompt = host.open(StandardPrompt::new(PromptKind::TrashMask));
    host.open_beside(second, None);
    let listed = inspect_stack(&host, own);
    assert_eq!((listed[0]["id"].clone(), listed[2]["id"].clone()), (json!("first"), json!("second")));
    assert_eq!(listed[1]["id"], prompt.value().to_string());
    assert!(host.stack().is_top(second));
    assert!(!names_open_dialog(&host, &json!({"dialog": first.value()})));
    assert!(call(&mut host, &ctx, "ui.dialog.cancel", json!({"dialog": second.value()})).is_err());

    host.close_beside(second);
    assert!(host.stack().is_top(prompt), "an app dialog that closed leaves the stack");
    assert!(host.cancel_by_id(&ctx, prompt));
    host.take::<StandardPrompt>();
    let child = host.open_child(StandardPrompt::confirm_delete(["Layer 1"], 1), first);
    host.open_beside(third, Some(child));
    assert!(host.stack().is_top(third));
    host.close_beside(first);
    assert!(matches!(host.take::<StandardPrompt>(), Some(DialogResult::Cancel)), "a child closes with its app dialog");
    assert!(host.stack().is_empty() && host.is_empty(), "the app dialog over the child leaves with it");
}

/// A startup modal waits while a dialog the app shows itself is open, and opens once it closes.
#[trial]
fn the_startup_queue_waits_for_beside_dialogs() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let own = dialog_id(("app", 1));
    host.borrow_mut().open_beside(own, None);
    host.borrow_mut().enqueue(StartupStage::Warning, StandardPrompt::new(PromptKind::TrashMask));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.idle(2);
    assert!(!host.borrow().is_open::<StandardPrompt>(), "the startup modal waits for the app's dialog");
    host.borrow_mut().close_beside(own);
    fixture.idle(2);
    assert!(host.borrow().is_open::<StandardPrompt>(), "the startup modal opens once the app's dialog closed");
}
