//! `Role`, the access facts of a component and the AccessKit emitter (chapter 12).
//!
//! This module decides what every node says (name, description, actions, rule checks, scope registration) and is
//! the only one that calls `runtime::egui::access`, which writes nodes and makes the parent and focus calls. One
//! `report` per node kind fills a node; `runtime::access::{item, text}` add theirs beside `report` and `report_sub`.

mod emit;
pub(crate) mod fold;
pub(crate) mod items;
mod live;
pub(crate) mod scope;
pub(crate) mod text;

use crate::runtime::egui::access::{self as adapter, NodeRole, RunLink};
use crate::runtime::env::Label;
use craft_behavior::PartRef;
use craft_theme::{Flags, Srgba, variant::Tone};
use egui::Id;
use std::ops::RangeInclusive;

#[doc(hidden)]
pub use emit::SubDecl;
#[doc(hidden)]
pub use emit::report_sub;
#[doc(hidden)]
pub use emit::sub_node_id;
pub(crate) use emit::{FaceFacts, Own, PopupShown, report, report_layer};
pub use live::announce;
pub(crate) use live::{begin_pass as begin_live_pass, end_pass as end_live_pass};
pub use scope::{FieldLabel, FieldLabelRole};

/// Makes `parent` the node `id` attaches to. Call it before the node of `id` is created.
pub(crate) fn register_parent(ctx: &egui::Context, id: Id, parent: Id) {
    adapter::register_parent(ctx, id, parent);
}

/// `builder` with `parent` as the node its `Ui`'s AccessKit nodes attach to.
pub(crate) fn with_parent(builder: egui::UiBuilder, parent: Id) -> egui::UiBuilder {
    adapter::with_parent(builder, parent)
}

/// Creates the node of `id` empty if it has none. False while AccessKit is off or the widget is invisible (a
/// popup in its first-show sizing pass).
pub(crate) fn create_node(ctx: &egui::Context, id: Id) -> bool {
    adapter::create_node(ctx, id)
}

/// Keeps the node egui gives the widget `id` out of the screen-reader tree: a widget that senses input and stands for
/// no node (a `Role::None` root, a cover's absorber).
pub(crate) fn hide(ctx: &egui::Context, id: Id) {
    adapter::hide(ctx, id);
}

/// Reports `id` as the focused node of this pass.
pub(crate) fn set_focus(ctx: &egui::Context, id: Id) {
    adapter::set_focus(ctx, id);
}

/// What a component is to assistive technology.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Role {
    /// No node.
    None,
    /// The node merges into its container's node.
    Merged,
    /// A node of this AccessKit role.
    Node(NodeRole),
}

impl Role {
    /// The role of a message of `tone`: status for the neutral tones, alert for a warning or a failure.
    pub const fn of_tone(tone: Tone) -> Role {
        match tone {
            Tone::Warn | Tone::Bad => Role::ALERT,
            _ => Role::STATUS,
        }
    }
}

/// How a node or sub-node is named.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Name {
    /// The component's label (an enclosing field label replaces it).
    #[default]
    FromLabel,
    /// This text.
    Text(String),
    /// These texts, joined with a comma.
    Joined(Vec<String>),
    /// The text of the node with this id.
    LabelledBy(Id),
    /// No name: the node is hidden from assistive technology.
    Decorative,
}

/// A numeric value.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Numeric {
    /// The value, `None` while it has none (a mixed selection).
    pub value: Option<f64>,
    /// The range; a slider needs a finite one.
    pub range: RangeInclusive<f64>,
    /// The step of an increment or decrement; it declares each action that can move the value: Increment below the
    /// range's end and Decrement above its start, or both when the value wraps or is mixed.
    pub step: Option<f64>,
    /// Whether a step past one end of the range continues from the other.
    pub wraps: bool,
    /// The value as the component words it ("40.0", "50 %"); assistive technology reads it in place of the number.
    pub text: Option<String>,
}

