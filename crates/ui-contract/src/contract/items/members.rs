//! Items that are members of the collection's run (`Collection::MEMBERS`): sensed with their parts in the collection's
//! arena, updated, painted and finished at their rows, reported as sub-nodes of the collection.

use super::{ITEM_NODE, ItemsHooks, key_hash};
use crate::contract::collection::{CollectionEvent, ContextMenuRequest, OpenChange, ShownRow};
use crate::contract::run;
use crate::contract::{Collection, Component as _, Cx, Internal, ItemOpener, ItemOutput as _, Meta as _};
use crate::primitives::sensing::{self, MemberSpec};
use crate::runtime::access::{self, SetPosition};
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
    /// Runs each member's parts step, joins the members to the collection's arena in one read, then runs each member's
    /// update, paint, report and finish at its row; leaves the hovered member's tooltip for step 16 and reports its
    /// output as the collection's item events.
    pub(super) fn show_members(
        &mut self,
        comp: &mut C,
        ui: &mut Ui,
        cx: &mut Cx<'_, C>,
        ((ctx, id, rtl), env): ((&Context, Id, bool), &Env),
        members: Vec<Member<C::Key, C::Item<'r>>>,
        shown_rows: &mut [ShownRow<C::Key>],
    ) {
        let enabled = cx.core.resolved.enabled;
        let parent = access::fold::owner_of(ctx, id).unwrap_or(id);
        let hover = sensing::read_outcome(ctx, ArenaId::new(id)).hover.map(|hit| hit.owner);
        let mut prepared: Vec<(Member<C::Key, ()>, Id, bool, Sense, run::MemberPrep<C::Item<'r>>)> = members
            .into_iter()
            .map(|member| {
                let Member { at, key, mut item, row, clip, flags, position, renaming, openers, shown } = member;
                let member_id = item_id(id, &key);
                let on = enabled && item.common_mut(Internal::new()).enabled.unwrap_or(true);
                let item_sense = item.sense();
                let prep = run::member_prepare(ui, env, member_id, row, hover == Some(member_id), item);
                let member = Member { at, key, item: (), row, clip, flags, position, renaming, openers, shown };
                (member, member_id, on, item_sense, prep)
            })
            .collect();
        let specs: Vec<MemberSpec<'_>> = prepared
            .iter()
            .map(|(member, member_id, on, _, prep)| MemberSpec {
                id: *member_id,
                rect: member.row,
                sense: prep.sense,
                parts: prep.decls.parts(),
                enabled: *on,
            })
            .collect();
        let sensed = sensing::sense_members(ui, ArenaId::new(id), &specs);
        drop(specs);
        let clip = ui.clip_rect();
        for ((member, member_id, on, item_sense, prep), sensed) in prepared.drain(..).zip(sensed) {
            let Member { at, key, row, clip: row_clip, flags, position, renaming, openers, shown, .. } = member;
            let node = run::MemberNode { owner: id, part: ITEM_NODE.at(key_hash(&key)), parent, position, flags, enabled: on, item_sense };
            ui.set_clip_rect(row_clip);
            let run::MemberShown { output, tip } = run::member_show(ui, env, (member_id, row), prep, sensed, &node);
            ui.set_clip_rect(clip);
            if tip.is_some() {
                cx.core.member_tip = tip;
            }
            if let Some(shown) = shown_rows.get_mut(shown) {
                shown.openers = self.item_openers(ui, ctx, id, enabled, openers);
            }
            let label = output.label_rect().unwrap_or(row);
            if let Some(ask) = output.open_request() {
                self.events.push(CollectionEvent::Open(OpenChange { key: key.clone(), open: ask.open, all: ask.all }));
            }
            if let Some(at) = output.menu_request() {
                self.events.push(CollectionEvent::ContextMenu(ContextMenuRequest {
                    key: key.clone(),
                    at: self.on_screen(at),
                    column: None,
                    part: None,
                }));
            }
            if let Some(event) = output.into_event() {
                self.events.push(CollectionEvent::Item(key.clone(), event));
            }
            if renaming {
                self.rename_row(comp, ui, (ctx, id, rtl), (at, &key), label);
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
