//! Items that are members of the collection's run (`Collection::MEMBERS`): sensed in the collection's arena,
//! painted as faces, reported as sub-nodes of the collection.

use super::{ITEM_NODE, ItemsHooks, key_hash};
use crate::contract::collection::ShownRow;
use crate::contract::run::{self, FaceSource};
use crate::contract::tooltip::MemberTip;
use crate::contract::{Collection, Component as _, Cx, Internal, ItemOpener, Meta as _};
use crate::primitives::PartResult;
use crate::primitives::sensing::{self, MemberSpec};
use crate::runtime::access::{self, Name, SetPosition, SubDecl, SubNode};
use crate::runtime::env::{ArenaId, Env};
use crate::style::Flags;
use craft_behavior::Sense;
use egui::{Context, Id, Rect, Ui};

/// A visible item of a members collection, built by the row loop and shown once every member has joined the arena.
pub(super) struct Member<K, I> {
    pub at: usize,
    pub key: K,
    pub item: I,
    pub row: Rect,
    /// The row's clip: below the frozen rows.
    pub clip: Rect,
    /// The row's flags: selected, lead, expanded, current, dragging, drop target.
    pub flags: Flags,
    pub position: SetPosition,
    pub renaming: bool,
    pub openers: Vec<ItemOpener>,
    /// The member's place in the pass's shown rows.
    pub shown: usize,
}

impl<'r, C: Collection> ItemsHooks<'_, 'r, C> {
    /// Joins the members to the collection's arena in one read, then paints each as the face of its item part,
    /// reports its sub-node, and leaves the hovered member's tooltip for step 16.
    pub(super) fn show_members(
        &mut self,
        comp: &mut C,
        ui: &mut Ui,
        cx: &mut Cx<'_, C>,
        ((ctx, id, rtl), env): ((&Context, Id, bool), &Env),
        mut members: Vec<Member<C::Key, C::Item<'r>>>,
        shown_rows: &mut [ShownRow<C::Key>],
    ) {
        let enabled = cx.core.resolved.enabled;
        let parent = access::fold::owner_of(ctx, id).unwrap_or(id);
        let facts: Vec<(bool, Sense, Option<crate::contract::Tooltip>)> = members
            .iter_mut()
            .map(|member| {
                let common = member.item.common_mut(Internal::new());
                let on = enabled && common.enabled.unwrap_or(true);
                let tip = common.tooltip.take();
                (on, member.item.sense(), tip)
            })
            .collect();
        let specs: Vec<MemberSpec<'_>> = members
            .iter()
            .zip(&facts)
            // A member senses the pointer over it, never the press: the collection's root takes the click and the
            // focus (17.2), as for an item that runs.
            .map(|(member, (on, _, _))| MemberSpec { id: item_id(id, &member.key), rect: member.row, sense: Sense::HOVER, parts: &[], enabled: *on })
            .collect();
        let sensed = sensing::sense_members(ui, ArenaId::new(id), &specs);
        let clip = ui.clip_rect();
        for ((member, (on, sense, tip)), sensed) in members.into_iter().zip(facts).zip(sensed) {
            let Member { at, key, item, row, clip: row_clip, flags, position, renaming, openers, shown } = member;
            let part = ITEM_NODE.at(key_hash(&key));
            let hovered = sensed.response.hovered();
            let result = PartResult { rect: row, press: sensed.press, hovered, hover_pos: None, flags, inactive: false };
            let source = FaceSource {
                id,
                result: &result,
                focus: None,
                response: &sensed.response,
                ink: None,
                muted: None,
                flags: Flags::empty(),
                enabled: on,
            };
            ui.set_clip_rect(row_clip);
            let (record, facts) = run::paint_face(ui, env, &source, part, item);
            ui.set_clip_rect(clip);
            let mut node = SubNode::new(part, facts.role.unwrap_or(record.role));
            node.name = Name::Text(record.name);
            node.value = facts.value;
            node.color = facts.color;
            node.position = Some(position);
            node.flags = flags;
            let decl = SubDecl {
                owner: id,
                component: "",
                node: &node,
                rect: row,
                enabled: on,
                focusable: on && sense.contains(Sense::FOCUS),
                clickable: on && sense.contains(Sense::CLICK),
                active: false,
                face: None,
                label: "",
                parent,
            };
            access::report_sub(ctx, &decl);
            if hovered && let Some(tip) = tip {
                cx.core.member_tip = Some(MemberTip { part, spec: tip.spec().clone(), rect: row, pressed: sensed.press.phase.is_press() });
            }
            if let Some(shown) = shown_rows.get_mut(shown) {
                shown.openers = self.item_openers(ui, ctx, id, enabled, openers);
            }
            if renaming {
                self.rename_row(comp, ui, (ctx, id, rtl), (at, &key), row);
            }
        }
    }
}

/// The id of the item `key` of the collection `id`: a member's owner id in the arena, as a run item's id.
fn item_id<K: std::hash::Hash>(id: Id, key: &K) -> Id {
    crate::runtime::memory::sub(id, ("key", key_hash(key)))
}

/// The access node a members collection's lead item stands for: the sub-node of its item part.
pub(super) fn lead_node<K: std::hash::Hash>(id: Id, key: &K) -> Id {
    access::sub_node_id(id, ITEM_NODE.at(key_hash(key)))
}
