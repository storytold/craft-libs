use super::builder::decls;
use super::import::PAGE_FONT;
use super::{Align, Fmt, Rgb, model, parse_color};

/// A CSS length (or font-size keyword) in points; `None` when it is not one.
pub fn css_len_pt(v: &str) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    let num = |s: &str| s.trim().parse::<f32>().ok().filter(|x| x.is_finite());
    if let Some(x) = v.strip_suffix("pt") {
        num(x)
    } else if let Some(x) = v.strip_suffix("px") {
        num(x).map(|p| p * 0.75)
    } else if let Some(x) = v.strip_suffix("rem").or_else(|| v.strip_suffix("em")) {
        num(x).map(|p| p * 12.0)
    } else if let Some(x) = v.strip_suffix("in") {
        num(x).map(|p| p * 72.0)
    } else if let Some(x) = v.strip_suffix("cm") {
        num(x).map(|p| p * 72.0 / 2.54)
    } else if let Some(x) = v.strip_suffix("mm") {
        num(x).map(|p| p * 72.0 / 25.4)
    } else if let Some(x) = v.strip_suffix('%') {
        num(x).map(|p| p * 0.12)
    } else {
        match v.as_str() {
            "xx-small" => Some(7.0),
            "x-small" => Some(7.5),
            "small" | "smaller" => Some(10.0),
            "medium" => Some(12.0),
            "large" | "larger" => Some(13.5),
            "x-large" => Some(18.0),
            "xx-large" => Some(24.0),
            "xxx-large" => Some(36.0),
            _ => num(&v).map(|p| p * 0.75),
        }
    }
}

pub(super) fn strip_important(v: &str) -> &str {
    let s = v.trim();
    if let Some(pos) = s.rfind('!') {
        let after = s[pos + 1..].trim();
        if after.eq_ignore_ascii_case("important") {
            return s[..pos].trim();
        }
    }
    s
}

/// Decode CSS escapes (CSS Syntax §4.3.7): `\` and one to six hex digits, with one whitespace
/// after them, is that code point; `\` and any other character is that character. CR LF, CR and
/// form feed count as one newline, as CSS reads them (§3.3).
pub(super) fn css_unescape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    // Takes a newline (CR LF as one) or other whitespace off the front, if there is one.
    let skip_space = |it: &mut std::iter::Peekable<std::str::Chars>| match it.peek() {
        Some('\r') => {
            it.next();
            it.next_if_eq(&'\n');
        }
        Some(' ' | '\t' | '\n' | '\u{C}') => {
            it.next();
        }
        _ => {}
    };
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 && it.peek().is_some_and(char::is_ascii_hexdigit) {
            hex.extend(it.next());
        }
        if hex.is_empty() {
            // Before a newline it continues the line; at the end it is dropped.
            match it.peek() {
                Some('\r' | '\n' | '\u{C}') => skip_space(&mut it),
                Some(_) => o.extend(it.next()),
                None => {}
            }
            continue;
        }
        skip_space(&mut it);
        let cp = u32::from_str_radix(&hex, 16).unwrap_or(0);
        o.push(char::from_u32(cp).filter(|c| *c != '\0').unwrap_or('\u{FFFD}'));
    }
    o
}

/// Maximum number of tokens parsed in a CSS background shorthand declaration to prevent DOS.
const MAX_CSS_SHORTHAND_TOKENS: usize = 64;

/// Returns true if `c` is a CSS whitespace character according to CSS Syntax Level 3 § 4.2.
///
/// Only U+0020 (space), U+0009 (tab), U+000A (LF), U+000C (FF), and U+000D (CR) are CSS whitespace.
/// Non-breaking space (U+00A0) and other Unicode whitespace characters are not CSS whitespace.
fn is_css_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r')
}

/// Trims leading and trailing CSS whitespace from a string slice according to CSS Syntax Level 3 § 4.2.
fn trim_css_whitespace(s: &str) -> &str {
    s.trim_matches(is_css_whitespace)
}

