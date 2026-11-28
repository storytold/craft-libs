//! Lists (N3.23): the bullet or number at the start of each paragraph of a list, numbered per
//! level, and the number styles (1, i, A, 01…) that list numbers and page numbers are written in.
//!
//! The app lays its paragraphs out; this module says what marker each one gets and where its
//! first and following lines start. A notes editor ([`crate::ui::notes`]), a slide's text box and
//! a document's lists use the same rules, so the numbering matches PowerPoint's and Word's.
//!
//! ```
//! use craft_text::lists::{ListMarker, ListParagraph, NumberPunctuation, NumberStyle, markers};
//!
//! let numbered = ListMarker::Number { style: NumberStyle::Arabic, start: 1, punctuation: NumberPunctuation::Period };
//! let paragraphs = [
//!     ListParagraph::new(0, Some(numbered.clone())),
//!     ListParagraph::new(1, Some(ListMarker::Bullet("•".into()))),
//!     ListParagraph::new(0, Some(numbered)),
//! ];
//! # let _ = || {
//! assert_eq!(markers(&paragraphs), [Some("1.".to_string()), Some("•".to_string()), Some("2.".to_string())]);
//! # };
//! ```

/// Numbers past this are written in Arabic digits by the styles that repeat characters (roman
/// numerals, letters, symbols), so a hostile start number can't build a huge label
/// (DesignCraft's `MAX_REPEATED_FORM`).
pub const MAX_REPEATED_FORM: u32 = 3999;

/// The deepest list level; deeper paragraphs count as this one (PowerPoint's nine levels).
pub const MAX_LEVEL: u8 = 8;

/// The indent a level without its own margin gets, per level, in points (PowerPoint's default
/// notes master).
pub const LEVEL_INDENT: f64 = 36.0;

/// How a number is written. The same styles serve list numbers and page numbers
/// (`craft_pages::numbering`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NumberStyle {
    /// 1, 2, 3
    #[default]
    Arabic,
    /// I, II, III
    UpperRoman,
    /// i, ii, iii
    LowerRoman,
    /// A, B, … Z, AA, BB (InDesign's and Word's letters repeat the letter)
    UpperLetters,
    /// a, b, … z, aa, bb
    LowerLetters,
    /// 01, 02, …
    ArabicLeadingZero,
    /// 001, 002, …
    ArabicThreeDigits,
    /// 0001, 0002, …
    ArabicFourDigits,
    /// *, †, ‡, §, ¶, ‖, then doubled (**, ††, …): footnotes.
    Symbols,
    /// 一, 二, 三, …: Chinese numerals written digit by digit, so 10 is 一〇 and 38 is 三八.
    Kanji,
}

impl NumberStyle {
    /// Every style, in the order a Style menu lists them: DesignCraft's page numbering order,
    /// then Symbols and Kanji.
    pub const ALL: [NumberStyle; 10] = [
        NumberStyle::Arabic,
        NumberStyle::UpperRoman,
        NumberStyle::LowerRoman,
        NumberStyle::UpperLetters,
        NumberStyle::LowerLetters,
        NumberStyle::ArabicLeadingZero,
        NumberStyle::ArabicThreeDigits,
        NumberStyle::ArabicFourDigits,
        NumberStyle::Symbols,
        NumberStyle::Kanji,
    ];

    /// `n` in this style. A number above [`MAX_REPEATED_FORM`] is written in Arabic digits by
    /// the roman, letter and symbol styles. 0 is "0" in the Arabic styles and an empty string in
    /// the others.
    // designcraft: crates/doc/src/attrs/rules_lists.rs:144 `NumberStyle::format` (roman, letters,
    // symbols, kanji_digits; the 3999 cap).
    pub fn format(self, n: u32) -> String {
        if n > MAX_REPEATED_FORM
            && matches!(
                self,
                NumberStyle::UpperRoman | NumberStyle::LowerRoman | NumberStyle::UpperLetters | NumberStyle::LowerLetters | NumberStyle::Symbols
            )
        {
            return n.to_string();
        }
        match self {
            NumberStyle::Arabic => n.to_string(),
            NumberStyle::ArabicLeadingZero => format!("{n:02}"),
            NumberStyle::ArabicThreeDigits => format!("{n:03}"),
            NumberStyle::ArabicFourDigits => format!("{n:04}"),
            NumberStyle::UpperRoman => roman(n),
            NumberStyle::LowerRoman => roman(n).to_lowercase(),
            NumberStyle::UpperLetters => letters(n),
            NumberStyle::LowerLetters => letters(n).to_lowercase(),
            NumberStyle::Symbols => {
                const SYMBOLS: [char; 6] = ['*', '†', '‡', '§', '¶', '‖'];
                let i = n.max(1) as usize - 1;
                std::iter::repeat_n(SYMBOLS[i % SYMBOLS.len()], i / SYMBOLS.len() + 1).collect()
            }
            NumberStyle::Kanji => {
                const DIGITS: [char; 10] = ['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
                n.to_string().chars().map(|c| c.to_digit(10).map_or(c, |d| DIGITS[d as usize])).collect()
            }
        }
    }

    /// The menu sample: the first four numbers and "…" ("1, 2, 3, 4…", "i, ii, iii, iv…",
    /// "01, 02, 03…" for the zero-padded styles, which show three). Language-neutral, so menus
    /// need no translated word for a style.
    // designcraft: crates/ui-egui/src/section_options.rs:14 `STYLES` (the samples it spells out).
    pub fn sample(self) -> String {
        let shown =
            if matches!(self, NumberStyle::ArabicLeadingZero | NumberStyle::ArabicThreeDigits | NumberStyle::ArabicFourDigits) { 3 } else { 4 };
        let numbers: Vec<String> = (1..=shown).map(|n| self.format(n)).collect();
        format!("{}…", numbers.join(", "))
    }
}

/// What follows (or surrounds) a list number: PowerPoint's autonumber schemes `arabicPeriod`,
/// `arabicParenR`, `arabicParenBoth`, `arabicPlain` and their roman and letter forms.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NumberPunctuation {
    /// "1."
    #[default]
    Period,
    /// "1)"
    ParenRight,
    /// "(1)"
    ParenBoth,
    /// "1"
    Plain,
}

