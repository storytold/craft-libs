//! AccessKit nodes and requests of item rows, section rows and toggle cells.

use super::{ITEM_NODE, ItemsHooks, Placement, SECTION, TOGGLE_CELL, key_hash, section_part};
use crate::contract::collection::{CollectionEvent, LeadSource, OpenChange};
use crate::contract::cx::Pointer as _;
use crate::contract::{Collection, Cx};
use crate::runtime::access::{self, Role};
use crate::runtime::focus;
use craft_behavior::ColumnKey;
use egui::{Context, Id, Rect};

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// The access sub-node of each toggle cell of row `(at, key)`, named by its cell, else by its column's header, and carrying its state,
    /// below the row's node `item`; an AccessKit click on it reaches `toggle_by_access`.
    pub(super) fn report_toggle_cells(
        &self,
        ctx: &Context,
        (id, item): (Id, Id),
        (at, key): (usize, &C::Key),
        (cells, enabled): (&[(ColumnKey, Rect)], bool),
        env: &crate::runtime::env::EnvView<'_>,
    ) {
        for (column, toggle) in self.header.toggle_columns() {
            let Some((_, rect)) = cells.iter().find(|(held, _)| *held == column) else { continue };
            let Some((name, mixed)) = self.header.cell_name(ctx, item, &column, env) else { continue };
            let mut node =
                access::SubNode::new(TOGGLE_CELL.at(key_hash(&(key, &column))), if toggle.exclusive { Role::RADIO_BUTTON } else { Role::SWITCH });
            node.name = access::Name::Text(name);
            node.flags.set(crate::style::Flags::CHECKED, self.source.toggled(at, &column));
            node.flags.set(crate::style::Flags::MIXED, mixed);
            let decl = access::SubDecl {
                owner: id,
                component: "",
                node: &node,
                rect: *rect,
                enabled: enabled && self.source.enabled(at) && self.toggle_open(at, &column),
                focusable: false,
                clickable: true,
                active: false,
                face: None,
                label: "",
                parent: item,
            };
            access::report_sub(ctx, &decl);
        }
    }

    /// The collection's node for the section row `at` that the component does not lay out itself: a heading named by
    /// the row's text, expandable when it has children, that serves Click, Expand and Collapse.
    pub(super) fn report_section_node(&self, ctx: &Context, id: Id, at: usize, rect: Rect, enabled: bool) {
        let mut node = access::SubNode::new(section_part(&self.source.key(at)), Role::HEADING);
        node.name = access::Name::Text(self.source.text(at).map(|text| text.into_owned()).unwrap_or_default());
        node.level(self.source.depth(at) + 1);
        if self.source.has_children(at) {
            node.expanded = Some(self.source.open(at));
        }
        let decl = access::SubDecl {
            owner: id,
            component: "",
            node: &node,
            rect,
            enabled: enabled && self.source.enabled(at),
            focusable: false,
            clickable: true,
            active: false,
            face: None,
            label: "",
            parent: id,
        };
        access::report_sub(ctx, &decl);
    }

    /// AccessKit requests on an item row's node or on a section row's collection node (`report_section_node`): Focus
    /// moves the lead to the row and key focus to the collection; Click does that and picks an item or activates a
    /// section, as a press does; Expand and Collapse open and close a row that has children.
    pub(super) fn node_by_access(&mut self, comp: &C, cx: &mut Cx<'_, C>, layout: &C::ItemLayout, place: Placement) {
        use craft_behavior::AccessAction;
        let (_, built) = self.built(layout, place.rect, place.offset);
        let rows: Vec<(u64, usize)> = built
            .into_iter()
            .filter(|&at| !self.source.section_stop(at) || cx.part_if_declared(section_part(&self.source.key(at))).is_none())
            .map(|at| (key_hash(&self.source.key(at)), at))
            .collect();
        let wanted = |request: &craft_behavior::AccessRequest| {
            request.target.is_some_and(|part| {
                (part.key == ITEM_NODE || part.key == SECTION) && part.index.is_some_and(|hash| rows.iter().any(|(held, _)| *held == hash))
            })
        };
        while let Some(request) = cx.core.intents.take_access_where(wanted) {
            let Some(at) = request.target.and_then(|part| part.index).and_then(|hash| rows.iter().find(|(held, _)| *held == hash)).map(|(_, at)| *at)
            else {
                continue;
            };
            if !self.source.enabled(at) {
                continue;
            }
            let key = self.source.key(at);
            match request.action {
                AccessAction::Focus | AccessAction::Click => {
                    if self.lead.key.as_ref() != Some(&key) {
                        self.move_lead(at, LeadSource::Keys);
                    }
                    focus::request_now(&cx.core.ctx, focus::FocusTarget::Id(cx.core.id));
                    if request.action == AccessAction::Click {
                        if self.source.section_stop(at) {
                            self.events.push(CollectionEvent::Activate(key));
                        } else {
                            self.pick(comp, at, false, false);
                        }
                    }
                }
                AccessAction::Expand | AccessAction::Collapse => {
                    let open = request.action == AccessAction::Expand;
                    if self.source.has_children(at) && self.source.open(at) != open {
                        self.events.push(CollectionEvent::Open(OpenChange::new(key, open)));
                    }
                }
                _ => {}
            }
        }
    }

    /// An AccessKit click on a toggle cell's sub-node flips that cell, as a click on it does.
    pub(super) fn toggle_by_access(&mut self, cx: &mut Cx<'_, C>, layout: &C::ItemLayout, place: Placement) {
        let wanted = |request: &craft_behavior::AccessRequest| {
            request.action == craft_behavior::AccessAction::Click && request.target.is_some_and(|part| part.key == TOGGLE_CELL)
        };
        while let Some(request) = cx.core.intents.take_access_where(wanted) {
            let Some(part) = request.target else { continue };
            let (_, built) = self.built(layout, place.rect, place.offset);
            let found = built.into_iter().find_map(|at| {
                let key = self.source.key(at);
                self.header
                    .toggle_columns()
                    .find(|(column, _)| TOGGLE_CELL.at(key_hash(&(&key, column))) == part)
                    .map(|(column, toggle)| (at, column, toggle))
            });
            if let Some((at, column, toggle)) = found {
                self.flip_toggle(cx, at, column, toggle);
            }
        }
    }
}
