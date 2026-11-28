//! The Pattern Fill dialog's automation: its fields, set and confirmed through the host.

use craft_draw::ui::pattern::{PatternColors, PatternFillDialog, PatternFillOptions, PatternSwatch};
use craft_theme::Srgba;
use craft_ui::dialog::{DialogHost, DialogId, DialogResult};
use craft_ui::testing::trial;
use serde_json::{Value, json};

fn swatches() -> Vec<PatternSwatch> {
    vec![PatternSwatch::new("bubbles", "Bubbles"), PatternSwatch::new("dots", "Dots")]
}

fn fields(host: &DialogHost, id: DialogId) -> Value {
    host.open_dialogs().into_iter().find(|open| open.id == id).and_then(|open| open.fields).unwrap_or(Value::Null)
}

/// A bitmap pattern lists its fields, refuses an unknown or malformed one, takes a scale held to its range, and OK
/// answers the set values; a preset pattern refuses the bitmap rows as inactive.
#[trial]
fn pattern_fill_automation_lists_sets_and_confirms_its_fields() {
    let ctx = egui::Context::default();
    let mut host = DialogHost::new();

    let id = host.open(PatternFillDialog::new(PatternFillOptions::bitmap("bubbles"), swatches()));
    assert_eq!(fields(&host, id)["scale"], json!(100.0), "the opening scale is listed");
    assert!(host.set_json_field(&ctx, id, "depth", json!(3)).is_err(), "an unknown field is refused");
    assert!(host.set_json_field(&ctx, id, "pattern", json!("missing")).is_err(), "a pattern the app does not offer is refused");
    assert_eq!(host.set_json_field(&ctx, id, "scale", json!(250)), Ok(()));
    assert_eq!(host.set_json_field(&ctx, id, "scale", json!(5000)), Ok(()), "a scale past the range is held to it");
    assert_eq!(fields(&host, id)["scale"], json!(*PatternFillOptions::SCALE.end()));
    assert_eq!(host.set_json_field(&ctx, id, "scale", json!(250)), Ok(()));
    assert_eq!(host.set_json_field(&ctx, id, "pattern", json!("dots")), Ok(()));
    assert!(host.confirm::<PatternFillDialog>(&ctx), "OK is enabled");
    let options = host.take::<PatternFillDialog>().and_then(DialogResult::ok).expect("OK answers the options");
    assert_eq!((options.pattern.as_str(), options.scale), ("dots", 250.0), "the answer carries the set values");

    let colors = PatternColors::new(Srgba::BLACK, Srgba::WHITE);
    let id = host.open(PatternFillDialog::new(PatternFillOptions::preset("bubbles", colors), swatches()));
    assert!(host.set_json_field(&ctx, id, "scale", json!(200)).is_err(), "a bitmap row is inactive in preset mode");
    assert_eq!(host.set_json_field(&ctx, id, "colors", json!({ "foreground": Srgba::WHITE, "background": Srgba::BLACK })), Ok(()));
}
