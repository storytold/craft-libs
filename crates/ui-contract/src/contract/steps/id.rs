//! Step 3, id: the `Auto` ordinals and the ids a pass registers.

use crate::contract::{Component, IdSource};
use crate::runtime::egui::pass;
use crate::runtime::frame::{PerViewport, Stamped};
use crate::runtime::inspect;
use egui::{Context, Id, Ui};
use std::any::TypeId;
use std::collections::{HashMap, HashSet};

/// The `Auto` ordinals and the ids registered in one pass of one viewport.
#[derive(Default)]
struct IdPass {
    next: HashMap<(Id, TypeId), u32>,
    seen: HashSet<Id>,
}

/// Runs `f` as a measuring run that leaves the `Auto` ordinals as it found them, so the run that follows in the same
/// pass gives every component the id the measuring run gave it.
pub(in crate::contract) fn measuring<R>(ctx: &Context, f: impl FnOnce() -> R) -> R {
    keeping_ids(ctx, || (crate::runtime::measure::run(ctx, f), true))
}

/// Runs `f` and, when it returns `true` beside its result, leaves the `Auto` ordinals as it found them, so what runs
/// next in the same pass gives every component the id `f`'s run gave it.
pub(crate) fn keeping_ids<R>(ctx: &Context, f: impl FnOnce() -> (R, bool)) -> R {
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    let ordinals = |take: Option<HashMap<(Id, TypeId), u32>>| {
        PerViewport::<Stamped<IdPass>>::with_pass(
            ctx,
            viewport,
            pass_nr,
            |pass, _| *pass = IdPass::default(),
            |pass| match take {
                Some(next) => {
                    pass.next = next;
                    None
                }
                None => Some(pass.next.clone()),
            },
        )
    };
    let kept = ordinals(None);
    let (out, restore) = f();
    if restore {
        ordinals(kept);
    }
    out
}

/// Step 3, id.
///
/// `register` is false for `measure_in`, which peeks, and in a sizing pass. `advance` is false only for
/// `measure_in`: a sizing pass advances the ordinals, so its ids equal those of the pass that follows it.
pub(crate) fn id<C: Component>(ui: &Ui, source: &IdSource, register: bool, advance: bool) -> Id {
    let ctx = ui.ctx();
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    let scope = ui.scope_id();
    let ty = TypeId::of::<C::StyleKey>();
    let id = match source {
        IdSource::Id(id) => *id,
        IdSource::Salt(salt) => scope.with(*salt),
        _ => {
            let n = PerViewport::<Stamped<IdPass>>::with_pass(
                ctx,
                viewport,
                pass_nr,
                |pass, _| *pass = IdPass::default(),
                |pass| {
                    let slot = pass.next.entry((scope, ty)).or_insert(0);
                    let n = *slot;
                    if advance {
                        *slot += 1;
                    }
                    n
                },
            );
            scope.with((ty, n))
        }
    };
    if register {
        crate::runtime::memory::register_owner(ctx, id);
        let repeated =
            PerViewport::<Stamped<IdPass>>::with_pass(ctx, viewport, pass_nr, |pass, _| *pass = IdPass::default(), |pass| !pass.seen.insert(id));
        if repeated {
            log::warn!("{} was shown twice with the id {id:?} in one pass", std::any::type_name::<C>());
            inspect::finding(ctx, inspect::Finding { rule: "duplicate-id", component: std::any::type_name::<C>(), subject: format!("{id:?}") });
        }
    }
    id
}
