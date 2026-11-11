//! The accessibility bridge: a page view draws pages as pictures, so on its own it is one opaque node to a screen
//! reader. The bridge describes the text under it, the way a browser describes a page: each paragraph a Paragraph,
//! Heading (with its level) or ListItem (grouped in Lists, with its ListMarker), holding one TextRun per laid-out line
//! with its characters' screen positions; tables are Table > Row > Cell, pictures Images named by their alt text,
//! hyperlinks Links around their runs. The page view's own node is the Document; the nodes below it are its
//! sub-nodes, and the document's text selection is the Document's text selection.
//!
//! The app describes the pages it was asked for through [`AccessDescriber`]. The bridge builds nothing unless a
//! screen reader (or a test) reads the page view's node, describes only the pages on screen and one either side, and
//! caps the tree at [`MAX_NODES`].

use std::collections::HashMap;
use std::ops::Range;

use craft_behavior::{PartKey, PartRef};
use craft_ui::contract::{Access, GridPlace, Name, Role};
use egui::{Pos2, Rect};

use super::view::PlacedPage;

/// The most nodes described at once (a dense page has a few hundred).
pub const MAX_NODES: usize = 6000;
/// AccessKit's word starts are bytes: a text run holds at most this many characters; a longer line is split into
/// runs.
pub const MAX_RUN: usize = 255;
/// The deepest table nesting described.
pub const MAX_DEPTH: usize = 16;

/// The key of every node the bridge describes, indexed in creation order.
const BLOCK: PartKey = PartKey::new("block");

/// What a block node is.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlockRole {
    /// A paragraph of text.
    Paragraph,
    /// A heading at outline level 1 to 9.
    Heading(u8),
    /// A list item at list level 0 to 8, with its marker text ("1.", "•").
    #[non_exhaustive]
    ListItem {
        /// List level, 0 to 8.
        level: u8,
        /// The marker text.
        marker: String,
    },
    /// A table.
    Table,
    /// A table row.
    Row,
    /// A table cell.
    Cell,
    /// A picture or chart named by its alt text.
    Image(String),
    /// A hyperlink to `url`.
    Link(String),
}

/// A handle to a node the describer added, for adding its children.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AccessNode(u32);

/// A text position the app names: (node of the text run, char offset in it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AccessPosition {
    /// The node of the text run.
    pub run: AccessNode,
    /// Character offset into the run's text.
    pub offset: usize,
}

impl BlockRole {
    /// A list item at list level `level` (0 to 8) with its marker text.
    pub fn list_item(level: u8, marker: impl Into<String>) -> Self {
        BlockRole::ListItem { level, marker: marker.into() }
    }
}

impl AccessPosition {
    /// The position at character `offset` of the text run `run`.
    pub fn new(run: AccessNode, offset: usize) -> Self {
        AccessPosition { run, offset }
    }
}

/// What a built node is.
#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Block(BlockRole),
    List,
    Marker(String),
    Run { text: String, positions: Vec<f32>, widths: Vec<f32> },
}

/// A node being built.
#[derive(Clone, Debug, PartialEq)]
struct Built {
    parent: Option<AccessNode>,
    kind: Kind,
    /// The node's rectangle on screen: a run's own, a block's the union of its children's.
    bounds: Option<Rect>,
    /// Tables from the document down to this node, itself included.
    tables: usize,
    /// A table's rows, a row's cells.
    parts: usize,
    /// A table's widest row.
    columns: usize,
    /// A row's place in its table, a cell's in its row.
    place: usize,
}

/// Collects one pass's description. Every method returns `None` once [`MAX_NODES`] is reached (or the parent is past
/// [`MAX_DEPTH`] tables deep), and the describer stops.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccessBuilder {
    nodes: Vec<Built>,
    /// Per parent, the List node still taking items and its items' level.
    open_lists: HashMap<Option<AccessNode>, (AccessNode, u8)>,
}

impl AccessBuilder {
    /// An empty description.
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of nodes described so far.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether nothing is described yet.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    fn built(&mut self, node: AccessNode) -> Option<&mut Built> {
        self.nodes.get_mut(node.0 as usize)
    }

    /// Adds a node of `kind` under `parent`, or `None` when the tree is full or `parent` isn't one of its nodes.
    fn push(&mut self, parent: Option<AccessNode>, kind: Kind, tables: usize, bounds: Option<Rect>) -> Option<AccessNode> {
        if self.nodes.len() >= MAX_NODES {
            return None;
        }
        if let Some(parent) = parent {
            self.built(parent)?;
        }
        let at = AccessNode(u32::try_from(self.nodes.len()).ok()?);
        self.nodes.push(Built { parent, kind, bounds, tables, parts: 0, columns: 0, place: 0 });
        Some(at)
    }

