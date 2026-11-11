//! Writes a decided node into AccessKit's node builder: the one place that names its setters.

use super::NodeRole;
use crate::runtime::access::{Numeric, Politeness, PopupKind};
use egui::{Context, Id, Rect};

/// The checked state of a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Toggle {
    Off,
    On,
    Mixed,
}

/// An action a node declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeAction {
    Focus,
    Click,
    Increment,
    Decrement,
    SetValue,
    Expand,
    Collapse,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
    SetScrollOffset,
}

impl NodeAction {
    fn action(self) -> accesskit::Action {
        match self {
            NodeAction::Focus => accesskit::Action::Focus,
            NodeAction::Click => accesskit::Action::Click,
            NodeAction::Increment => accesskit::Action::Increment,
            NodeAction::Decrement => accesskit::Action::Decrement,
            NodeAction::SetValue => accesskit::Action::SetValue,
            NodeAction::Expand => accesskit::Action::Expand,
            NodeAction::Collapse => accesskit::Action::Collapse,
            NodeAction::ScrollUp => accesskit::Action::ScrollUp,
            NodeAction::ScrollDown => accesskit::Action::ScrollDown,
            NodeAction::ScrollLeft => accesskit::Action::ScrollLeft,
            NodeAction::ScrollRight => accesskit::Action::ScrollRight,
            NodeAction::SetScrollOffset => accesskit::Action::SetScrollOffset,
        }
    }
}

/// Everything one node says, decided by `runtime::access`.
pub(crate) struct NodeSpec {
    pub role: NodeRole,
    pub rect: Option<Rect>,
    pub hidden: bool,
    pub label: Option<String>,
    pub description: Option<String>,
    pub placeholder: Option<String>,
    pub value: Option<String>,
    pub color: Option<craft_theme::Srgba>,
    pub url: Option<String>,
    pub shortcut: Option<String>,
    pub access_key: Option<char>,
    pub numeric: Option<Numeric>,
    pub expanded: Option<bool>,
    pub has_popup: Option<PopupKind>,
    /// The current item of its set (the current page).
    pub current: bool,
    /// The index in the set and the size of the set.
    pub position: Option<(usize, Option<usize>)>,
    pub level: Option<usize>,
    pub grid: crate::runtime::access::GridPlace,
    /// The header cell's sort order: `true` descending.
    pub sort: Option<bool>,
    /// The header sorts when activated: it declares a sort direction while unsorted (`Other`).
    pub sortable: bool,
    pub labelled_by: Vec<Id>,
    pub described_by: Vec<Id>,
    pub controls: Vec<Id>,
    pub flow_to: Vec<Id>,
    pub live: Option<Politeness>,
    pub toggled: Option<Toggle>,
    pub selected: Option<bool>,
    pub invalid: bool,
    pub busy: bool,
    pub disabled: bool,
    pub read_only: bool,
    pub required: bool,
    pub modal: bool,
    pub actions: Vec<NodeAction>,
    pub custom_actions: Vec<(i32, String)>,
    /// The content keeps its left-to-right order under a right-to-left interface.
    pub ltr_axis: bool,
}

impl NodeSpec {
    /// The root node of an overlay layer.
    pub(crate) fn layer_root() -> Self {
        NodeSpec::new(accesskit::Role::GenericContainer)
    }

    pub(crate) fn new(role: NodeRole) -> Self {
        NodeSpec {
            role,
            rect: None,
            hidden: false,
            label: None,
            description: None,
            placeholder: None,
            value: None,
            color: None,
            url: None,
            shortcut: None,
            access_key: None,
            numeric: None,
            expanded: None,
            has_popup: None,
            current: false,
            position: None,
            level: None,
            grid: crate::runtime::access::GridPlace::default(),
            sort: None,
            sortable: false,
            labelled_by: Vec::new(),
            described_by: Vec::new(),
            controls: Vec::new(),
            flow_to: Vec::new(),
            live: None,
            toggled: None,
            selected: None,
            invalid: false,
            busy: false,
            disabled: false,
            read_only: false,
            required: false,
            modal: false,
            actions: Vec::new(),
            custom_actions: Vec::new(),
            ltr_axis: false,
        }
    }
}

fn ids(ids: &[Id]) -> Vec<accesskit::NodeId> {
    ids.iter().map(|id| id.accesskit_id()).collect()
}

fn bounds(rect: Rect) -> accesskit::Rect {
    accesskit::Rect { x0: f64::from(rect.min.x), y0: f64::from(rect.min.y), x1: f64::from(rect.max.x), y1: f64::from(rect.max.y) }
}

fn write_numeric(node: &mut accesskit::Node, numeric: &Numeric) {
    if let Some(value) = numeric.value {
        node.set_numeric_value(value);
    }
    node.set_min_numeric_value(*numeric.range.start());
    node.set_max_numeric_value(*numeric.range.end());
    if let Some(step) = numeric.step {
        node.set_numeric_value_step(step);
    }
    if let Some(text) = &numeric.text {
        node.set_value(text.clone());
    }
}