impl Numeric {
    /// A value in a range.
    pub fn new(value: f64, range: RangeInclusive<f64>) -> Self {
        Numeric { value: Some(value), range, step: None, wraps: false, text: None }
    }

    /// A range with no value.
    pub fn empty(range: RangeInclusive<f64>) -> Self {
        Numeric { value: None, range, step: None, wraps: false, text: None }
    }

    /// The step of an increment or decrement.
    #[must_use]
    pub fn step(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }

    /// Whether a step past one end of the range continues from the other.
    #[must_use]
    pub fn wraps(mut self, wraps: bool) -> Self {
        self.wraps = wraps;
        self
    }

    /// The value as shown.
    #[must_use]
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }
}

/// A node's place in its set, on the closed list of items it belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SetPosition {
    /// The zero-based index.
    pub index: usize,
    /// The number of items, when known.
    pub size: Option<usize>,
    /// The depth in a tree.
    pub level: Option<usize>,
    /// The number of columns, when the item is a row of a grid.
    pub columns: Option<usize>,
}

impl SetPosition {
    /// Position `index`.
    pub const fn new(index: usize) -> Self {
        SetPosition { index, size: None, level: None, columns: None }
    }

    /// In a grid of `columns` columns.
    #[must_use]
    pub const fn columns(mut self, columns: usize) -> Self {
        self.columns = Some(columns);
        self
    }

    /// In a set of `size`.
    #[must_use]
    pub const fn of(mut self, size: usize) -> Self {
        self.size = Some(size);
        self
    }

    /// At depth `level`.
    #[must_use]
    pub const fn level(mut self, level: usize) -> Self {
        self.level = Some(level);
        self
    }
}

/// The indexes and counts of a grid, table, row or cell node (12.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct GridPlace {
    /// The zero-based row.
    pub row_index: Option<usize>,
    /// The number of rows.
    pub row_count: Option<usize>,
    /// The zero-based column.
    pub column_index: Option<usize>,
    /// The number of columns.
    pub column_count: Option<usize>,
}

impl GridPlace {
    /// A grid of `rows` rows and `columns` columns.
    pub const fn grid(rows: usize, columns: usize) -> Self {
        GridPlace { row_index: None, row_count: Some(rows), column_index: None, column_count: Some(columns) }
    }

    /// The cell at `row` and `column`.
    pub const fn cell(row: usize, column: usize) -> Self {
        GridPlace { row_index: Some(row), row_count: None, column_index: Some(column), column_count: None }
    }

    /// The facts a rule reads.
    pub(crate) const fn facts(&self) -> craft_behavior::GridFacts {
        craft_behavior::GridFacts::new()
            .row_index(self.row_index.is_some())
            .row_count(self.row_count.is_some())
            .column_index(self.column_index.is_some())
            .column_count(self.column_count.is_some())
    }
}

/// How urgently a live announcement is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Politeness {
    /// After the current speech.
    Polite,
    /// At once.
    Assertive,
}

/// The kind of popup a component opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PopupKind {
    /// A menu.
    Menu,
    /// A list box.
    Listbox,
    /// A tree.
    Tree,
    /// A grid.
    Grid,
    /// A dialog.
    Dialog,
}

/// Nodes a node relates to, by id.
#[derive(Clone, Debug, Default)]
pub(crate) struct Relations {
    pub labelled_by: Vec<Id>,
    pub described_by: Vec<Id>,
    pub controls: Vec<Id>,
    pub flow_to: Vec<Id>,
}

/// The facts a component adds to its AccessKit node: what the pipeline cannot derive from the label, the flags
/// and the parts.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Access {
    pub(crate) role: Option<Role>,
    pub(crate) name: Name,
    pub(crate) description: Option<String>,
    pub(crate) placeholder: Option<String>,
    pub(crate) value: Option<String>,
    pub(crate) color: Option<Srgba>,
    pub(crate) numeric: Option<Numeric>,
    pub(crate) expanded: Option<bool>,
    pub(crate) has_popup: Option<PopupKind>,
    pub(crate) level: Option<usize>,
    pub(crate) position: Option<SetPosition>,
    pub(crate) grid: GridPlace,
    pub(crate) required: bool,
    pub(crate) url: Option<String>,
    pub(crate) shortcut: Option<String>,
    pub(crate) access_key: Option<char>,
    pub(crate) relations: Relations,
    pub(crate) custom_actions: Vec<(i32, Label)>,
    pub(crate) live: Option<Politeness>,
    pub(crate) busy: bool,
    pub(crate) pans: (bool, bool),
    pub(crate) nodes: Vec<SubNode>,
    pub(crate) text_selection: Option<(TextEnd, TextEnd)>,
}

