//! The profile prompt's automation fields follow its question.

use craft_color_ui::ui::{ProfileAction, ProfileAnswer, ProfileMismatchDialog, ProfileMode, ProfileQuestion};
use craft_ui::dialog::{DialogHost, DialogId, DialogResult};
use craft_ui::testing::trial;
use egui::Context;
use serde_json::{Value, json};

/// The fields of the open dialog `id`, as `ui.inspect` lists them.
fn fields(host: &DialogHost, id: DialogId) -> Value {
    host.open_dialogs().into_iter().find(|dialog| dialog.id == id).and_then(|dialog| dialog.fields).unwrap_or(Value::Null)
}

/// A mismatch takes its three actions and, only when rememberable, `remember`; a missing profile has no `discard`;
/// OK answers the action and the box as set.
// craft-color-ui has no craft-automation dev-dependency (color/Cargo.toml is dialogs-w6's), so the trial drives the
// by-id host calls `dialog_method` makes for `ui.dialog.set` and `ui.dialog.confirm`.
#[trial]
fn profile_prompt_fields_set_and_confirm() {
    let ctx = Context::default();
    let mut host = DialogHost::new();

    let id = host.open(ProfileMismatchDialog::new("poster.psd", ProfileMode::Rgb, ProfileQuestion::mismatch("Adobe RGB (1998)", "sRGB")));
    assert_eq!(fields(&host, id), json!({"action": "useEmbedded"}), "no `remember` without rememberable");
    assert!(host.set_json_field(&ctx, id, "remember", json!(true)).is_err_and(|e| e.contains("unknown field")));
    for action in ["useEmbedded", "convertToWorking", "discard"] {
        assert_eq!(host.set_json_field(&ctx, id, "action", json!(action)), Ok(()), "a mismatch takes {action}");
    }
    assert!(host.set_json_field(&ctx, id, "action", json!("leaveAsIs")).is_err_and(|e| e.contains("can't take")));
    assert!(host.cancel_by_id(&ctx, id));
    host.take::<ProfileMismatchDialog>();

    let dialog = ProfileMismatchDialog::new("poster.psd", ProfileMode::Rgb, ProfileQuestion::mismatch("Adobe RGB (1998)", "sRGB")).rememberable(true);
    let id = host.open(dialog);
    assert_eq!(fields(&host, id), json!({"action": "useEmbedded", "remember": false}));
    assert_eq!(host.set_json_field(&ctx, id, "action", json!("convertToWorking")), Ok(()));
    assert!(host.set_json_field(&ctx, id, "remember", json!("yes")).is_err_and(|e| e.contains("can't take")));
    assert_eq!(host.set_json_field(&ctx, id, "remember", json!(true)), Ok(()));
    assert!(host.confirm_by_id(&ctx, id));
    let answer = host.take::<ProfileMismatchDialog>().and_then(DialogResult::ok);
    assert_eq!(answer.map(|ProfileAnswer { action, remember, .. }| (action, remember)), Some((ProfileAction::ConvertToWorking, true)));

    let missing = ProfileMismatchDialog::new("scan.tif", ProfileMode::Gray, ProfileQuestion::missing("Gray Gamma 2.2")).rememberable(true);
    let id = host.open(missing);
    assert_eq!(fields(&host, id), json!({"action": "leaveAsIs"}), "a missing profile never remembers");
    assert!(host.set_json_field(&ctx, id, "action", json!("discard")).is_err_and(|e| e.contains("can't take")));
    assert_eq!(host.set_json_field(&ctx, id, "action", json!("assignWorking")), Ok(()));
    assert!(host.confirm_by_id(&ctx, id));
    let answer = host.take::<ProfileMismatchDialog>().and_then(DialogResult::ok);
    assert_eq!(answer.map(|answer| (answer.action, answer.remember)), Some((ProfileAction::AssignWorking, false)));
}