/// Check whether a token is a valid CSS length or percentage value.
///
/// Returns `true` for `"0"` or numeric quantities ending in valid CSS length units
/// (`px`, `pt`, `em`, `rem`, `%`, `in`, `cm`, `mm`, `vw`, `vh`, `vmin`, `vmax`, `pc`, `ch`, `ex`).
fn is_length_or_percentage(s: &str) -> bool {
    let s = trim_css_whitespace(s);
    if s.parse::<f64>().is_ok_and(|x| x == 0.0) {
        return true;
    }
    let units = ["px", "pt", "em", "rem", "%", "in", "cm", "mm", "vw", "vh", "vmin", "vmax", "pc", "ch", "ex"];
    for unit in units {
        if let Some(num_str) = s.strip_suffix(unit)
            && trim_css_whitespace(num_str).parse::<f32>().is_ok_and(|x| x.is_finite())
        {
            return true;
        }
    }
    false
}

/// Represents a parsed CSS background layer containing its tokens and depth-zero slash count.
struct BgLayer<'a> {
    /// Tokens belonging to this layer, trimmed of CSS whitespace.
    tokens: Vec<&'a str>,
    /// Number of slashes occurring at parenthesis depth 0 in this layer.
    depth_zero_slashes: usize,
}

/// Push a sliced and trimmed token to the layer's token list, enforcing the shorthand token limit.
fn push_shorthand_token<'a>(val: &'a str, start: &mut Option<usize>, end: usize, tokens: &mut Vec<&'a str>, total_tokens: &mut usize) -> Option<()> {
    if let Some(s) = start.take()
        && let Some(token) = val.get(s..end)
    {
        let trimmed = trim_css_whitespace(token);
        if !trimmed.is_empty() {
            if *total_tokens >= MAX_CSS_SHORTHAND_TOKENS {
                return None;
            }
            tokens.push(trimmed);
            *total_tokens = total_tokens.saturating_add(1);
        }
    }
    Some(())
}

/// Parse CSS background shorthand value into comma-separated layers and tokens.
///
/// Respects quotes and parenthesis depth. Rejects and returns `None` if the token
/// limit `MAX_CSS_SHORTHAND_TOKENS` is exceeded (indicating hostile or truncated input),
/// or if quotes or parentheses are unbalanced.
fn parse_bg_layers(val: &str) -> Option<Vec<BgLayer<'_>>> {
    let mut layers = Vec::new();
    let mut current_tokens = Vec::new();
    let mut current_slashes = 0usize;
    let mut total_tokens = 0usize;

    let mut start = None;
    let mut paren_depth = 0usize;
    let mut in_quote = None;
    let mut escaped = false;
    let mut in_raw_url = false;

    let mut byte_idx = 0;
    while byte_idx < val.len() {
        let Some(c) = val[byte_idx..].chars().next() else { break };
        let char_len = c.len_utf8();
        let i = byte_idx;
        byte_idx += char_len;

        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            if start.is_none() {
                start = Some(i);
            }
            continue;
        }
        if let Some(q) = in_quote {
            if c == q {
                in_quote = None;
            }
        } else if c == '\'' || c == '"' {
            in_quote = Some(c);
            if start.is_none() {
                start = Some(i);
            }
        } else if !in_raw_url && val[i..].starts_with("/*") {
            // CSS comments outside strings and URLs are consumed before delimiter classification
            if paren_depth == 0 {
                push_shorthand_token(val, &mut start, i, &mut current_tokens, &mut total_tokens)?;
            }
            if let Some(end_offset) = val[i + 2..].find("*/") {
                byte_idx = i + 2 + end_offset + 2;
            } else {
                byte_idx = val.len();
            }
            continue;
        } else if c == '(' {
            if in_quote.is_none()
                && let Some(s) = start
                && val.get(s..i).is_some_and(|prefix| trim_css_whitespace(prefix).eq_ignore_ascii_case("url"))
            {
                in_raw_url = true;
            }
            paren_depth = paren_depth.saturating_add(1);
            if start.is_none() {
                start = Some(i);
            }
        } else if c == ')' {
            if paren_depth == 0 {
                return None;
            }
            paren_depth -= 1;
            if paren_depth == 0 {
                in_raw_url = false;
            }
        } else if paren_depth == 0 {
            if c == '/' {
                current_slashes = current_slashes.saturating_add(1);
                push_shorthand_token(val, &mut start, i, &mut current_tokens, &mut total_tokens)?;
                if total_tokens >= MAX_CSS_SHORTHAND_TOKENS {
                    return None;
                }
                if let Some(slash_token) = val.get(i..i + 1) {
                    current_tokens.push(slash_token);
                    total_tokens = total_tokens.saturating_add(1);
                }
            } else if c == ',' {
                push_shorthand_token(val, &mut start, i, &mut current_tokens, &mut total_tokens)?;
                if current_tokens.is_empty() {
                    return None;
                }
                layers.push(BgLayer { tokens: std::mem::take(&mut current_tokens), depth_zero_slashes: current_slashes });
                current_slashes = 0;
            } else if is_css_whitespace(c) {
                push_shorthand_token(val, &mut start, i, &mut current_tokens, &mut total_tokens)?;
            } else if start.is_none() {
                start = Some(i);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }

    if escaped || paren_depth != 0 || in_quote.is_some() {
        return None;
    }

    push_shorthand_token(val, &mut start, val.len(), &mut current_tokens, &mut total_tokens)?;

    if current_tokens.is_empty() {
        return None;
    }

    layers.push(BgLayer { tokens: current_tokens, depth_zero_slashes: current_slashes });

    Some(layers)
}