impl Access {
    /// The role of this instance, one of the component's `ALT_ROLES`.
    pub fn role(&mut self, role: Role) -> &mut Self {
        self.role = Some(role);
        self
    }

    /// How the node is named, when the label is not the name.
    pub fn name(&mut self, name: Name) -> &mut Self {
        self.name = name;
        self
    }

    /// A description, over the bound command's hint and the tooltip.
    pub fn description(&mut self, text: impl Into<String>) -> &mut Self {
        self.description = Some(text.into());
        self
    }

    /// The hint text of an empty input.
    pub fn placeholder(&mut self, text: impl Into<String>) -> &mut Self {
        self.placeholder = Some(text.into());
        self
    }

    /// The text value of a text component.
    pub fn value(&mut self, text: impl Into<String>) -> &mut Self {
        self.value = Some(text.into());
        self
    }

    /// The colour value of a colour well.
    pub fn color(&mut self, color: Srgba) -> &mut Self {
        self.color = Some(color);
        self
    }

    /// The numeric value of a value component; a step also declares increment and decrement.
    pub fn numeric(&mut self, numeric: Numeric) -> &mut Self {
        self.numeric = Some(numeric);
        self
    }

    /// Whether a disclosure is open, when no flag says it.
    pub fn expanded(&mut self, on: bool) -> &mut Self {
        self.expanded = Some(on);
        self
    }

    /// What kind of popup the component opens.
    pub fn has_popup(&mut self, kind: PopupKind) -> &mut Self {
        self.has_popup = Some(kind);
        self
    }

    /// The heading level or tree depth.
    pub fn level(&mut self, level: usize) -> &mut Self {
        self.level = Some(level);
        self
    }

    /// The row and column counts of a grid or table node. A collection's item step fills them for its own node.
    pub fn grid(&mut self, grid: GridPlace) -> &mut Self {
        self.grid = grid;
        self
    }

    /// The place in a closed set.
    pub fn position(&mut self, position: SetPosition) -> &mut Self {
        self.position = Some(position);
        self
    }

    /// The input must be filled in.
    pub fn required(&mut self, on: bool) -> &mut Self {
        self.required = on;
        self
    }

    /// The target of a link.
    pub fn url(&mut self, url: impl Into<String>) -> &mut Self {
        self.url = Some(url.into());
        self
    }

    /// The keyboard shortcut, over the bound command's.
    pub fn shortcut(&mut self, text: impl Into<String>) -> &mut Self {
        self.shortcut = Some(text.into());
        self
    }

    /// The mnemonic letter.
    pub fn access_key(&mut self, key: char) -> &mut Self {
        self.access_key = Some(key);
        self
    }

    /// The node `id` names this one. Targets are `cx.key(..)` ids; one shown in a later frame follows the
    /// one-frame rule.
    pub fn labelled_by(&mut self, id: Id) -> &mut Self {
        self.relations.labelled_by.push(id);
        self
    }

    /// The node `id` describes this one.
    pub fn described_by(&mut self, id: Id) -> &mut Self {
        self.relations.described_by.push(id);
        self
    }

    /// This node controls the node `id` (the popup of an open combo box).
    pub fn controls(&mut self, id: Id) -> &mut Self {
        self.relations.controls.push(id);
        self
    }

    /// Reading continues at the node `id`.
    pub fn flow_to(&mut self, id: Id) -> &mut Self {
        self.relations.flow_to.push(id);
        self
    }

