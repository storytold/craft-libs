use serde_json::Value;

use crate::Error;

/// One migration step: upgrades manifest JSON from version `n` to `n + 1` in place. The runner
/// sets `format_version` after each step.
pub type Step<E = Error> = fn(&mut Value) -> Result<(), E>;

/// Upgrades the manifest JSON `v` in place to version `target`, where `steps[i]` upgrades
/// version `i + 1` to `i + 2`. Returns the version the manifest had.
///
/// - A missing or non-integer `format_version`, one past `u32`, or `0` is [`Error::Corrupt`].
/// - A version newer than `target` is [`Error::TooNew`].
/// - A version with no step to the next one is [`Error::Unsupported`].
///
/// A step's error is returned as it is, and the steps before it have already changed `v`. A step
/// that leaves `v` something other than a JSON object is [`Error::Corrupt`].
pub fn migrate<E: From<Error>>(v: &mut Value, steps: &[Step<E>], target: u32) -> Result<u32, E> {
    let found = v.get("format_version").and_then(Value::as_u64).ok_or_else(|| corrupt("manifest has no format_version"))?;
    let found = u32::try_from(found).map_err(|_| corrupt("format_version out of range"))?;
    if found == 0 {
        return Err(corrupt("format_version 0 is invalid").into());
    }
    if found > target {
        return Err(Error::TooNew { found, supported: target }.into());
    }
    for ver in found..target {
        let step =
            usize::try_from(ver - 1).ok().and_then(|i| steps.get(i)).ok_or_else(|| Error::Unsupported(format!("no migration from format {ver}")))?;
        step(v)?;
        let object = v.as_object_mut().ok_or_else(|| corrupt("a migration step left the manifest without an object"))?;
        object.insert("format_version".to_owned(), Value::from(ver + 1));
    }
    Ok(found)
}

fn corrupt(msg: &str) -> Error {
    Error::Corrupt(msg.to_owned())
}