fn is_valid_rgb_component(s: &str) -> bool {
    let s = trim_css_whitespace(s);
    if s.eq_ignore_ascii_case("none") {
        return true;
    }
    if let Some(p) = s.strip_suffix('%') {
        trim_css_whitespace(p).parse::<f32>().is_ok_and(|v| v.is_finite())
    } else {
        s.parse::<f32>().is_ok_and(|v| v.is_finite())
    }
}

fn is_valid_hue_component(s: &str) -> bool {
    let s = trim_css_whitespace(s);
    if s.eq_ignore_ascii_case("none") {
        return true;
    }
    let num_str = if let Some(deg) = s.strip_suffix("deg") {
        deg
    } else if let Some(rad) = s.strip_suffix("rad") {
        rad
    } else if let Some(turn) = s.strip_suffix("turn") {
        turn
    } else {
        s
    };
    trim_css_whitespace(num_str).parse::<f32>().is_ok_and(|v| v.is_finite())
}

fn is_valid_zero_alpha(a: &str) -> bool {
    let a = trim_css_whitespace(a);
    if a.eq_ignore_ascii_case("none") || a == "0" || a == "0%" || a == "0.0" {
        return true;
    }
    if let Some(p) = a.strip_suffix('%') {
        trim_css_whitespace(p).parse::<f32>().is_ok_and(|v| v <= 0.0)
    } else {
        a.parse::<f32>().is_ok_and(|v| v <= 0.0)
    }
}