    /// A block under `parent` (`None`: under the document). Consecutive list items at one level are grouped into a List
    /// node by the builder.
    // wordcraft: crates/ui-egui/src/a11y.rs:239 `Builder::container`, :274 `para_node`.
    pub fn block(&mut self, parent: Option<AccessNode>, role: BlockRole) -> Option<AccessNode> {
        let depth = match parent {
            Some(parent) => self.built(parent)?.tables,
            None => 0,
        };
        let tables = depth + usize::from(role == BlockRole::Table);
        if tables > MAX_DEPTH {
            return None;
        }
        if !matches!(role, BlockRole::ListItem { .. }) {
            self.open_lists.remove(&parent);
        }
        let mut under = parent;
        if let BlockRole::ListItem { level, .. } = &role {
            under = match self.open_lists.get(&parent) {
                Some((list, open)) if open == level => Some(*list),
                _ => {
                    let list = self.push(parent, Kind::List, depth, None)?;
                    self.open_lists.insert(parent, (list, *level));
                    Some(list)
                }
            };
        }
        let place = match (&role, parent.and_then(|p| self.built(p))) {
            (BlockRole::Row | BlockRole::Cell, Some(above)) => {
                above.parts += 1;
                above.parts - 1
            }
            _ => 0,
        };
        if role == BlockRole::Cell {
            let row = parent.and_then(|p| self.built(p)).map(|row| (row.parts, row.parent));
            if let Some((cells, Some(table))) = row {
                let table = self.built(table)?;
                table.columns = table.columns.max(cells);
            }
        }
        let marker = match &role {
            BlockRole::ListItem { marker, .. } if !marker.trim().is_empty() => Some(marker.clone()),
            _ => None,
        };
        let at = self.push(under, Kind::Block(role), tables, None)?;
        self.built(at)?.place = place;
        if let Some(marker) = marker {
            self.push(Some(at), Kind::Marker(marker), depth, None);
        }
        Some(at)
    }

    /// One laid-out line's text under `parent`, with each character's screen x and width and the line's screen
    /// rectangle; word starts are worked out from the text.
    // wordcraft: crates/ui-egui/src/a11y.rs:431 `Builder::flush`, :501 `push_char`, :515 `word_starts`.
    pub fn line(&mut self, parent: AccessNode, text: &str, chars: &[(f32, f32)], line: Rect) -> Option<AccessNode> {
        let tables = self.built(parent)?.tables;
        let text: Vec<char> = text.chars().collect();
        let mut first = None;
        let mut start = 0;
        loop {
            let end = (start + MAX_RUN).min(text.len());
            let spans: Vec<(f32, f32)> =
                (start..end).map(|i| chars.get(i).or_else(|| chars.last()).map_or((line.left(), 0.0), |&(x, w)| (finite(x), finite(w)))).collect();
            let left = spans.iter().map(|c| c.0).fold(f32::INFINITY, f32::min);
            let right = spans.iter().map(|c| c.0 + c.1).fold(f32::NEG_INFINITY, f32::max);
            let (left, right) = if left.is_finite() && right.is_finite() { (left, right) } else { (line.left(), line.left()) };
            let bounds = Rect::from_min_max(Pos2::new(left, line.top()), Pos2::new(right.max(left), line.bottom()));
            let run = Kind::Run {
                text: text[start..end].iter().collect(),
                positions: spans.iter().map(|c| c.0 - left).collect(),
                widths: spans.iter().map(|c| c.1).collect(),
            };
            let at = self.push(Some(parent), run, tables, Some(bounds))?;
            first.get_or_insert(at);
            if end >= text.len() {
                return first;
            }
            start = end;
        }
    }

