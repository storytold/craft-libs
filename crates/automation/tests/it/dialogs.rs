//! `ui.dialog.*` on typed dialogs through one generic path.

use craft_automation::dialogs::{dialog_method, inspect};
use craft_ui::contract::Bind;
use craft_ui::dialog::{DialogHost, DialogResult, PromptChoice, PromptKind, StandardPrompt};
use craft_ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog};
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

    host.open(StandardPrompt::new(PromptKind::TrashMask));
    let prompt = inspect(&host)[0]["id"].clone();
    assert_eq!(call(&mut host, &ctx, "ui.dialog.cancel", json!({"dialog": prompt})), Ok(Value::Null));
    assert!(matches!(host.take::<StandardPrompt>(), Some(DialogResult::Cancel)));
    assert!(call(&mut host, &ctx, "ui.dialog.cancel", json!({"dialog": prompt})).is_err());
}