/// Returns true if a CSS color expression specifies a fully transparent color (alpha = 0).
fn is_zero_alpha_color(s: &str) -> bool {
    let s = trim_css_whitespace(s).to_ascii_lowercase();
    if s == "transparent" {
        return true;
    }
    if let Some(h) = s.strip_prefix('#') {
        if h.len() == 4 && h.ends_with('0') && h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return true;
        }
        if h.len() == 8 && h.ends_with("00") && h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return true;
        }
        return false;
    }
    if let Some(inner) = s.strip_prefix("rgb(").or_else(|| s.strip_prefix("rgba(")) {
        let Some(inner) = inner.strip_suffix(')') else { return false };
        let (rgb_part, alpha_part) = inner.split_once('/').map_or((inner, None), |(a, b)| (a, Some(b)));
        let (parts, alpha_str) = match alpha_part {
            Some(a) => {
                let parts: Vec<&str> = if rgb_part.contains(',') { rgb_part.split(',').collect() } else { rgb_part.split_whitespace().collect() };
                (parts, a)
            }
            None => {
                let parts: Vec<&str> = if inner.contains(',') { inner.split(',').collect() } else { inner.split_whitespace().collect() };
                if parts.len() == 4 {
                    (parts[..3].to_vec(), parts[3])
                } else {
                    return false;
                }
            }
        };
        return parts.len() == 3 && parts.iter().all(|&p| is_valid_rgb_component(p)) && is_valid_zero_alpha(alpha_str);
    }
    if let Some(inner) = s.strip_prefix("hsl(").or_else(|| s.strip_prefix("hsla(")) {
        let Some(inner) = inner.strip_suffix(')') else { return false };
        let (hsl_part, alpha_part) = inner.split_once('/').map_or((inner, None), |(a, b)| (a, Some(b)));
        let (parts, alpha_str) = match alpha_part {
            Some(a) => {
                let parts: Vec<&str> = if hsl_part.contains(',') { hsl_part.split(',').collect() } else { hsl_part.split_whitespace().collect() };
                (parts, a)
            }
            None => {
                let parts: Vec<&str> = if inner.contains(',') { inner.split(',').collect() } else { inner.split_whitespace().collect() };
                if parts.len() == 4 {
                    (parts[..3].to_vec(), parts[3])
                } else {
                    return false;
                }
            }
        };
        return parts.len() == 3
            && is_valid_hue_component(parts[0])
            && is_valid_rgb_component(parts[1])
            && is_valid_rgb_component(parts[2])
            && is_valid_zero_alpha(alpha_str);
    }
    false
}

/// Returns true if a lowercase token represents a recognized CSS image function.
fn is_valid_image_function(ltok: &str) -> bool {
    if !ltok.ends_with(')') {
        return false;
    }
    ltok.starts_with("url(")
        || ltok.starts_with("image(")
        || ltok.starts_with("image-set(")
        || ltok.starts_with("-webkit-image-set(")
        || ltok.starts_with("cross-fade(")
        || ltok.starts_with("element(")
        || ltok.starts_with("paint(")
        || ltok.starts_with("linear-gradient(")
        || ltok.starts_with("radial-gradient(")
        || ltok.starts_with("conic-gradient(")
        || ltok.starts_with("repeating-linear-gradient(")
        || ltok.starts_with("repeating-radial-gradient(")
        || ltok.starts_with("repeating-conic-gradient(")
        || ltok.starts_with("-webkit-linear-gradient(")
        || ltok.starts_with("-webkit-radial-gradient(")
        || ltok.starts_with("-webkit-gradient(")
        || ltok.starts_with("-moz-linear-gradient(")
        || ltok.starts_with("-moz-radial-gradient(")
}

/// Returns true if a lowercase token represents a recognized CSS math or environment function.
fn is_valid_math_or_value_function(ltok: &str) -> bool {
    if !ltok.ends_with(')') {
        return false;
    }
    ltok.starts_with("calc(")
        || ltok.starts_with("min(")
        || ltok.starts_with("max(")
        || ltok.starts_with("clamp(")
        || ltok.starts_with("var(")
        || ltok.starts_with("env(")
}

/// Check whether a token is a valid CSS background position component (`top`, `bottom`, `left`, `right`, `center`, length, percentage, or math function).
fn is_valid_pos_token(s: &str) -> bool {
    let ls = s.to_ascii_lowercase();
    matches!(ls.as_str(), "top" | "bottom" | "left" | "right" | "center") || is_length_or_percentage(&ls) || is_valid_math_or_value_function(&ls)
}

/// Check whether a token is a valid CSS background size component (`auto`, length, percentage, or math function).
fn is_valid_size_component(s: &str) -> bool {
    let ls = s.to_ascii_lowercase();
    ls == "auto" || is_length_or_percentage(&ls) || is_valid_math_or_value_function(&ls)
}