    /// Makes the node a live region.
    pub fn live(&mut self, politeness: Politeness) -> &mut Self {
        self.live = Some(politeness);
        self
    }

    /// The leaf pans its content along the vertical and horizontal axes: the node serves the scroll actions on them,
    /// as a scrolling body does. A pan is the leaf's own view change, not a value step.
    pub fn pans(&mut self, vertical: bool, horizontal: bool) -> &mut Self {
        self.pans = (vertical, horizontal);
        self
    }

    /// The node is working.
    pub fn busy(&mut self, on: bool) -> &mut Self {
        self.busy = on;
        self
    }

    /// A node below this component's node, keyed by a `PartRef` (content painters key lines and runs as
    /// `LINE.at(i)`), with the id `sub(id, ("node", part_ref))`.
    pub fn node(&mut self, part: impl Into<PartRef>, role: Role) -> &mut SubNode {
        self.nodes.push(SubNode::new(part.into(), role));
        let at = self.nodes.len() - 1;
        &mut self.nodes[at]
    }

    /// The text selection of the component's own text, from `anchor` to `focus`: each end is a text-run sub-node
    /// ([`SubNode::text_run`]) of this component and a character index in the run.
    pub fn text_selection(&mut self, anchor: (PartRef, usize), focus: (PartRef, usize)) -> &mut Self {
        self.text_selection = Some((TextEnd { part: anchor.0, character: anchor.1 }, TextEnd { part: focus.0, character: focus.1 }));
        self
    }

    /// An action the component serves itself; it arrives through `cx.intents().take_access()`.
    pub fn custom_action(&mut self, id: i32, label: Label) -> &mut Self {
        self.custom_actions.push((id, label));
        self
    }
}

/// One end of a text selection: a text-run sub-node and a character index in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextEnd {
    pub(crate) part: PartRef,
    pub(crate) character: usize,
}

/// What a text-run sub-node says about its characters.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RunFacts {
    pub(crate) text: String,
    /// Each character's start offset from the run's left edge, and its advance.
    pub(crate) positions: Vec<f32>,
    pub(crate) widths: Vec<f32>,
    pub(crate) rtl: bool,
}

/// A node below a component's node, for a part, a cell or a line of a content painter.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SubNode {
    /// How the sub-node is named.
    pub name: Name,
    /// A description.
    pub description: Option<String>,
    /// A numeric value: the value, its range and, with a step, the Increment and Decrement actions; SetValue is
    /// declared with any numeric value. The requests reach the component as they do for its own node: a focusable
    /// part gets the arrow `take_move` on its active part and `take_set_value_at(part)`; a part with no focus stop gets
    /// `take_access` requests that name the part.
    pub numeric: Option<Numeric>,
    /// A plain text value, for a sub-node that has one and no number.
    pub(crate) value: Option<String>,
    /// Whether the node's popup or region is open; the node is expandable when this or its face says.
    pub expanded: Option<bool>,
    /// The place in a closed set.
    pub position: Option<SetPosition>,
    /// The indexes of a cell.
    pub grid: GridPlace,
    /// The heading level or tree depth.
    pub level: Option<usize>,
    pub(crate) sort: Option<bool>,
    /// The column header sorts when activated: the node declares a sort direction even while the column is unsorted.
    pub(crate) sortable: bool,
    /// The column of a cell, which the item step turns into the cell's column index.
    pub(crate) column: Option<craft_behavior::ColumnKey>,
    pub(crate) access_key: Option<char>,
    pub(crate) shortcut: Option<String>,
    pub(crate) has_popup: Option<PopupKind>,
    pub(crate) current: bool,
    pub(crate) part: PartRef,
    pub(crate) role: Role,
    pub(crate) under: Option<PartRef>,
    #[doc(hidden)]
    pub flags: Flags,
    pub(crate) custom_actions: Vec<(i32, Label)>,
    pub(crate) bounds: Option<egui::Rect>,
    pub(crate) run: Option<RunFacts>,
    pub(crate) run_links: Vec<RunLink>,
}

