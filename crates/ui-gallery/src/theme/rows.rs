//! The rows of the token browser: group headings with the tokens, component tokens and sizes under them, and the
//! filters that keep some of them.

use super::subject::Pick;
use super::words::{COMPONENT_GROUP, group_of, size_group};
use craft_behavior::{DisplayOrder, ItemSource};
use craft_theme::{ComponentField, Explained, SizeOrigin, TokenRef};
use craft_ui::list::OpenSet;
use std::borrow::Cow;

/// A row of the token list: a group's heading, or a token or size under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Row {
    /// A group heading.
    Group(&'static str),
    /// A colour or component token.
    Token(TokenRef),
    /// An index into [`Explained::sizes`].
    Size(usize),
}

impl Row {
    /// The row a pick selects, when the explanation lists it.
    pub(crate) fn of(pick: &Pick, explained: &Explained) -> Option<Row> {
        match pick {
            Pick::Token(token) => Some(Row::Token(*token)),
            Pick::Size(path) => explained.sizes.iter().position(|origin| origin.path == *path).map(Row::Size),
        }
    }

    /// The pick selecting the row makes; a group heading picks nothing.
    pub(crate) fn pick(self, explained: &Explained) -> Option<Pick> {
        match self {
            Row::Group(_) => None,
            Row::Token(token) => Some(Pick::Token(token)),
            Row::Size(at) => explained.sizes.get(at).map(|origin| Pick::Size(origin.path.clone())),
        }
    }

    /// The heading the row is listed under.
    pub(crate) fn group(self, explained: &Explained) -> Option<&'static str> {
        match self {
            Row::Group(_) => None,
            Row::Token(TokenRef::Color(token)) => Some(group_of(token)),
            Row::Token(TokenRef::Component(_)) => Some(COMPONENT_GROUP),
            Row::Size(at) => explained.sizes.get(at).map(size_group),
        }
    }
}

/// What the browser's filters keep.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Filters {
    pub(crate) query: String,
    pub(crate) pinned: bool,
    pub(crate) moved: bool,
    pub(crate) failing: bool,
}

impl Filters {
    /// Whether any filter narrows the list.
    pub(crate) fn active(&self) -> bool {
        !self.query.is_empty() || self.pinned || self.moved || self.failing
    }

    fn matches_path(&self, path: &str) -> bool {
        self.query.is_empty() || path.contains(&self.query.to_lowercase())
    }

    /// Whether a colour or component token passes.
    fn keeps_token(&self, explained: &Explained, token: TokenRef, pinned: bool, moved: bool) -> bool {
        self.matches_path(&token.path())
            && (!self.pinned || pinned)
            && (!self.moved || moved)
            && (!self.failing || explained.pairs_of(token).any(|pair| !pair.pair.passes()))
    }

    /// Whether a size passes. Contrast filters hide sizes, which have no colour.
    fn keeps_size(&self, origin: &SizeOrigin) -> bool {
        self.matches_path(&origin.path) && (!self.pinned || origin.layer.from_app()) && !self.moved && !self.failing
    }
}

struct Entry {
    row: Row,
    depth: usize,
    open: bool,
    text: String,
}

/// The visible rows of the list in display order, as a collection source.
pub(crate) struct TokenRows {
    entries: Vec<Entry>,
}

impl TokenRows {
    /// The rows `filters` keep, grouped. A group shows its children when it is open or a filter is on.
    pub(crate) fn of(explained: &Explained, filters: &Filters, open: &OpenSet<Row>) -> Self {
        let mut groups: Vec<(&'static str, Vec<Row>)> = Vec::new();
        let mut add = |group: &'static str, row: Row| {
            if !groups.last().is_some_and(|(name, _)| *name == group) {
                groups.push((group, Vec::new()));
            }
            if let Some((_, rows)) = groups.last_mut() {
                rows.push(row);
            }
        };
        for origin in &explained.colors {
            let token = TokenRef::Color(origin.token);
            if filters.keeps_token(explained, token, origin.pin.is_some(), origin.contrast_move.is_some()) {
                add(group_of(origin.token), Row::Token(token));
            }
        }
        for field in ComponentField::ALL {
            let token = TokenRef::Component(*field);
            if filters.keeps_token(explained, token, false, false) {
                add(COMPONENT_GROUP, Row::Token(token));
            }
        }
        for (at, origin) in explained.sizes.iter().enumerate() {
            if filters.keeps_size(origin) {
                add(size_group(origin), Row::Size(at));
            }
        }
        let mut entries = Vec::new();
        for (name, rows) in groups {
            let heading = Row::Group(name);
            let shown = filters.active() || open.is_open(&heading);
            entries.push(Entry { row: heading, depth: 0, open: shown, text: name.to_owned() });
            if shown {
                entries.extend(rows.into_iter().map(|row| Entry { row, depth: 1, open: false, text: text_of(explained, row) }));
            }
        }
        TokenRows { entries }
    }

    /// The entry of `row`.
    fn entry(&self, row: &Row) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.row == *row)
    }

    /// The depth of `row` in the list.
    pub(crate) fn depth_of(&self, row: &Row) -> usize {
        self.entry(row).map_or(0, |entry| entry.depth)
    }

    /// Whether the group heading `row` is shown open.
    pub(crate) fn is_open(&self, row: &Row) -> bool {
        self.entry(row).is_some_and(|entry| entry.open)
    }

    /// Whether the list shows `row`.
    pub(crate) fn contains(&self, row: &Row) -> bool {
        self.entry(row).is_some()
    }

    /// Every group heading shown, for an Alt-click that opens or closes them all.
    pub(crate) fn headings(&self) -> impl Iterator<Item = &Row> {
        self.entries.iter().filter(|entry| entry.depth == 0).map(|entry| &entry.row)
    }
}

fn text_of(explained: &Explained, row: Row) -> String {
    match row {
        Row::Group(name) => name.to_owned(),
        Row::Token(token) => token.path(),
        Row::Size(at) => explained.sizes.get(at).map(|origin| origin.path.clone()).unwrap_or_default(),
    }
}

impl DisplayOrder<Row> for TokenRows {
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn key(&self, at: usize) -> Row {
        self.entries[at].row
    }

    fn depth(&self, at: usize) -> usize {
        self.entries[at].depth
    }

    fn has_children(&self, at: usize) -> bool {
        matches!(self.entries[at].row, Row::Group(_))
    }

    fn open(&self, at: usize) -> bool {
        self.entries[at].open
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.entries[at].text.as_str()))
    }
}

impl ItemSource<Row> for TokenRows {}
