//! Integration tests of craft-pcraft-format: version migration steps.
use craft_pcraft_format::{Error, Step, migrate};
use serde_json::{Value, json};

fn rename_title(v: &mut Value) -> Result<(), Error> {
    let document = v.get_mut("document").and_then(Value::as_object_mut).ok_or(Error::Corrupt("no document".into()))?;
    let title = document.remove("title").unwrap_or_default();
    document.insert("name".into(), title);
    Ok(())
}

fn add_flag(v: &mut Value) -> Result<(), Error> {
    v["document"]["flag"] = json!(true);
    Ok(())
}

const STEPS: &[Step] = &[rename_title, add_flag];

#[test]
fn steps_run_in_order_from_the_file_s_version() {
    let mut v = json!({"format_version": 1, "document": {"title": "old"}});
    assert_eq!(migrate(&mut v, STEPS, 3).unwrap(), 1);
    assert_eq!(v, json!({"format_version": 3, "document": {"name": "old", "flag": true}}));
}
