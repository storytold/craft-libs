//! The search rule, shared by the window, hosts and tests. Text is matched by the shared [`Matcher`], so case and
//! accents are ignored in the scope's language.

use craft_i18n::matcher::Matcher;

use super::model::{PageSpec, RowSpec};

/// Whether `label`, `keywords` or `key` contains the matcher's query. An empty query matches everything.
pub fn matches(matcher: &Matcher, label: &str, keywords: &str, key: &str) -> bool {
    matcher.is_empty() || [label, keywords, key].iter().any(|text| matcher.score(text).is_some())
}

/// Whether `row` matches the matcher's query: its label, keywords, key or help.
pub fn row_matches(matcher: &Matcher, row: &RowSpec<'_>) -> bool {
    matches(matcher, row.label, row.keywords, row.key) || row.help.is_some_and(|help| matches(matcher, help, "", ""))
}

/// A page that the page list shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Listed {
    /// Its place in the host's pages.
    pub index: usize,
    /// Drawn dimmed: no visible row.
    pub dimmed: bool,
    /// The title matched (or there is no query): every visible row shows.
    pub all_rows: bool,
}

/// The pages the list shows for the matcher's query: a page whose title or keywords match, or that has a matching
/// visible row. Without a query every page is listed, the empty ones dimmed; with one an empty page is listed only
/// when its title matches.
pub fn list_pages(pages: &[PageSpec<'_>], matcher: &Matcher) -> Vec<Listed> {
    pages
        .iter()
        .enumerate()
        .filter_map(|(index, page)| {
            let title = matches(matcher, page.title, page.keywords, "");
            let any_row = page.groups.iter().flat_map(|g| &g.rows).any(|r| r.visible && row_matches(matcher, r));
            let listed = matcher.is_empty() || title || any_row;
            listed.then_some(Listed { index, dimmed: !page.has_content(), all_rows: title })
        })
        .collect()
}

/// Whether `row` shows on a page listed as `listed`, under the matcher's query.
pub fn row_shown(matcher: &Matcher, listed: &Listed, row: &RowSpec<'_>) -> bool {
    row.visible && (listed.all_rows || row_matches(matcher, row))
}

/// The place in `listed` of the page to show: the selected page, else the first listed one. The selection itself is
/// the user's and is kept while the search hides it.
pub fn shown_position(listed: &[Listed], pages: &[PageSpec<'_>], selected: &str) -> Option<usize> {
    listed.iter().position(|l| pages[l.index].id == selected).or(if listed.is_empty() { None } else { Some(0) })
}
