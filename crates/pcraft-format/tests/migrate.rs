use craft_pcraft_format::{Error, FORMAT_VERSION, Step, migrate};
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

fn replace_with_array(v: &mut Value) -> Result<(), Error> {
    *v = json!([]);
    Ok(())
}

const STEPS: &[Step] = &[rename_title, add_flag];

#[test]
fn steps_run_in_order_from_the_file_s_version() {
    let mut v = json!({"format_version": 1, "document": {"title": "old"}});
    assert_eq!(migrate(&mut v, STEPS, 3).unwrap(), 1);
    assert_eq!(v, json!({"format_version": 3, "document": {"name": "old", "flag": true}}));
}

#[test]
fn a_file_at_a_later_version_runs_only_the_later_steps() {
    let mut v = json!({"format_version": 2, "document": {"title": "kept"}});
    assert_eq!(migrate(&mut v, STEPS, 3).unwrap(), 2);
    assert_eq!(v, json!({"format_version": 3, "document": {"title": "kept", "flag": true}}));
}

#[test]
fn the_current_version_is_left_unchanged() {
    let mut v = json!({"format_version": FORMAT_VERSION, "document": {}});
    let before = v.clone();
    let none: &[Step] = &[];
    assert_eq!(migrate(&mut v, none, FORMAT_VERSION).unwrap(), FORMAT_VERSION);
    assert_eq!(v, before);
}

#[test]
fn a_newer_version_is_too_new() {
    let mut v = json!({"format_version": 4});
    assert!(matches!(migrate(&mut v, STEPS, 3), Err(Error::TooNew { found: 4, supported: 3 })));
}

#[test]
fn a_missing_zero_or_out_of_range_version_is_corrupt() {
    for v in [
        json!({}),
        json!({"format_version": 0}),
        json!({"format_version": "1"}),
        json!({"format_version": -1}),
        json!({"format_version": 1.5}),
        json!({"format_version": 4_294_967_296_u64}),
        json!([1]),
        json!(null),
    ] {
        let mut v = v;
        assert!(matches!(migrate(&mut v, STEPS, 3), Err(Error::Corrupt(_))), "{v}");
    }
}

#[test]
fn a_version_with_no_step_is_unsupported() {
    let mut v = json!({"format_version": 1, "document": {"title": "t"}});
    let one: &[Step] = &[rename_title];
    assert!(matches!(migrate(&mut v, one, 3), Err(Error::Unsupported(m)) if m == "no migration from format 2"));
    assert_eq!(v["format_version"], 2);
}

#[test]
fn a_step_that_leaves_no_object_is_corrupt() {
    let mut v = json!({"format_version": 1});
    let steps: &[Step] = &[replace_with_array];
    assert!(matches!(migrate(&mut v, steps, 2), Err(Error::Corrupt(_))));
}

#[derive(Debug)]
enum AppError {
    Format(Error),
    Step(&'static str),
}

impl From<Error> for AppError {
    fn from(e: Error) -> Self {
        AppError::Format(e)
    }
}

fn failing(_: &mut Value) -> Result<(), AppError> {
    Err(AppError::Step("refused"))
}

fn app_add_flag(v: &mut Value) -> Result<(), AppError> {
    add_flag(v).map_err(AppError::from)
}

#[test]
fn steps_return_the_caller_s_error_type() {
    let steps: &[Step<AppError>] = &[app_add_flag, failing];
    let mut v = json!({"format_version": 1, "document": {}});
    assert!(matches!(migrate(&mut v, steps, 3), Err(AppError::Step("refused"))));
    assert_eq!(v, json!({"format_version": 2, "document": {"flag": true}}));
    let mut newer = json!({"format_version": 9});
    assert!(matches!(migrate(&mut newer, steps, 3), Err(AppError::Format(Error::TooNew { found: 9, supported: 3 }))));
}
