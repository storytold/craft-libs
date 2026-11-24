//! A bounded subset of PostScript used for simple EPS previews.

use std::collections::HashMap;

/// A solid rectangle drawn by a simple EPS program.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rectangle {
    /// Left edge in PostScript points.
    pub x: f64,
    /// Bottom edge in PostScript points.
    pub y: f64,
    /// Width in PostScript points.
    pub width: f64,
    /// Height in PostScript points.
    pub height: f64,
    /// Straight sRGB RGBA.
    pub color: [u8; 4],
}

pub const MAX_SIMPLE_RECTANGLES: usize = 200_000;
pub const MAX_SIMPLE_RECTANGLE_TOKENS: usize = 2_000_000;
pub const MAX_SIMPLE_RECTANGLE_SAVE_DEPTH: usize = 64;

/// Simple PostScript tokens without comments, with braces kept as individual tokens.
fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(['\n', '\r']).flat_map(|line| {
        line.split('%').next().unwrap_or("").split_whitespace().flat_map(|word| word.split_inclusive(['{', '}'])).flat_map(|piece| {
            let (body, brace) = match piece.char_indices().last() {
                Some((at, '{' | '}')) => (piece.get(..at).unwrap_or(""), piece.get(at..)),
                _ => (piece, None),
            };
            [Some(body).filter(|body| !body.is_empty()), brace].into_iter().flatten()
        })
    })
}

/// Read solid rectangle fills and their sRGB colours from the restricted PostScript subset used
/// by simple EPS previews. None means the program uses an unsupported operator or exceeds a
/// fixed work bound. The function never executes PostScript.
pub fn simple_rectangles(ps: &[u8]) -> Option<Vec<Rectangle>> {
    let text = std::str::from_utf8(ps).ok()?;
    let mut aliases: HashMap<String, String> = HashMap::new();
    let mut stack: Vec<f64> = Vec::new();
    let mut color = [0u8, 0, 0, 255];
    let mut saved: Vec<[u8; 4]> = Vec::new();
    let mut rectangles = Vec::new();
    let mut input = tokens(text);
    let mut count = 0usize;
    while let Some(token) = input.next() {
        count += 1;
        if count > MAX_SIMPLE_RECTANGLE_TOKENS {
            return None;
        }
        if let Ok(value) = token.parse::<f64>() {
            if !value.is_finite() {
                return None;
            }
            stack.push(value);
            continue;
        }
        if let Some(name) = token.strip_prefix('/') {
            let mut body: Vec<&str> = Vec::new();
            while body.len() < 5 {
                count += 1;
                if count > MAX_SIMPLE_RECTANGLE_TOKENS {
                    return None;
                }
                match input.next() {
                    Some("def") | None => break,
                    Some(token) => body.push(token),
                }
            }
            let op = match body.as_slice() {
                ["{", op, "}"] | ["{", op, "}", _] => (*op).to_string(),
                _ => return None,
            };
            aliases.insert(name.to_string(), op);
            continue;
        }
        let op = aliases.get(token).map_or(token, String::as_str);
        let mut pop = |n: usize| -> Option<Vec<f64>> { (stack.len() >= n).then(|| stack.split_off(stack.len() - n)) };
        match op {
            "setcmykcolor" => {
                let values = pop(4)?;
                let black = values[3].clamp(0.0, 1.0);
                let channel = |value: f64| ((1.0 - value.clamp(0.0, 1.0)) * (1.0 - black) * 255.0).round() as u8;
                color = [channel(values[0]), channel(values[1]), channel(values[2]), 255];
            }
            "setrgbcolor" => {
                let values = pop(3)?;
                let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
                color = [channel(values[0]), channel(values[1]), channel(values[2]), 255];
            }
            "setgray" => {
                let values = pop(1)?;
                let gray = (values[0].clamp(0.0, 1.0) * 255.0).round() as u8;
                color = [gray, gray, gray, 255];
            }
            "rectfill" => {
                let values = pop(4)?;
                if rectangles.len() >= MAX_SIMPLE_RECTANGLES {
                    return None;
                }
                let x1 = values[0] + values[2];
                let y1 = values[1] + values[3];
                if !x1.is_finite() || !y1.is_finite() {
                    return None;
                }
                rectangles.push(Rectangle {
                    x: values[0].min(x1),
                    y: values[1].min(y1),
                    width: (x1 - values[0]).abs(),
                    height: (y1 - values[1]).abs(),
                    color,
                });
            }
            "gsave" | "save" => {
                if saved.len() >= MAX_SIMPLE_RECTANGLE_SAVE_DEPTH {
                    return None;
                }
                saved.push(color);
            }
            "grestore" | "restore" => {
                if let Some(previous) = saved.pop() {
                    color = previous;
                }
            }
            "pop" => {
                pop(1)?;
            }
            "bind" | "newpath" | "showpage" | "end" | "begin" => {}
            _ => return None,
        }
    }
    (!rectangles.is_empty()).then_some(rectangles)
}