/// Check whether a token is a valid CSS background property token outside of position-and-size slash expressions.
///
/// Recognizes color names/hex codes, images, repeat styles, attachments, and box keywords.
/// Standalone size keywords (`cover`, `contain`, `auto`) and slashes are rejected unless positioned in a valid slash layer.
fn is_valid_non_slash_bg_token(tok: &str) -> bool {
    let ltok = tok.to_ascii_lowercase();
    if is_valid_image_function(&ltok) || is_valid_math_or_value_function(&ltok) {
        return true;
    }
    if is_zero_alpha_color(tok) || parse_color(tok).is_some() || ltok == "transparent" {
        return true;
    }
    match ltok.as_str() {
        "none" | "repeat" | "no-repeat" | "repeat-x" | "repeat-y" | "space" | "round" | "scroll" | "fixed" | "local" | "border-box"
        | "padding-box" | "content-box" | "top" | "bottom" | "left" | "right" | "center" => true,
        _ => is_length_or_percentage(&ltok),
    }
}

/// Validate that a single CSS background shorthand layer satisfies the position-and-size grammar.
///
/// If depth-zero slashes are present, exactly one slash must separate a valid `<bg-position>` from a valid `<bg-size>`.
/// Background size keywords `cover` and `contain` are rejected unless preceded by a valid position and slash (#1786).
fn is_valid_bg_layer(layer: &BgLayer<'_>) -> bool {
    if layer.depth_zero_slashes > 1 {
        return false;
    }
    if layer.depth_zero_slashes == 0 {
        return layer.tokens.iter().all(|t| is_valid_non_slash_bg_token(t));
    }

    let Some(slash_idx) = layer.tokens.iter().position(|&t| t == "/") else {
        return false;
    };
    let before_tokens = &layer.tokens[..slash_idx];
    let after_tokens = &layer.tokens[slash_idx + 1..];

    let Some(&last_before) = before_tokens.last() else {
        return false;
    };
    if !is_valid_pos_token(last_before) {
        return false;
    }
    if !before_tokens[..before_tokens.len().saturating_sub(1)].iter().all(|t| is_valid_non_slash_bg_token(t)) {
        return false;
    }

    if after_tokens.is_empty() {
        return false;
    }
    let first_after = after_tokens[0];
    let remaining_after = if first_after.eq_ignore_ascii_case("cover") || first_after.eq_ignore_ascii_case("contain") {
        &after_tokens[1..]
    } else if is_valid_size_component(first_after) {
        if after_tokens.len() > 1 && is_valid_size_component(after_tokens[1]) { &after_tokens[2..] } else { &after_tokens[1..] }
    } else {
        return false;
    };

    remaining_after.iter().all(|t| is_valid_non_slash_bg_token(t))
}