/// The marker a paragraph of a list starts with.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ListMarker {
    /// A bullet: the character (or short text) as the app resolved it from its bullet font; a
    /// picture bullet is "•".
    Bullet(String),
    /// A number counted per level from `start`.
    Number {
        /// How the number is written.
        style: NumberStyle,
        /// The number of the first paragraph at this level.
        start: u32,
        /// The characters around the number.
        punctuation: NumberPunctuation,
    },
}

impl ListMarker {
    /// The number `n` (already counted from `start`) with its punctuation: "3.", "iv)", "(c)".
    // deckcraft: crates/ui-egui/src/panels/notes.rs:79 (`deckcraft_text::autonum(scheme, n)`).
    pub fn number_text(style: NumberStyle, punctuation: NumberPunctuation, n: u32) -> String {
        let number = style.format(n);
        match punctuation {
            NumberPunctuation::Period => format!("{number}."),
            NumberPunctuation::ParenRight => format!("{number})"),
            NumberPunctuation::ParenBoth => format!("({number})"),
            NumberPunctuation::Plain => number,
        }
    }
}

/// One paragraph as numbering sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListParagraph {
    /// 0 is the outermost level; deeper than [`MAX_LEVEL`] counts as [`MAX_LEVEL`].
    pub level: u8,
    /// The marker the paragraph starts with, if any.
    pub marker: Option<ListMarker>,
    /// An empty paragraph shows no marker and doesn't count.
    pub empty: bool,
}

impl ListParagraph {
    /// A non-empty paragraph at `level`.
    pub fn new(level: u8, marker: Option<ListMarker>) -> Self {
        ListParagraph { level, marker, empty: false }
    }
}

/// Each paragraph's marker text, numbered as PowerPoint numbers a text box: per level, a number
/// counts on from the last number at its level and restarts after a paragraph at the same or a
/// lower (outer) level that isn't numbered; a deeper paragraph doesn't interrupt it. Empty
/// paragraphs get `None` and don't count. A number starts at its marker's `start`.
// deckcraft: crates/ui-egui/src/panels/notes.rs:62 `bullets` (the per-level counters and their
// reset rule).
pub fn markers(paragraphs: &[ListParagraph]) -> Vec<Option<String>> {
    let mut counts = [0u32; MAX_LEVEL as usize + 1];
    paragraphs
        .iter()
        .map(|p| {
            if p.empty {
                return None;
            }
            let level = usize::from(p.level.min(MAX_LEVEL));
            let numbered = matches!(p.marker, Some(ListMarker::Number { .. }));
            for (i, count) in counts.iter_mut().enumerate().skip(level) {
                if i > level || !numbered {
                    *count = 0;
                }
            }
            match p.marker.as_ref()? {
                ListMarker::Bullet(text) => Some(text.clone()),
                ListMarker::Number { style, start, punctuation } => {
                    let count = &mut counts[level];
                    *count = count.saturating_add(1);
                    Some(ListMarker::number_text(*style, *punctuation, start.saturating_sub(1).saturating_add(*count)))
                }
            }
        })
        .collect()
}

fn roman(mut n: u32) -> String {
    if n == 0 {
        return "0".into();
    }
    const TABLE: [(u32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (value, text) in TABLE {
        while n >= value {
            out.push_str(text);
            n -= value;
        }
    }
    out
}

/// A, B, …, Z, AA, BB, …: the letter repeats.
fn letters(n: u32) -> String {
    if n == 0 {
        return String::new();
    }
    let letter = char::from(b'A' + ((n - 1) % 26) as u8);
    std::iter::repeat_n(letter, ((n - 1) / 26 + 1) as usize).collect()
}

/// Where a paragraph's first line and its other lines start, in points from the text box's
/// start edge: `margin` (or [`LEVEL_INDENT`] × level when the paragraph has none) for the other
/// lines, `margin + indent` for the first. A negative `indent` is a hanging indent: the marker
/// sits in it and the text of every line lines up.
// deckcraft: crates/ui-egui/src/panels/notes.rs:54 `indents`.
pub fn line_starts(level: u8, margin: Option<f64>, indent: Option<f64>) -> (f64, f64) {
    let margin = margin.unwrap_or(LEVEL_INDENT * f64::from(level.min(MAX_LEVEL)));
    (margin + indent.unwrap_or(0.0), margin)
}
