//! Page ranges (N3.13): pages typed as "1-3, 5" (Acrobat's Pages selected and Print's Pages
//! field), and Acrobat's Page Range Options (All pages or From/To, with an even/odd subset).
//! Page numbers are 1-based in text and 0-based in the API. [`crate::ui::range`] has the field.

use std::collections::BTreeSet;

/// The longest range text read, in bytes.
pub const MAX_RANGE_TEXT: usize = 16_384;

/// The most pages one range text may expand to, counting overlaps again ("1-100000, 1-100000").
pub const MAX_EXPANDED: usize = 1_000_000;

/// Why a range text can't be read. `Display` gives PdfCraft's English sentence; the field shows
/// its translation (`crate::ui::range::PageRangeLabels`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageRangeError {
    /// `"{page}" is not a page number or label` (the token as typed).
    NotAPage(String),
    /// `page {n} is out of range (1–{count})`.
    OutOfRange {
        /// The page number that was asked for.
        page: usize,
        /// The number of pages in the document.
        count: usize,
    },
    /// Longer than [`MAX_RANGE_TEXT`], or expanding past [`MAX_EXPANDED`].
    TooLong,
}

impl std::fmt::Display for PageRangeError {
    // pdfcraft: crates/print/src/error.rs:46 `PrintError::said`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PageRangeError::NotAPage(token) => write!(f, "{token:?} is not a page number or label"),
            PageRangeError::OutOfRange { page, count } => write!(f, "page {page} is out of range (1\u{2013}{count})"),
            PageRangeError::TooLong => f.write_str("the page range is too long"),
        }
    }
}

impl std::error::Error for PageRangeError {}

/// The pages (0-based, sorted, each once) `text` names in a document of `count` pages: numbers and
/// `a-b` ranges separated by `,` or `;`, spaces anywhere, a reversed range read forwards. Blank
/// text is no pages (an explicit selection cleared).
// pdfcraft: crates/ui-egui/src/panels/page_selection_ui.rs:15 `parse`.
pub fn parse_pages(text: &str, count: usize) -> Result<Vec<usize>, PageRangeError> {
    if text.len() > MAX_RANGE_TEXT {
        return Err(PageRangeError::TooLong);
    }
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let number = |token: &str| {
        // `usize::from_str` takes a leading `+`; a page number is digits only, as PdfCraft reads it.
        let digits = token.trim();
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(PageRangeError::NotAPage(token.into()));
        }
        let page = digits.parse::<usize>().map_err(|_| PageRangeError::NotAPage(token.into()))?;
        if page == 0 || page > count { Err(PageRangeError::OutOfRange { page, count }) } else { Ok(page - 1) }
    };
    let mut pages = BTreeSet::new();
    let mut budget = MAX_EXPANDED;
    for part in text.split([',', ';']).map(str::trim) {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (number(a)?, number(b)?),
            None => {
                let page = number(part)?;
                (page, page)
            }
        };
        let (a, b) = (a.min(b), a.max(b));
        budget = budget.checked_sub(b - a + 1).ok_or(PageRangeError::TooLong)?;
        pages.extend(a..=b);
    }
    Ok(pages.into_iter().collect())
}

/// `pages` (0-based, any order) as range text: runs joined, "1-3, 5, 8-9".
// pdfcraft: crates/ui-egui/src/panels/page_selection_ui.rs:43 `formatted`; vectorcraft:
// crates/ui-egui/src/dialogs/export_for_screens.rs:262 `range_text`.
pub fn format_pages(pages: &[usize]) -> String {
    let pages: BTreeSet<usize> = pages.iter().copied().collect();
    let mut parts = Vec::new();
    let mut iter = pages.into_iter().peekable();
    while let Some(first) = iter.next() {
        let mut last = first;
        while let Some(next) = iter.next_if(|&p| last.checked_add(1) == Some(p)) {
            last = next;
        }
        let (first, last) = (first.saturating_add(1), last.saturating_add(1));
        parts.push(if first == last { first.to_string() } else { format!("{first}-{last}") });
    }
    parts.join(", ")
}

/// Which pages of a From/To range count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Subset {
    /// All pages in range.
    #[default]
    All,
    /// Even pages only (2, 4, … by page number).
    Even,
    /// Odd pages only.
    Odd,
}

/// Which pages a [`PageRange`] starts from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum PageScope {
    /// All pages; `from` and `to` are kept but not applied.
    #[default]
    All,
    /// The current page.
    Current,
    /// The selected pages, in order without repeats; an empty selection falls back to the current page.
    Selection,
    /// Pages `from` to `to`.
    Range,
}

/// Acrobat's Page Range Options: All pages, the current page, the selected pages, or Pages from `from` to `to`
/// (1-based), and a subset.
/// Shared by the Header & Footer, Watermark and Background dialogs in PdfCraft.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PageRange {
    /// Which pages the range starts from.
    pub scope: PageScope,
    /// First page of the range, 1-based.
    pub from: usize,
    /// The last page of the range; `None` is the document's last page, whatever its length.
    pub to: Option<usize>,
    /// Which pages of the range count: all, even or odd.
    pub subset: Subset,
}

impl Default for PageRange {
    fn default() -> Self {
        PageRange { scope: PageScope::All, from: 1, to: None, subset: Subset::All }
    }
}

impl PageRange {
    /// The pages (0-based) the range selects in a document of `count` pages, with `current` the current page and
    /// `selection` the selected pages (all 0-based). The scope picks the pages, then the subset keeps the even or odd
    /// page numbers among them; a page past the document is never returned. An end below the start selects the start
    /// page, as the range fields do when an edit is committed.
    // pdfcraft: crates/ui-egui/src/dialogs/marks_ui.rs:46 `PageRange::pages`.
    pub fn pages(&self, count: usize, current: usize, selection: &[usize]) -> Vec<usize> {
        let picked: Vec<usize> = match self.scope {
            PageScope::All => (0..count).collect(),
            PageScope::Current => vec![current],
            PageScope::Selection if selection.is_empty() => vec![current],
            PageScope::Selection => {
                let mut seen = BTreeSet::new();
                selection.iter().copied().filter(|page| seen.insert(*page)).collect()
            }
            // `from` and `to` are 1-based and inclusive: pages `from - 1` up to, not including, `to`.
            PageScope::Range => {
                let from = self.from.max(1);
                (from - 1..self.to.map_or(count, |to| to.max(from).min(count))).collect()
            }
        };
        picked
            .into_iter()
            .filter(|&page| page < count)
            .filter(|page| match self.subset {
                Subset::All => true,
                Subset::Even => (page + 1) % 2 == 0,
                Subset::Odd => (page + 1) % 2 == 1,
            })
            .collect()
    }
}