/// Apply CSS `style` declarations to formatting and block properties, updating character and block attributes.
pub(super) fn apply_style(
    css: &str,
    f: &mut Fmt,
    align: &mut Option<Align>,
    preserve: &mut bool,
    page_break: &mut bool,
    page_break_after: &mut bool,
    parent_bg: Option<Rgb>,
) {
    for (k, v) in decls(css) {
        let lv = v.to_ascii_lowercase();
        match k.as_str() {
            "font-weight" => f.bold = lv.starts_with("bold") || lv.parse::<u32>().is_ok_and(|w| w >= 600),
            "font-style" => f.italic = lv == "italic" || lv == "oblique",
            "text-decoration" | "text-decoration-line" => {
                if lv.contains("none") {
                    f.underline = false;
                    f.strike = false;
                }
                if lv.contains("underline") {
                    f.underline = true;
                }
                if lv.contains("line-through") {
                    f.strike = true;
                }
            }
            "color" => {
                if let Some(c) = parse_color(v) {
                    f.color = Some(c);
                }
            }
            "background-color" => {
                let trimmed = trim_css_whitespace(v);
                let lv = trimmed.to_ascii_lowercase();
                if lv == "transparent" || lv == "none" || lv == "initial" || lv == "unset" || is_zero_alpha_color(trimmed) {
                    f.background = None;
                } else if lv == "inherit" {
                    f.background = parent_bg;
                } else if let Some(c) = parse_color(trimmed) {
                    f.background = Some(c);
                }
            }
            "background" => {
                let Some(layers) = parse_bg_layers(v) else {
                    continue;
                };

                // CSS-wide keywords (inherit, initial, unset) must be the sole value of the declaration.
                if layers.len() == 1 && layers[0].tokens.len() == 1 {
                    let first = layers[0].tokens[0].to_ascii_lowercase();
                    match first.as_str() {
                        "inherit" => {
                            f.background = parent_bg;
                            continue;
                        }
                        "initial" | "unset" | "none" | "transparent" => {
                            f.background = None;
                            continue;
                        }
                        _ => {}
                    }
                } else if layers.iter().any(|l| l.tokens.iter().any(|t| matches!(t.to_ascii_lowercase().as_str(), "inherit" | "initial" | "unset"))) {
                    // Invalid CSS: CSS-wide keywords cannot be combined with other values.
                    continue;
                }

                // In CSS background shorthand:
                // 1. Each layer may have at most one slash separating position and size.
                // 2. All layers must satisfy the grammar (e.g. `cover` requires position and slash).
                // 3. Only the final layer may specify a background color.
                if !layers.iter().all(is_valid_bg_layer) {
                    continue;
                }

                // Only the final layer can specify background-color. If any earlier layer has a color,
                // the entire declaration is invalid CSS.
                let earlier_has_color = layers[..layers.len().saturating_sub(1)]
                    .iter()
                    .any(|l| l.tokens.iter().any(|t| is_zero_alpha_color(t) || parse_color(t).is_some()));
                if earlier_has_color {
                    continue;
                }

                // Extract background color from the final layer.
                let final_layer = &layers[layers.len().saturating_sub(1)];
                let color_tokens: Vec<_> =
                    final_layer.tokens.iter().copied().filter(|t| is_zero_alpha_color(t) || parse_color(t).is_some()).collect();
                if color_tokens.len() > 1 {
                    // Invalid CSS: multiple background colors in a single layer.
                    continue;
                }
                if let Some(&tok) = color_tokens.first() {
                    if is_zero_alpha_color(tok) {
                        f.background = None;
                    } else if let Some(c) = parse_color(tok) {
                        f.background = Some(c);
                    }
                } else {
                    // A valid background shorthand without a color (such as background: url(...) or none repeat)
                    // resets background-color to transparent.
                    f.background = None;
                }
            }
            "font-size" => {
                if let Some(p) = css_len_pt(v).filter(|p| *p >= 1.0 && *p <= 1638.0) {
                    f.size = Some(p);
                }
            }
            "font-family" => {
                let fam = css_unescape(v.split(',').next().unwrap_or("").trim().trim_matches(['"', '\'']));
                if model::is_mono(&fam) || fam.eq_ignore_ascii_case("monospace") {
                    f.code = true;
                } else if !fam.is_empty() && !matches!(fam.to_ascii_lowercase().as_str(), "serif" | "sans-serif" | "inherit" | "initial") {
                    f.font = model::kept_font(&fam).or(f.font.take());
                } else if f.font.is_some() {
                    // A generic family replaces a face the text has (a page's Times New Roman);
                    // text without one keeps the document's font.
                    match fam.to_ascii_lowercase().as_str() {
                        "serif" => f.font = Some(PAGE_FONT.into()),
                        "sans-serif" => f.font = Some("Arial".into()),
                        _ => {}
                    }
                }
            }
            "vertical-align" => {
                f.sup = lv == "super";
                f.sub = lv == "sub";
            }
            "text-align" => {
                *align = match lv.as_str() {
                    "center" => Some(Align::Center),
                    "right" | "end" => Some(Align::Right),
                    "justify" => Some(Align::Justify),
                    "left" | "start" => Some(Align::Left),
                    _ => *align,
                }
            }
            "white-space" => *preserve = lv.starts_with("pre"),
            "page-break-before" | "break-before" => *page_break = lv == "always" || lv == "page",
            "page-break-after" | "break-after" => *page_break_after = lv == "always" || lv == "page",
            _ => {}
        }
    }
}
