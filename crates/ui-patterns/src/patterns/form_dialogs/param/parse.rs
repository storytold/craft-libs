//! The spec-string parser behind `Param::parse_list`.

use super::{Param, ParamKind};
use crate::runtime::env::Label;
use crate::value_drag::SliderScale;
use craft_theme::Srgba;

/// The parameters the spec names, in order.
pub(super) fn list(spec: &str) -> Vec<Param> {
    split_fields(object_body(spec))
        .iter()
        .filter_map(|part| {
            let (key, value) = part.split_once(':')?;
            let key = key.trim().trim_matches('"');
            if key.is_empty() || key == "layer" {
                return None;
            }
            let kind = parse_kind(value.trim())?;
            Some(Param::new(key, source_label(key), kind))
        })
        .collect()
}

/// A range whose max is over this many times its min gets a logarithmic slider.
const LOG_SLIDER_RATIO: f64 = 500.0;

/// The kind one `"key":value` entry of the notation names. Values the dialog does not edit
/// (documents, structured JSON, references) are `None`.
fn parse_kind(v: &str) -> Option<ParamKind> {
    if let Some(choices) = v.strip_prefix('"') {
        let (body, rest) = choices.split_once('"').unwrap_or((choices, ""));
        let ids: Vec<&str> = body.split('|').collect();
        let default = rest.split_once('=').and_then(|(_, def)| {
            let def = def.trim();
            let def = if let Some(quoted) = def.strip_prefix('"') {
                quoted.split('"').next().unwrap_or_default()
            } else if let Some(quoted) = def.strip_prefix('\'') {
                quoted.split('\'').next().unwrap_or_default()
            } else {
                def.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | ',' | '}')).next().unwrap_or_default()
            };
            ids.contains(&def).then(|| def.to_owned())
        });
        let options = ids.iter().map(|id| ((*id).to_owned(), Label::from(source_label(id)))).collect();
        Some(ParamKind::choice(options, default))
    } else if let Some(rest) = v.strip_prefix("bool") {
        Some(ParamKind::bool(rest.trim_start_matches('=').trim() == "true"))
    } else if v == "text" {
        Some(ParamKind::text(""))
    } else if v == "color" {
        Some(ParamKind::color(None))
    } else if let Some(hex) = v.strip_prefix("color=") {
        Some(ParamKind::color(Srgba::from_hex(hex).map(Srgba::to_rgb_array)))
    } else if v == "doc" || v == "json" || v.starts_with("layer id") || v.starts_with(['[', '{']) || v.trim_end_matches('?') == "index|name" {
        None
    } else if let Some(cells) = v.strip_prefix("int[").and_then(|r| r.strip_suffix(']')).and_then(|n| n.parse::<usize>().ok()) {
        Some(ParamKind::grid(cells, 0))
    } else {
        let (range, default) = v.split_once('=').map_or((v, ""), |(a, b)| (a, b.trim()));
        if let Some((lo, hi)) = range.split_once("..") {
            let min = leading_number(lo).unwrap_or(0.0);
            let max = leading_number(hi).unwrap_or(100.0);
            let default = default.parse().unwrap_or(min);
            // Over `LOG_SLIDER_RATIO` a linear track leaves the small values no room.
            let scale = if min > 0.0 && max / min > LOG_SLIDER_RATIO { SliderScale::Log } else { SliderScale::Linear };
            Some(ParamKind::range(min, max, default, scale))
        } else {
            Some(ParamKind::int(None, None, default.parse().unwrap_or(0)))
        }
    }
}

/// Splits on the commas that separate entries: not inside a string or brackets.
fn split_fields(inner: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let (mut in_str, mut depth) = (false, 0i32);
    for ch in inner.chars() {
        if ch == '"' {
            in_str = !in_str;
        }
        if !in_str {
            match ch {
                '[' | '{' => depth += 1,
                ']' | '}' => depth -= 1,
                _ => {}
            }
        }
        if ch == ',' && !in_str && depth == 0 {
            parts.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    if !cur.trim().is_empty() {
        parts.push(cur);
    }
    parts
}

/// The number a range bound starts with, ignoring a unit or marker after it: `5000 px?` is 5000,
/// `1000 %` is 1000.
fn leading_number(bound: &str) -> Option<f64> {
    let b = bound.trim();
    if let Ok(v) = b.parse() {
        return Some(v);
    }
    let end = b.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+'))).unwrap_or(b.len());
    b.get(..end)?.parse().ok()
}

/// The inside of the spec's leading `{...}` object. Notes after it (`-> {...}` results,
/// parenthesised remarks) are documentation, not parameters.
fn object_body(spec: &str) -> &str {
    let s = spec.trim();
    let Some(body) = s.strip_prefix('{') else { return s };
    let (mut depth, mut in_str) = (0i32, false);
    for (i, ch) in body.char_indices() {
        match ch {
            '"' => in_str = !in_str,
            '[' | '{' if !in_str => depth += 1,
            ']' | '}' if !in_str => {
                if depth == 0 {
                    return body.get(..i).unwrap_or(body);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    body.trim_end_matches('}')
}

/// The English label of a key: camelCase read as Title Case words (`cellSize` is `Cell Size`), two fixed names, and a
/// key with a space is already text.
fn source_label(key: &str) -> String {
    match key {
        // Photoshop's name for the localized colour matching (Replace Color, Color Range).
        "localized" => return "Localized Color Clusters".into(),
        "bpc" => return "Black Point Compensation".into(),
        _ => {}
    }
    if key.contains(' ') {
        return key.to_owned();
    }
    let mut out = String::new();
    for (i, c) in key.chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else {
            if c.is_uppercase() {
                out.push(' ');
            }
            out.push(c);
        }
    }
    out
}
