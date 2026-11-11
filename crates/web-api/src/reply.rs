//! Reply shaping that needs no browser, so it is natively tested.

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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::screenshot_data;

    #[test]
    fn every_apps_field_name_becomes_data_and_stays_where_it_was() {
        for field in ["pngBase64", "png_base64", "png", "base64"] {
            let reply = json!({"ok": true, "result": {field: "UE5H", "width": 2}});
            let out = screenshot_data(reply);
            assert_eq!(out["result"]["data"], "UE5H", "{field}");
            assert_eq!(out["result"][field], "UE5H", "{field} is kept");
            assert_eq!(out["result"]["width"], 2);
        }
    }

    #[test]
    fn a_reply_with_its_own_data_a_path_only_or_an_error_is_left_alone() {
        for reply in [
            json!({"ok": true, "result": {"data": "own", "png": "other"}}),
            json!({"ok": true, "result": {"data": null, "png": "UE5H"}}),
            json!({"ok": true, "result": {"data": {"n": 1}, "png": "UE5H"}}),
            json!({"ok": true, "result": {"path": "/tmp/a.png", "width": 2}}),
            json!({"ok": false, "error": "boom"}),
            json!({"ok": false, "error": "boom", "result": {"png": "UE5H"}}),
            json!({"ok": true, "result": null}),
            json!({"ok": true, "result": {"png": 7}}),
        ] {
            assert_eq!(screenshot_data(reply.clone()), reply);
        }
    }

    #[test]
    fn the_first_field_in_the_list_wins() {
        let out = screenshot_data(json!({"ok": true, "result": {"png": "b", "pngBase64": "a"}}));
        assert_eq!(out["result"]["data"], "a");
    }
}