fn write_identity(node: &mut accesskit::Node, spec: &NodeSpec) {
    node.set_role(spec.role);
    if let Some(rect) = spec.rect {
        node.set_bounds(bounds(rect));
    }
    if spec.hidden {
        node.set_hidden();
    }
    if let Some(text) = &spec.label {
        node.set_label(text.clone());
    }
    if let Some(text) = &spec.description {
        node.set_description(text.clone());
    }
    if let Some(text) = &spec.placeholder {
        node.set_placeholder(text.clone());
    }
    if let Some(text) = &spec.value {
        node.set_value(text.clone());
    }
    if let Some(color) = spec.color {
        let [red, green, blue, alpha] = color.to_rgba().0;
        node.set_color_value(accesskit::Color { red, green, blue, alpha });
    }
    if let Some(numeric) = &spec.numeric {
        write_numeric(node, numeric);
    }
    if let Some(url) = &spec.url {
        node.set_url(url.clone());
    }
    if let Some(shortcut) = &spec.shortcut {
        node.set_keyboard_shortcut(shortcut.clone());
    }
    if let Some(key) = spec.access_key {
        node.set_access_key(key.to_string());
    }
    if let Some((index, size)) = spec.position {
        node.set_position_in_set(index);
        if let Some(size) = size {
            node.set_size_of_set(size);
        }
    }
    if let Some(level) = spec.level {
        node.set_level(level);
    }
    let grid = &spec.grid;
    if let Some(index) = grid.row_index {
        node.set_row_index(index);
    }
    if let Some(count) = grid.row_count {
        node.set_row_count(count);
    }
    if let Some(index) = grid.column_index {
        node.set_column_index(index);
    }
    if let Some(count) = grid.column_count {
        node.set_column_count(count);
    }
    if let Some(descending) = spec.sort {
        node.set_sort_direction(if descending { accesskit::SortDirection::Descending } else { accesskit::SortDirection::Ascending });
    } else if spec.sortable {
        node.set_sort_direction(accesskit::SortDirection::Other);
    }
}

fn write_relations(node: &mut accesskit::Node, spec: &NodeSpec) {
    if !spec.labelled_by.is_empty() {
        node.set_labelled_by(ids(&spec.labelled_by));
    }
    if !spec.described_by.is_empty() {
        node.set_described_by(ids(&spec.described_by));
    }
    if !spec.controls.is_empty() {
        node.set_controls(ids(&spec.controls));
    }
    if !spec.flow_to.is_empty() {
        node.set_flow_to(ids(&spec.flow_to));
    }
    if let Some(popup) = spec.has_popup {
        node.set_has_popup(match popup {
            PopupKind::Menu => accesskit::HasPopup::Menu,
            PopupKind::Listbox => accesskit::HasPopup::Listbox,
            PopupKind::Tree => accesskit::HasPopup::Tree,
            PopupKind::Grid => accesskit::HasPopup::Grid,
            PopupKind::Dialog => accesskit::HasPopup::Dialog,
        });
    }
}

fn write_states(node: &mut accesskit::Node, spec: &NodeSpec) {
    if let Some(expanded) = spec.expanded {
        node.set_expanded(expanded);
    }
    if spec.current {
        node.set_aria_current(accesskit::AriaCurrent::Page);
    }
    if let Some(politeness) = spec.live {
        node.set_live(match politeness {
            Politeness::Polite => accesskit::Live::Polite,
            Politeness::Assertive => accesskit::Live::Assertive,
        });
    }
    if let Some(toggled) = spec.toggled {
        node.set_toggled(match toggled {
            Toggle::Off => accesskit::Toggled::False,
            Toggle::On => accesskit::Toggled::True,
            Toggle::Mixed => accesskit::Toggled::Mixed,
        });
    }
    if let Some(selected) = spec.selected {
        node.set_selected(selected);
    }
    if spec.invalid {
        node.set_invalid(accesskit::Invalid::True);
    }
    if spec.ltr_axis {
        node.set_text_direction(accesskit::TextDirection::LeftToRight);
    }
    for (on, set) in [
        (spec.busy, accesskit::Node::set_busy as fn(&mut accesskit::Node)),
        (spec.disabled, accesskit::Node::set_disabled),
        (spec.read_only, accesskit::Node::set_read_only),
        (spec.required, accesskit::Node::set_required),
        (spec.modal, accesskit::Node::set_modal),
    ] {
        if on {
            set(node);
        }
    }
}

/// The actions the spec declares replace the ones egui declared for the widget's sense: a root whose widget senses
/// its parts' clicks does not itself take a click.
fn write_actions(node: &mut accesskit::Node, spec: &NodeSpec) {
    node.clear_actions();
    for action in &spec.actions {
        node.add_action(action.action());
    }
    if !spec.custom_actions.is_empty() {
        node.add_action(accesskit::Action::CustomAction);
        node.set_custom_actions(
            spec.custom_actions
                .iter()
                .map(|(id, description)| accesskit::CustomAction { id: *id, description: description.clone() })
                .collect::<Vec<_>>(),
        );
    }
}

/// Writes `spec` into the node of `id`, creating it. Nothing while AccessKit is off or the widget is invisible.
pub(crate) fn write_node(ctx: &Context, id: Id, spec: &NodeSpec) {
    crate::runtime::egui::widgets::note_node(ctx, id);
    super::note_written(ctx, id);
    ctx.accesskit_node_builder(id, |node| {
        write_identity(node, spec);
        write_relations(node, spec);
        write_states(node, spec);
        write_actions(node, spec);
    });
}
