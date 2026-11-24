//! Readers for window geometry that apps stored before the `window.json` store.
//!
//! eframe keeps a RON map of strings in `app.ron` (`ui.ron`); its `"window"` entry is the RON of
//! egui-winit's `WindowSettings`, and `"egui"` holds the interface zoom. No RON crate is
//! involved: the readers look up the few named fields they need, in both layouts eframe has
//! used (a flat struct, and one with a nested `normal_geometry`). Nothing is written back; old
//! builds keep working from the old file.
//!
//! An old layout has no scale factor, so imports assume 1; [`place`](super::place) takes the
//! real one from the monitor the window lands on.

use std::path::Path;

use serde_json::Value;

use super::place::{NormalRect, Placement};

/// The placement in an eframe `app.ron` text: its `"window"` entry, with the size converted to
/// points at zoom 1 using the `"egui"` entry's `zoom_factor` (1 when absent).
pub fn eframe_app_ron(text: &str) -> Option<Placement> {
    let window = ron_string_entry(text, "window")?;
    let zoom = ron_string_entry(text, "egui").and_then(|e| number(field(&e, "zoom_factor")?)).unwrap_or(1.0);
    eframe_window_settings(&window, zoom)
}

/// [`eframe_app_ron`] for a file; `None` when it is missing or unreadable.
pub fn eframe_app_ron_file(path: &Path) -> Option<Placement> {
    eframe_app_ron(&std::fs::read_to_string(path).ok()?)
}

/// The placement in the RON of one eframe `WindowSettings`. `zoom` is the interface zoom factor
/// the size was saved under (eframe saves points at that zoom).
pub fn eframe_window_settings(ron: &str, zoom: f32) -> Option<Placement> {
    let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 };
    // The nested layout also names the scale; the flat one does not.
    let geometry = field(ron, "normal_geometry").and_then(group).unwrap_or(ron);
    let scale = field(geometry, "scale_factor").and_then(number).filter(|s| *s > 0.0).unwrap_or(1.0);
    let size = field(geometry, "inner_size_points").and_then(vec2)?;
    let pos = field(geometry, "outer_position_pixels").and_then(vec2).or_else(|| field(geometry, "inner_position_pixels").and_then(vec2));
    // The window's pixel size divided by zoom × scale (eframe >= 0.36), so × zoom is points at
    // zoom 1. Older eframe saved points at the then-current zoom, which this reads the same way.
    let size = size.map(|v| v * zoom);
    // Windows parks a minimized window near (-32000, -32000).
    let pos = pos.filter(|p| p.iter().all(|v| *v > -30000.0));
    Some(Placement {
        normal: NormalRect { pos: pos.map_or([0.0; 2], |p| p.map(|v| v / scale)), size },
        position_px: pos.map(|p| p.map(|v| v.round() as i32)),
        scale,
        zoom,
        maximized: field(ron, "maximized").is_some_and(truth),
        fullscreen: field(ron, "fullscreen").is_some_and(truth),
        monitor: None,
    })
}

/// The value at `path` of a JSON object (`json_at(&v, &["window", "size"])`).
pub fn json_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(v, |v, k| v.get(*k))
}

/// A finite number at `path`.
pub fn json_f32(v: &Value, path: &[&str]) -> Option<f32> {
    json_at(v, path)?.as_f64().filter(|n| n.is_finite()).map(|n| n as f32)
}

/// A bool at `path`.
pub fn json_bool(v: &Value, path: &[&str]) -> Option<bool> {
    json_at(v, path)?.as_bool()
}

/// A pair of finite numbers at `path`, written as `[a, b]` or as an object with the two given keys.
pub fn json_pair(v: &Value, path: &[&str], keys: [&str; 2]) -> Option<[f32; 2]> {
    let at = json_at(v, path)?;
    let num = |x: Option<&Value>| x?.as_f64().filter(|n| n.is_finite()).map(|n| n as f32);
    match at {
        Value::Array(a) if a.len() == 2 => Some([num(a.first())?, num(a.get(1))?]),
        Value::Object(_) => Some([num(at.get(keys[0]))?, num(at.get(keys[1]))?]),
        _ => None,
    }
}

/// [`json_at`] on the text of a JSON file; `None` when missing or not JSON.
pub fn read_json_file(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

// ---- RON scanning --------------------------------------------------------------------------

/// The text after `name:` for the first `name` that is a whole identifier.
fn field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(i) = text[from..].find(name) {
        let start = from + i;
        let end = start + name.len();
        let before_ok = text[..start].chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        let rest = text[end..].trim_start();
        if before_ok && let Some(value) = rest.strip_prefix(':') {
            return Some(value.trim_start());
        }
        from = end;
    }
    None
}

