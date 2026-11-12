//! Helpers for the readers that build this shape from other files.

use std::collections::BTreeMap;

/// Most distinct warnings [`Warnings`] keeps.
pub const MAX_WARNINGS: usize = 200;

/// Distinct warnings in first-seen order, with repeat counts: a reader's notes repeat per layer.
#[derive(Debug, Default)]
pub struct Warnings {
    list: Vec<(String, usize)>,
    index: BTreeMap<String, usize>,
}

impl Warnings {
    /// Note `msg`: counted when it was met before, kept when fewer than [`MAX_WARNINGS`] are.
    pub fn add(&mut self, msg: String) {
        if let Some(i) = self.index.get(&msg) {
            if let Some(e) = self.list.get_mut(*i) {
                e.1 += 1;
            }
        } else if self.list.len() < MAX_WARNINGS {
            self.index.insert(msg.clone(), self.list.len());
            self.list.push((msg, 1));
        }
    }

    /// The notes met so far, without their counts.
    pub fn seen(&self) -> impl Iterator<Item = &str> {
        self.list.iter().map(|(msg, _)| msg.as_str())
    }

    /// The notes, each with `(×n)` after it where it was met more than once (a reader of the
    /// notes can take the suffix off to look the note up).
    pub fn finish(self) -> Vec<String> {
        self.list.into_iter().map(|(m, n)| if n > 1 { format!("{m} (×{n})") } else { m }).collect()
    }
}

/// A font style under the name the format uses: the standard weight names as words ("SemiBold"
/// → "Semi Bold", "ExtraBoldItalic" → "Extra Bold Italic", "Regular Italic" → "Italic"). Other
/// names stay as they are.
pub fn style_name(style: &str) -> String {
    const WEIGHTS: [&str; 12] =
        ["Thin", "Extra Light", "Ultra Light", "Light", "Regular", "Medium", "Semi Bold", "Demi Bold", "Bold", "Extra Bold", "Ultra Bold", "Black"];
    let compact = |s: &str| s.chars().filter(|c| !matches!(c, ' ' | '-' | '_')).flat_map(char::to_lowercase).collect::<String>();
    let key = compact(style);
    let (base, italic) = match key.strip_suffix("italic") {
        Some(b) => (b, true),
        None => (key.as_str(), false),
    };
    let weight = if base.is_empty() && italic { Some("Regular") } else { WEIGHTS.into_iter().find(|w| compact(w) == base) };
    match (weight, italic) {
        (None, _) => style.to_string(),
        (Some("Regular"), true) => "Italic".into(),
        (Some(w), true) => format!("{w} Italic"),
        (Some(w), false) => w.into(),
    }
}
