//! AccessKit policy of collections (17.3 step 16; 17.4, 17.5, 17.7): the header cells as `ColumnHeader` sub-nodes
//! and where AccessKit focus goes inside a collection.

use super::emit::{SubDecl, report_sub};
use super::{Name, Role, SubNode, sub_node_id};
use crate::runtime::focus::StopKey;
use craft_behavior::{PartKey, PartRef};
use egui::{Context, Id, Rect};

/// A header cell as step 16 reports it.
pub(crate) struct HeaderCell<'a> {
    pub part: PartRef,
    pub name: &'a str,
    pub rect: Rect,
    /// Activating the cell sorts.
    pub clickable: bool,
    /// The cell holds the collection's keyboard focus.
    pub active: bool,
    /// The column is the sorted one: `true` when it sorts descending.
    pub sort: Option<bool>,
}

/// The `ColumnHeader` sub-nodes of the collection `owner`. Every cell is a focus stop of the header's roving group.
pub(crate) fn report_header(ctx: &Context, owner: Id, enabled: bool, cells: &[HeaderCell<'_>]) {
    for cell in cells {
        let mut node = SubNode::new(cell.part, Role::COLUMN_HEADER);
        node.name = Name::Text(cell.name.to_owned());
        node.sort = cell.sort;
        node.sortable = cell.clickable;
        report_sub(
            ctx,
            &SubDecl {
                owner,
                component: "Collection",
                node: &node,
                rect: cell.rect,
                enabled,
                focusable: enabled,
                clickable: enabled && cell.clickable,
                active: cell.active,
                face: None,
                label: cell.name,
                parent: owner,
            },
        );
    }
}

/// The node AccessKit focus goes to: the focused child or part of the lead row, else the lead row's cell
/// sub-node when the collection selects cells, else the lead row's node.
pub(crate) fn focus_node(row: Id, child: Option<StopKey>, cell: Option<usize>) -> Id {
    match (child, cell) {
        (Some((id, Some(part))), _) => sub_node_id(id, part),
        (Some((id, None)), _) => id,
        (None, Some(column)) => sub_node_id(row, PartKey::CELL.at(column as u64)),
        (None, None) => row,
    }
}