impl SubNode {
    #[doc(hidden)]
    pub fn new(part: PartRef, role: Role) -> Self {
        SubNode {
            name: Name::FromLabel,
            description: None,
            numeric: None,
            value: None,
            expanded: None,
            position: None,
            grid: GridPlace::default(),
            level: None,
            sort: None,
            sortable: false,
            column: None,
            access_key: None,
            shortcut: None,
            has_popup: None,
            current: false,
            part,
            role,
            under: None,
            flags: Flags::empty(),
            custom_actions: Vec::new(),
            bounds: None,
            run: None,
            run_links: Vec::new(),
        }
    }

    /// The node's rectangle on screen, for a node that has no part of its own to take it from (a block of text a
    /// content painter draws).
    pub fn bounds(&mut self, rect: egui::Rect) -> &mut Self {
        self.bounds = Some(rect);
        self
    }

    /// Makes the node a run of text: `text` laid out on one line, each character's start offset from the node's left
    /// edge in `positions` and its advance in `widths` (one per character), `rtl` for a right-to-left run. The node
    /// is a `TextRun` whatever role it was given, takes its [`bounds`](Self::bounds) as the line's rectangle, and is
    /// a valid end of [`Access::text_selection`].
    pub fn text_run(&mut self, text: impl Into<String>, positions: Vec<f32>, widths: Vec<f32>, rtl: bool) -> &mut Self {
        self.run = Some(RunFacts { text: text.into(), positions, widths, rtl });
        self
    }

    /// Marks characters `chars` of a [text run](Self::text_run) as a link to `target`: the run node gets a `Link`
    /// child named by those characters, bounded by their extent on the line. Indices count characters, as
    /// `positions` and `widths` do. A link on a node that is not a run is not emitted.
    pub fn run_link(&mut self, chars: std::ops::Range<usize>, target: impl Into<String>) -> &mut Self {
        self.run_links.push(RunLink { chars, target: target.into() });
        self
    }

    /// The heading level or tree depth.
    pub fn level(&mut self, level: usize) -> &mut Self {
        self.level = Some(level);
        self
    }

    /// A plain text value, for a sub-node that shows text as its value and has no number.
    pub fn value(&mut self, text: impl Into<String>) -> &mut Self {
        self.value = Some(text.into());
        self
    }

    /// The column of a cell of an item row; the item step reads the cell's column index from the collection's
    /// columns.
    pub fn column(&mut self, key: craft_behavior::ColumnKey) -> &mut Self {
        self.column = Some(key);
        self
    }

    /// The mnemonic letter.
    pub fn access_key(&mut self, key: char) -> &mut Self {
        self.access_key = Some(key);
        self
    }

    /// The key that activates the sub-node, as the access tree lists it (`Access::shortcut` for the component's own
    /// node): a header cell that opens its menu with Alt+Down says `"Alt+Down"`. The key acts through the
    /// component's key use, which delivers it to the focused part.
    pub fn shortcut(&mut self, text: impl Into<String>) -> &mut Self {
        self.shortcut = Some(text.into());
        self
    }

    /// What kind of popup the sub-node opens.
    pub fn has_popup(&mut self, kind: PopupKind) -> &mut Self {
        self.has_popup = Some(kind);
        self
    }

    /// Whether the sub-node's popup or region is open.
    pub fn expanded(&mut self, on: bool) -> &mut Self {
        self.expanded = Some(on);
        self
    }

    /// The sub-node is the current one of its set: the current page of a trail.
    pub fn current(&mut self, on: bool) -> &mut Self {
        self.current = on;
        self
    }

    /// Nests this sub-node below another sub-node of the same component.
    pub fn under(&mut self, other: impl Into<PartRef>) -> &mut Self {
        self.under = Some(other.into());
        self
    }

    /// An action the component serves itself, targeted at this sub-node.
    pub fn custom_action(&mut self, id: i32, label: Label) -> &mut Self {
        self.custom_actions.push((id, label));
        self
    }
}
