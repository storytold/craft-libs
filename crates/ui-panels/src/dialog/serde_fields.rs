//! The serde round trip a dialog's `json_fields` and `set_json_field` share.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::host::FieldRefusal;

/// The value as a JSON object; `None` when it does not serialise to one.
pub fn serde_fields<T: Serialize>(value: &T) -> Option<serde_json::Value> {
    serde_json::to_value(value).ok().filter(Value::is_object)
}

/// Replaces the top-level key `field` of `value` with `json`, by serialising the value, replacing the key and reading
/// it back. An unknown key or a value `T` cannot hold gives the refusal's message and leaves `value` unchanged.
pub fn set_serde_field<T: Serialize + DeserializeOwned>(value: &mut T, field: &str, json: serde_json::Value) -> Result<(), String> {
    let Some(Value::Object(mut object)) = serde_json::to_value(&*value).ok() else {
        return Err(FieldRefusal::NoFields.message(field));
    };
    let Some(slot) = object.get_mut(field) else { return Err(FieldRefusal::Unknown.message(field)) };
    *slot = json.clone();
    match serde_json::from_value::<T>(Value::Object(object)) {
        Ok(read) => {
            *value = read;
            Ok(())
        }
        Err(_) => Err(FieldRefusal::Rejects(&json).message(field)),
    }
}