/// The text inside the parentheses that `s` starts with, as `(…)` or `Some(…)`.
fn group(s: &str) -> Option<&str> {
    let s = s.strip_prefix("Some").map_or(s, str::trim_start);
    let s = s.strip_prefix('(')?;
    let mut depth = 1usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[..i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn number(s: &str) -> Option<f32> {
    let end = s.find(|c: char| !(c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))).unwrap_or(s.len());
    s[..end].parse::<f32>().ok().filter(|n| n.is_finite())
}

fn truth(s: &str) -> bool {
    s.starts_with("true")
}

/// `Some((x: 1.0, y: 2.0))`, `(x: 1.0, y: 2.0)` or `(1.0, 2.0)`; `None` for `None`.
fn vec2(s: &str) -> Option<[f32; 2]> {
    let inner = group(s)?;
    // `Some((…))` wraps the pair in a second group.
    let inner = if inner.trim_start().starts_with('(') { group(inner.trim_start())? } else { inner };
    if let (Some(x), Some(y)) = (field(inner, "x").and_then(number), field(inner, "y").and_then(number)) {
        return Some([x, y]);
    }
    let (a, b) = inner.split_once(',')?;
    Some([number(a.trim())?, number(b.trim())?])
}

/// The string value for `key` in a RON map of strings (`{"key": "value", …}`), unescaped.
fn ron_string_entry(text: &str, key: &str) -> Option<String> {
    let mut literals = string_literals(text).into_iter();
    while let Some(k) = literals.next() {
        let v = literals.next()?;
        if k == key {
            return Some(v);
        }
    }
    None
}

/// Every string literal of `text` in order: `"…"` with escapes, and raw `r#"…"#`.
fn string_literals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'r' && matches!(b.get(i + 1), Some(b'"' | b'#')) {
            let hashes = b[i + 1..].iter().take_while(|c| **c == b'#').count();
            if b.get(i + 1 + hashes) == Some(&b'"') {
                let body = i + 2 + hashes;
                let close = format!("\"{}", "#".repeat(hashes));
                let Some(len) = text[body..].find(&close) else { break };
                out.push(text[body..body + len].to_owned());
                i = body + len + close.len();
                continue;
            }
        }
        if b[i] == b'"' {
            let (s, next) = escaped(text, i + 1);
            out.push(s);
            i = next;
            continue;
        }
        i += 1;
    }
    out
}

/// Reads an escaped literal body starting at byte `from`; returns it and the index after the closing quote.
fn escaped(text: &str, from: usize) -> (String, usize) {
    let mut s = String::new();
    let mut chars = text[from..].char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return (s, from + i + 1),
            '\\' => match chars.next().map(|(_, e)| e) {
                Some('n') => s.push('\n'),
                Some('t') => s.push('\t'),
                Some('r') => s.push('\r'),
                Some('u') => {
                    let mut hex = String::new();
                    for (_, h) in chars.by_ref() {
                        if h == '}' {
                            break;
                        }
                        if h != '{' {
                            hex.push(h);
                        }
                    }
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        s.push(ch);
                    }
                }
                Some(other) => s.push(other),
                None => break,
            },
            c => s.push(c),
        }
    }
    (s, text.len())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::restore::Store;

    const FLAT: &str = r#"{
    "egui": "(options:(zoom_factor:1.5,other:1))",
    "window": "(outer_position_pixels:Some((x:120.0,y:-30.0)),fullscreen:false,maximized:true,inner_size_points:Some((x:800.0,y:600.0)))",
}"#;

    #[test]
    fn imports_the_flat_eframe_layout_with_zoom() {
        let p = eframe_app_ron(FLAT).unwrap();
        assert_eq!(p.position_px, Some([120, -30]));
        assert_eq!(p.normal.size, [1200.0, 900.0]);
        assert_eq!(p.zoom, 1.5);
        assert!(p.maximized && !p.fullscreen);
    }

    #[test]
    fn imports_the_nested_layout_with_its_scale() {
        let w = "(normal_geometry:(outer_position_pixels:Some((x:200.0,y:100.0)),scale_factor:2.0,inner_size_points:(x:640.0,y:480.0)),fullscreen:true,maximized:false)";
        let text = format!("{{\"window\": r#\"{w}\"#}}");
        let p = eframe_app_ron(&text).unwrap();
        assert_eq!((p.scale, p.position_px, p.normal.size), (2.0, Some([200, 100]), [640.0, 480.0]));
        assert_eq!(p.normal.pos, [100.0, 50.0]);
        assert!(p.fullscreen);
    }

    #[test]
    fn missing_position_and_garbage_are_handled() {
        let w = "(outer_position_pixels:None,fullscreen:false,maximized:false,inner_size_points:Some((x:300.0,y:200.0)))";
        assert_eq!(eframe_window_settings(w, 1.0).unwrap().position_px, None);
        assert!(eframe_window_settings("(", 1.0).is_none());
        assert!(eframe_app_ron("not ron").is_none());
    }

    #[test]
    fn minimized_windows_parked_off_screen_have_no_position() {
        let w = "(outer_position_pixels:Some((x:-32000.0,y:-32000.0)),fullscreen:false,maximized:false,inner_size_points:Some((x:300.0,y:200.0)))";
        let p = eframe_window_settings(w, 1.0).unwrap();
        assert_eq!((p.position_px, p.normal.size), (None, [300.0, 200.0]));
    }

    #[test]
    fn imported_placement_lands_in_the_store() {
        let mut store = Store::in_memory();
        store.set("main", &eframe_app_ron(FLAT).unwrap());
        assert_eq!(store.get("main").unwrap().position_px, Some([120, -30]));
    }

    #[test]
    fn json_helpers_read_nested_values() {
        let v: Value = serde_json::from_str(r#"{"window":{"size":[800,600],"pos":{"x":1,"y":2},"max":true,"w":3.5}}"#).unwrap();
        assert_eq!(json_pair(&v, &["window", "size"], ["w", "h"]), Some([800.0, 600.0]));
        assert_eq!(json_pair(&v, &["window", "pos"], ["x", "y"]), Some([1.0, 2.0]));
        assert_eq!(json_bool(&v, &["window", "max"]), Some(true));
        assert_eq!(json_f32(&v, &["window", "w"]), Some(3.5));
        assert_eq!(json_f32(&v, &["window", "nope"]), None);
    }
}
