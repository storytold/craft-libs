//! Reply shaping that needs no browser.

use serde_json::Value;

/// The names the desktop control protocols give the base64 PNG of a screenshot reply, in the order
/// they are looked for. VectorCraft sends `pngBase64` (when asked with `data: true`), SoundCraft
/// `png_base64`, WordCraft `png`, GridCraft `base64`.
const PNG_FIELDS: [&str; 4] = ["pngBase64", "png_base64", "png", "base64"];

/// A successful `screenshot` reply with the PNG also under `data`, whatever the app calls it, so a
/// page reads `const { data } = await window.<app>.screenshot()` in every app. The app's own field
/// stays where it is. A reply with no inline PNG (the app wrote it to a `path`), a reply that
/// already has a `data` key of any type, and an error reply are returned as they are.
pub fn screenshot_data(mut reply: Value) -> Value {
    if reply.get("ok").and_then(Value::as_bool) == Some(false) {
        return reply;
    }
    let Some(result) = reply.get_mut("result").and_then(Value::as_object_mut) else {
        return reply;
    };
    if result.contains_key("data") {
        return reply;
    }
    if let Some(png) = PNG_FIELDS.iter().find_map(|field| result.get(*field).filter(|v| v.is_string()).cloned()) {
        result.insert("data".to_string(), png);
    }
    reply
}