    /// The text of the runs below `node`, in order.
    fn text_below(&self, node: AccessNode) -> String {
        self.nodes
            .iter()
            .filter(|built| built.parent == Some(node))
            .filter_map(|built| match &built.kind {
                Kind::Run { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Gives every block the union of its children's rectangles.
    fn span_blocks(&mut self) {
        for at in (0..self.nodes.len()).rev() {
            let (Some(bounds), Some(parent)) = (self.nodes[at].bounds, self.nodes[at].parent) else { continue };
            let above = &mut self.nodes[parent.0 as usize];
            above.bounds = Some(above.bounds.map_or(bounds, |b| b.union(bounds)));
        }
    }

    /// The part a node is described at.
    fn part(node: AccessNode) -> PartRef {
        BLOCK.at(u64::from(node.0))
    }
}

fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

/// The app's side: it describes pages of its layout.
pub trait AccessDescriber {
    /// Describe the pages in `placed` (their placement on screen) into `out`, in reading order.
    fn describe(&self, placed: &[PlacedPage], out: &mut AccessBuilder);

    /// The caret and the selection's other end, if the document has a text selection on the described pages.
    fn selection(&self) -> Option<(AccessPosition, AccessPosition)>;
}

/// Describes the pages in view of `pages` and one either side as sub-nodes of the page view's node, and its text
/// selection as the node's. Called from the page view's access step when it has a describer.
pub(crate) fn describe(access: &mut Access, pages: &[PlacedPage], describer: &dyn AccessDescriber) {
    let mut built = AccessBuilder::new();
    describer.describe(&pages[page_window(pages)], &mut built);
    built.span_blocks();
    for at in 0..built.nodes.len() {
        emit(access, &built, AccessNode(at as u32));
    }
    if let Some((caret, other)) = describer.selection() {
        let end = |position: AccessPosition| {
            built.nodes.get(position.run.0 as usize).filter(|node| matches!(node.kind, Kind::Run { .. }))?;
            Some((AccessBuilder::part(position.run), position.offset))
        };
        if let Some(focus) = end(caret) {
            access.text_selection(end(other).unwrap_or(focus), focus);
        }
    }
}

/// Adds the sub-node of `at`.
fn emit(access: &mut Access, built: &AccessBuilder, at: AccessNode) {
    let node = &built.nodes[at.0 as usize];
    let part = AccessBuilder::part(at);
    let table_of = |node: &Built| node.parent.map(|parent| &built.nodes[parent.0 as usize]);
    let sub = match &node.kind {
        Kind::List => access.node(part, Role::LIST),
        Kind::Marker(text) => access.node(part, Role::LIST_MARKER).value(text.clone()),
        Kind::Run { text, positions, widths } => {
            let sub = access.node(part, Role::TEXT_RUN).text_run(text.clone(), positions.clone(), widths.clone(), false);
            if let Some(bounds) = node.bounds {
                sub.bounds(bounds);
            }
            return finish_nested(sub, node);
        }
        Kind::Block(role) => match role {
            BlockRole::Paragraph => access.node(part, Role::PARAGRAPH),
            BlockRole::Heading(level) => access.node(part, Role::HEADING).level(usize::from((*level).clamp(1, 9))),
            BlockRole::ListItem { level, .. } => access.node(part, Role::LIST_ITEM).level(usize::from(*level) + 1),
            BlockRole::Table => {
                let sub = access.node(part, Role::TABLE);
                sub.grid = GridPlace::grid(node.parts, node.columns);
                sub
            }
            BlockRole::Row => {
                let sub = access.node(part, Role::ROW);
                sub.grid.row_index = Some(node.place);
                sub.grid.row_count = table_of(node).map(|table| table.parts);
                sub
            }
            BlockRole::Cell => {
                let sub = access.node(part, Role::CELL);
                let row = table_of(node);
                sub.grid = GridPlace::cell(row.map_or(0, |row| row.place), node.place);
                sub
            }
            BlockRole::Image(alt) => {
                let sub = access.node(part, Role::IMAGE);
                if !alt.trim().is_empty() {
                    sub.name = Name::Text(alt.clone());
                } else {
                    sub.name = Name::Decorative;
                }
                sub
            }
            BlockRole::Link(url) => {
                let label = built.text_below(at);
                let sub = access.node(part, Role::LINK);
                sub.name = Name::Text(if label.is_empty() { url.clone() } else { label });
                sub
            }
        },
    };
    if let Some(bounds) = node.bounds {
        sub.bounds(bounds);
    }
    finish_nested(sub, node);
}

/// Nests `sub` below its parent.
fn finish_nested(sub: &mut craft_ui::contract::SubNode, node: &Built) {
    if let Some(parent) = node.parent {
        sub.under(AccessBuilder::part(parent));
    }
    sub.name = match std::mem::take(&mut sub.name) {
        Name::FromLabel => Name::Text(String::new()),
        other => other,
    };
}

/// The pages meeting the view, and one either side (positions in `pages`): a page is in view while its band, the part
/// of it on screen, is not empty.
fn page_window(pages: &[PlacedPage]) -> Range<usize> {
    let shown = |page: &PlacedPage| page.band.1 > page.band.0;
    let Some(first) = pages.iter().position(shown) else { return 0..0 };
    let last = pages.iter().rposition(shown).unwrap_or(first);
    first.saturating_sub(1)..(last + 2).min(pages.len())
}
