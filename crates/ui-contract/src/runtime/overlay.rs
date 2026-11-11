//! The layer stack (15.2): the record of the egui layers shown, their order and the open-modal fact.
//!
//! The stack owns no open state. The overlay host records an entry for each layer it shows; entries recorded in a
//! pass are read in the next one, and an entry not recorded for a frame is gone.

use crate::runtime::commands;
use crate::runtime::egui::areas::{self, Layer, Tier};
use crate::runtime::egui::memory::Service;
use crate::runtime::egui::pass;
use crate::runtime::frame::{PassStep, PerViewport, Stamped};
use crate::runtime::pointer;
use craft_behavior::OwnerClass;
use craft_behavior::overlay::Side;
use egui::{Context, Id, Pos2, Rect, ViewportId};
use smallvec::SmallVec;
use std::collections::HashMap;

/// A modal's backdrop.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Backdrop {
    /// No scrim; the layer still blocks input below it.
    None,
    /// One scrim over the viewport.
    Scrim,
    /// A scrim with a hole around a rect (the tour).
    ScrimWithHole(Rect),
}

/// An overlay's place in the order. `Rank` is the only place a layer's egui order is chosen.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Rank {
    /// A floating window or panel.
    Window,
    /// A toast.
    Toast,
    /// A popup, menu or dropdown.
    Popup,
    /// A modal layer; it blocks input below it.
    #[non_exhaustive]
    Modal {
        /// The scrim behind it.
        backdrop: Backdrop,
    },
    /// The chrome of a modal that stays above its popups.
    ModalChrome,
    /// A tooltip.
    Tooltip,
}

impl Rank {
    /// A modal rank with `backdrop`.
    pub const fn modal(backdrop: Backdrop) -> Rank {
        Rank::Modal { backdrop }
    }

    /// Families of one egui order sort by this class: toast below popup below modal below modal chrome.
    const fn class(self) -> u8 {
        match self {
            Rank::Window => 0,
            Rank::Toast => 1,
            Rank::Popup => 2,
            Rank::Modal { .. } => 3,
            Rank::ModalChrome => 4,
            Rank::Tooltip => 5,
        }
    }

    pub(crate) const fn tier(self) -> Tier {
        match self {
            Rank::Window => Tier::Window,
            Rank::Tooltip => Tier::Tooltip,
            _ => Tier::Foreground,
        }
    }

    /// Whether a layer of this rank takes keys without a focus scope. Modal chrome takes none: Cancel and Enter stay
    /// with the modal under it.
    const fn takes_keys(self) -> bool {
        matches!(self, Rank::Popup | Rank::Modal { .. })
    }

    #[doc(hidden)]
    pub const fn is_modal(self) -> bool {
        matches!(self, Rank::Modal { .. })
    }
}

/// Why a layer closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CloseReason {
    /// Code asked for it.
    Requested,
    /// Escape reached the top layer.
    Escape,
    /// A press landed outside the layer and its anchor.
    Outside,
    /// Focus left the layer.
    FocusLeft,
    /// A component inside was activated.
    Activated,
    /// Enter was pressed inside.
    Enter,
    /// The timeout ran out.
    Timeout,
    /// The anchor is no longer shown.
    AnchorGone,
}

/// What `any_open` counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenFilter {
    /// Every layer.
    Any,
    /// Modal layers.
    Modal,
    /// Layers that take keys: popups and modals.
    Keys,
}

/// One shown egui layer.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct OpenEntry {
    /// The layer's id.
    pub id: Id,
    /// The layer it was opened from inside.
    pub parent: Option<Id>,
    /// Its rank.
    pub rank: Rank,
    /// Its key class.
    pub keys: OwnerClass,
    /// The anchor in screen space.
    pub anchor: Option<Rect>,
    /// The rects in screen space of the other parts of the owner that open popups: a press on one is not outside,
    /// so the press reaches it and it switches the open popup.
    pub peers: Vec<Rect>,
    /// The layer's rect in screen space.
    pub rect: Rect,
    /// The side of the anchor it took by its size; `None` for a float and while its size is unknown.
    pub side: Option<Side>,
    /// It is fading out: not `top`, not counted by `any_open`, taking no input.
    pub exiting: bool,
    /// The layer's egui id and the cover under it, listed in the order.
    pub(crate) layer: Layer,
    pub(crate) cover: Option<Layer>,
    /// The layer has a focus scope.
    pub(crate) scoped: bool,
    /// A press outside its family closes it and goes no further.
    pub(crate) absorbs: bool,
    /// The entry is an OS window of the integration: it takes its place in the stack's order and facts, and no egui
    /// layer is shown for it.
    pub(crate) window: bool,
    /// The screen rect the layer is contained in: its scrim, placement, clip and input blocking stop at it.
    pub(crate) region: Option<Rect>,
}

impl OpenEntry {
    /// An entry for the layer `id` of `rank`.
    pub(crate) fn new(id: Id, rank: Rank, keys: OwnerClass, rect: Rect, layer: Layer) -> Self {
        OpenEntry {
            id,
            parent: None,
            rank,
            keys,
            anchor: None,
            peers: Vec::new(),
            rect,
            side: None,
            exiting: false,
            layer,
            cover: None,
            scoped: false,
            absorbs: false,
            window: false,
            region: None,
        }
    }

    /// Whether the layer is a modal over its whole window: a modal contained in a region blocks only inside it, and the
    /// window's keys, commands and hits stay with the rest.
    pub(crate) fn blocks_window(&self) -> bool {
        self.rank.is_modal() && self.region.is_none()
    }

    /// Whether the layer takes keys: the layers `top` can name. A layer with a focus scope takes keys whatever its rank.
    pub(crate) fn takes_keys(&self) -> bool {
        self.scoped || self.rank.takes_keys()
    }

    /// Whether the layer takes pointer input.
    fn interactable(&self) -> bool {
        !self.exiting && self.rank != Rank::Tooltip
    }
}

#[derive(Default)]
struct Pass {
    current: Vec<OpenEntry>,
    last: Vec<OpenEntry>,
    /// The layers whose bodies are running, innermost last.
    drawing: Vec<Id>,
    /// Each activation of this pass: the component and a layer whose body was running around it.
    activated: Vec<(Id, Id)>,
    /// The components whose activation this pass opened or closed a popup (`claim_activation`).
    claimed: Vec<Id>,
    /// The layers a primary press landed outside of, this pass, while their dismissal keeps them open.
    pressed_outside: Vec<Id>,
    /// The popup a press that opened it ended on, and where, this pass.
    dragged_release: Option<(Id, Pos2)>,
}

/// The top modal layer of a viewport: its id and the scrim under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TopModal {
    pub id: Id,
    pub cover: Option<Layer>,
}

#[derive(Default)]
struct Persist {
    raised: HashMap<Id, u64>,
    counter: u64,
    modal: bool,
    /// The layers of the last pass, bottom to top, as `end_pass` handed them to egui.
    stack: Vec<Id>,
    /// A press the modal drop removed, for the window host to raise the modal's window.
    dropped_press: bool,
    /// The screen rects a press may land in without closing the top menu family: the family and its anchors. `None`
    /// when the top layer does not absorb outside presses.
    absorb_zone: Option<Vec<Rect>>,
    /// An outside press the input hook removed for the running pass.
    outside_press: Option<Pos2>,
    /// The top live modal layer of the last finished pass in this viewport.
    top_modal: Option<TopModal>,
    /// A popup stands over a modal layer here, as of the last finished pass.
    popup_over_modal: bool,
    /// The layers that first showed while the primary button was held, until that press ends.
    held_open: std::collections::HashSet<Id>,
    /// The root frame and pass each shown layer first ran its body in; gone with the layer's last entry.
    appeared: HashMap<Id, (u64, u64)>,
}

/// The closes queued and not yet taken, each with the root frame that queued it and whether it is carried: a carried
/// close waits for its owner however long the owner does not run.
#[derive(Default)]
struct Closes(Vec<(Id, CloseReason, u64, bool)>);

fn with_pass<R>(ctx: &Context, f: impl FnOnce(&mut Pass) -> R) -> R {
    let viewport = pass::current_viewport(ctx);
    PerViewport::<Stamped<Pass>>::with_pass(
        ctx,
        viewport,
        pass::pass_nr(ctx, viewport),
        |pass, step| {
            pass.last = std::mem::take(&mut pass.current);
            if matches!(step, PassStep::Skipped) {
                pass.last.clear();
            }
            pass.drawing.clear();
            pass.activated.clear();
            pass.claimed.clear();
            pass.pressed_outside.clear();
            pass.dragged_release = None;
        },
        f,
    )
}

fn with_persist<R>(ctx: &Context, viewport: ViewportId, f: impl FnOnce(&mut Persist) -> R) -> R {
    PerViewport::<Persist>::with(ctx, viewport, f)
}

/// Whether the layer `id` was shown last frame.
pub fn is_open(ctx: &Context, id: Id) -> bool {
    with_pass(ctx, |pass| pass.last.iter().any(|entry| entry.id == id))
}

/// Queues a close of the layer `id` (a popup or a container). The owner's step 9 or `OverlayUi::show` collects it and
/// step 15 writes it.
pub fn request_close(ctx: &Context, id: Id, why: CloseReason) {
    if !crate::runtime::measure::effect(ctx, "overlay::request_close") {
        return;
    }
    push_close(ctx, id, why, false);
}

fn push_close(ctx: &Context, id: Id, why: CloseReason, carried: bool) {
    let frame = pass::root_frame_nr(ctx);
    Service::<Closes>::run(ctx, |closes| {
        if carried {
            closes.0.retain(|(closed, .., kept)| !(*closed == id && *kept));
        }
        closes.0.push((id, why, frame, carried));
    });
    crate::runtime::egui::repaint::request_of(ctx, crate::runtime::egui::pass::current_viewport(ctx));
}

/// The queued close of `id`, once.
pub(crate) fn take_close(ctx: &Context, id: Id) -> Option<CloseReason> {
    Service::<Closes>::run(ctx, |closes| {
        let at = closes.0.iter().position(|(closed, ..)| *closed == id)?;
        let reason = closes.0[at].1;
        closes.0.retain(|(closed, ..)| *closed != id);
        Some(reason)
    })
}

/// The queued close of `id`, left queued.
pub(crate) fn peek_close(ctx: &Context, id: Id) -> Option<CloseReason> {
    Service::<Closes>::run(ctx, |closes| closes.0.iter().find(|(closed, ..)| *closed == id).map(|entry| entry.1))
}

/// Whether any layer of the running viewport that `filter` counts was shown last frame.
pub fn any_open(ctx: &Context, filter: OpenFilter) -> bool {
    with_pass(ctx, |pass| {
        pass.last.iter().filter(|entry| !entry.exiting).any(|entry| match filter {
            OpenFilter::Any => true,
            OpenFilter::Modal => entry.blocks_window(),
            OpenFilter::Keys => entry.takes_keys(),
        })
    })
}

/// The layer that receives Cancel: the newest member of the topmost family that takes keys.
pub fn top(ctx: &Context) -> Option<Id> {
    let viewport = pass::current_viewport(ctx);
    let raised = with_persist(ctx, viewport, |persist| persist.raised.clone());
    with_pass(ctx, |pass| {
        let live: Vec<OpenEntry> = pass.last.iter().filter(|entry| !entry.exiting).cloned().collect();
        let families = families(&live, &raised);
        families.iter().rev().find_map(|members| members.iter().rev().find(|entry| entry.takes_keys()).map(|entry| entry.id))
    })
}

/// `id` and everything opened from inside it, shown last frame.
pub fn family(ctx: &Context, id: Id) -> SmallVec<[Id; 4]> {
    with_pass(ctx, |pass| {
        let mut out: SmallVec<[Id; 4]> = SmallVec::new();
        if pass.last.iter().any(|entry| entry.id == id) {
            out.push(id);
        }
        // Entries are recorded parent first, so one sweep finds every descendant.
        for entry in &pass.last {
            if entry.parent.is_some_and(|parent| out.contains(&parent)) && !out.contains(&entry.id) {
                out.push(entry.id);
            }
        }
        out
    })
}

/// Every layer of the running viewport shown last frame, not exiting, parent first.
#[cfg(feature = "testing")]
pub(crate) fn open_entries(ctx: &Context) -> Vec<OpenEntry> {
    with_pass(ctx, |pass| pass.last.iter().filter(|entry| !entry.exiting).cloned().collect())
}

/// The entry of the layer `id` shown last frame.
#[doc(hidden)]
pub fn entry(ctx: &Context, id: Id) -> Option<OpenEntry> {
    with_pass(ctx, |pass| pass.last.iter().find(|entry| entry.id == id).cloned())
}

/// The parent a layer opened now has: the layer whose body is running.
#[doc(hidden)]
pub fn drawing_parent(ctx: &Context) -> Option<Id> {
    with_pass(ctx, |pass| pass.drawing.last().copied())
}

/// Whether the layer `id`, open in this pass, faded out in the last one: it opens anew.
fn reopened(ctx: &Context, id: Id, exiting: bool) -> bool {
    !exiting && with_pass(ctx, |pass| pass.last.iter().any(|entry| entry.id == id && entry.exiting))
}

/// Whether the layer `id` appears in this pass, read before its `begin_layer`: it was hidden in the last pass, or it
/// faded out and is open again. What the layer remembers starts fresh then; a discard pass that repeats the frame
/// keeps what the first pass of the frame recorded.
pub(crate) fn appears(ctx: &Context, id: Id, exiting: bool) -> bool {
    reopened(ctx, id, exiting) || with_persist(ctx, pass::current_viewport(ctx), |persist| !persist.appeared.contains_key(&id))
}

/// Marks the layer `id`'s body as running, so layers opened from inside it become its children. A layer shown
/// again while it fades out opens anew: it appears in this frame and pass.
pub(crate) fn begin_layer(ctx: &Context, id: Id, exiting: bool) {
    let reopened = reopened(ctx, id, exiting);
    with_pass(ctx, |pass| pass.drawing.push(id));
    let viewport = pass::current_viewport(ctx);
    let first = (pass::root_frame_nr(ctx), pass::pass_nr(ctx, viewport));
    let (down, released) = pointer::with_pass_pointer(ctx, |read| {
        let released = read.frame.as_ref().filter(|frame| frame.release == Some(craft_behavior::PointerButton::Primary)).map(|frame| frame.pos);
        (read.primary_down, released)
    });
    let release = with_persist(ctx, viewport, |persist| {
        let new = reopened || !persist.appeared.contains_key(&id);
        if reopened {
            persist.appeared.insert(id, first);
        } else {
            persist.appeared.entry(id).or_insert(first);
        }
        if exiting {
            return None;
        }
        // A layer that first shows while the primary button is held was opened by that press.
        if new && down {
            persist.held_open.insert(id);
        }
        match released {
            Some(at) => persist.held_open.contains(&id).then_some(at).flatten(),
            None => {
                if !down {
                    persist.held_open.remove(&id);
                }
                None
            }
        }
    });
    if let Some(at) = release {
        note_dragged_release(ctx, id, at);
    }
}

/// Whether the running pass is the one the layer `id` first ran its body in: false in the discard passes that
/// repeat that frame, so what a body resets on opening resets once.
#[doc(hidden)]
pub fn opening(ctx: &Context, id: Id) -> bool {
    let viewport = pass::current_viewport(ctx);
    let now = (pass::root_frame_nr(ctx), pass::pass_nr(ctx, viewport));
    with_persist(ctx, viewport, |persist| persist.appeared.get(&id) == Some(&now))
}

/// Queues a close of every layer opened from inside the layer `id`, so the open state of a popup in its body does
/// not outlive it. The closes are carried: with no motion the layer's body hides in the same frame, so the owner of
/// a popup in it runs again only when the layer reopens, and takes the close then.
pub(crate) fn close_descendants(ctx: &Context, id: Id) {
    for descendant in family(ctx, id).into_iter().filter(|member| *member != id) {
        push_close(ctx, descendant, CloseReason::Requested, true);
    }
}

/// Records that a primary press landed outside the layer `id`, its anchor and its family in this pass.
pub(crate) fn note_pressed_outside(ctx: &Context, id: Id) {
    with_pass(ctx, |pass| pass.pressed_outside.push(id));
}

/// Whether a primary press landed outside the layer `id` in this pass while its dismissal kept it open.
#[doc(hidden)]
pub fn pressed_outside(ctx: &Context, id: Id) -> bool {
    with_pass(ctx, |pass| pass.pressed_outside.contains(&id))
}

/// Records that the press which opened the popup `popup` ended at the screen position `at` (press, drag, release): a
/// row of the popup under `at` is activated.
pub(crate) fn note_dragged_release(ctx: &Context, popup: Id, at: Pos2) {
    with_pass(ctx, |pass| pass.dragged_release = Some((popup, at)));
}

/// Whether a press that opened the layer whose body is running ended inside `rect` (screen space) in this pass.
pub(crate) fn released_inside(ctx: &Context, rect: Rect) -> bool {
    with_pass(ctx, |pass| pass.dragged_release.is_some_and(|(popup, at)| pass.drawing.contains(&popup) && rect.contains(at)))
}

/// Ends the body `begin_layer` started.
pub(crate) fn end_layer(ctx: &Context) {
    with_pass(ctx, |pass| {
        pass.drawing.pop();
    });
}

/// Records that the component `id` was activated: every layer whose body is running, except the one `id` is the
/// container of, may close on it (`Dismiss.on_activate`).
pub(crate) fn note_activated(ctx: &Context, id: Id) {
    with_pass(ctx, |pass| {
        let layers: Vec<(Id, Id)> = pass.drawing.iter().copied().filter(|layer| *layer != id).map(|layer| (id, layer)).collect();
        pass.activated.extend(layers);
    });
}

/// Records that the activation of `component` in this pass toggled a popup it opens: an opener's activation opens
/// its popup and is no pick, so the layers it sits in stay open. Called before or after `note_activated`.
pub(crate) fn claim_activation(ctx: &Context, component: Id) {
    with_pass(ctx, |pass| {
        if !pass.claimed.contains(&component) {
            pass.claimed.push(component);
        }
    });
}

/// Whether a component inside the layer `id` was activated in this pass, other than an opener toggling its popup.
pub(crate) fn activated_inside(ctx: &Context, id: Id) -> bool {
    with_pass(ctx, |pass| pass.activated.iter().any(|(component, layer)| *layer == id && !pass.claimed.contains(component)))
}

/// The input hook: removes the primary presses that land outside the top menu family, its anchors and its parents,
/// so nothing below the menu sees them (`Outside::Absorb`), and keeps the first one for the layers' dismissal.
/// Returns whether it removed one.
pub(crate) fn absorb_presses(ctx: &Context, input: &mut crate::runtime::egui::input::RawInput) -> bool {
    let zone = with_persist(ctx, input.viewport_id, |persist| persist.absorb_zone.clone());
    let Some(zone) = zone else { return false };
    let before = input.events.len();
    let mut first = None;
    input.events.retain(|event| match event {
        egui::Event::PointerButton { pos, pressed: true, .. } if !zone.iter().any(|rect| rect.contains(*pos)) => {
            first.get_or_insert(*pos);
            false
        }
        _ => true,
    });
    if let Some(pos) = first {
        with_persist(ctx, input.viewport_id, |persist| persist.outside_press = Some(pos));
    }
    input.events.len() != before
}

/// The outside press the input hook removed for the running pass.
pub(crate) fn absorbed_press(ctx: &Context) -> Option<Pos2> {
    with_persist(ctx, pass::current_viewport(ctx), |persist| persist.outside_press)
}

/// Records the toggling parts that sit beside the anchor of the layer `id` shown in this pass.
pub(crate) fn note_peers(ctx: &Context, id: Id, peers: Vec<Rect>) {
    with_pass(ctx, |pass| {
        if let Some(entry) = pass.current.iter_mut().find(|entry| entry.id == id) {
            entry.peers = peers;
        }
    });
}

/// Records the layer shown in this pass, parent first: a layer is recorded once its body ran, after the layers opened
/// from inside it, so it goes in front of them, and every layer then stacks above the layer it was opened from. A
/// layer shown again in a discarded pass replaces its entry.
pub(crate) fn record(ctx: &Context, entry: OpenEntry) {
    with_pass(ctx, |pass| {
        if let Some(at) = pass.current.iter().position(|recorded| recorded.id == entry.id) {
            pass.current[at] = entry;
            return;
        }
        let descends = |recorded: &OpenEntry| {
            let mut parent = recorded.parent;
            while let Some(id) = parent {
                if id == entry.id {
                    return true;
                }
                parent = pass.current.iter().find(|other| other.id == id).and_then(|other| other.parent);
            }
            false
        };
        let at = pass.current.iter().position(descends).unwrap_or(pass.current.len());
        pass.current.insert(at, entry);
    });
}

/// Groups `entries` into families, each parent first, ordered bottom to top.
fn families(entries: &[OpenEntry], raised: &HashMap<Id, u64>) -> Vec<Vec<OpenEntry>> {
    let root_of = |entry: &OpenEntry| {
        let mut at = entry;
        while let Some(parent) = at.parent.and_then(|parent| entries.iter().find(|other| other.id == parent)) {
            at = parent;
        }
        at.id
    };
    let mut groups: Vec<(Id, Vec<OpenEntry>)> = Vec::new();
    for entry in entries {
        let root = root_of(entry);
        match groups.iter_mut().find(|(id, _)| *id == root) {
            Some((_, members)) => members.push(entry.clone()),
            None => groups.push((root, vec![entry.clone()])),
        }
    }
    // A modal opened from a floating panel lifts the whole family: the family sorts by its highest member.
    let class = |members: &Vec<OpenEntry>| members.iter().map(|member| member.rank.class()).max().unwrap_or(0);
    groups.sort_by_key(|(root, members)| (class(members), raised.get(root).copied().unwrap_or(0)));
    groups.into_iter().map(|(_, members)| members).collect()
}

/// The end hook: raises the families that appeared or were pressed, hands egui the whole order, and records the
/// open-modal fact.
pub(crate) fn end_pass(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    let (current, last_ids) = with_pass(ctx, |pass| (pass.current.clone(), pass.last.iter().map(|entry| entry.id).collect::<Vec<_>>()));
    let pressed = pointer::frame(ctx).press.map(|press| press.origin);
    let order = with_persist(ctx, viewport, |persist| {
        let stamp = |id: Id, persist: &mut Persist| {
            persist.counter += 1;
            persist.raised.insert(id, persist.counter);
        };
        let live = |entry: &&OpenEntry| !entry.exiting;
        let roots: Vec<Id> = current
            .iter()
            .filter(|entry| entry.parent.is_none_or(|parent| !current.iter().any(|other| other.id == parent)))
            .map(|entry| entry.id)
            .collect();
        for root in &roots {
            if !last_ids.contains(root) {
                stamp(*root, persist);
            }
        }
        persist.raised.retain(|id, _| roots.contains(id));
        persist.appeared.retain(|id, _| current.iter().any(|entry| entry.id == *id));
        let before = families(&current, &persist.raised);
        if let Some(point) = pressed
            && let Some(hit) = before
                .iter()
                .rev()
                .find(|members| members.iter().filter(live).filter(|entry| entry.interactable()).any(|entry| entry.rect.contains(point)))
            && let Some(root) = hit.first()
        {
            stamp(root.id, persist);
        }
        let families = families(&current, &persist.raised);
        let mut order: Vec<Layer> = Vec::new();
        persist.stack.clear();
        for members in &families {
            for member in members {
                if !member.window {
                    order.extend(member.cover);
                    order.push(member.layer);
                }
                persist.stack.push(member.id);
            }
        }
        persist.modal = current.iter().any(|entry| entry.blocks_window() && !entry.exiting);
        persist.top_modal =
            current.iter().rev().find(|entry| entry.blocks_window() && !entry.exiting).map(|entry| TopModal { id: entry.id, cover: entry.cover });
        persist.popup_over_modal = current.iter().filter(|entry| entry.rank == Rank::Popup && !entry.exiting).any(|popup| {
            let mut at = popup.parent;
            while let Some(parent) = at.and_then(|id| current.iter().find(|entry| entry.id == id)) {
                if parent.rank.is_modal() && !parent.exiting {
                    return true;
                }
                at = parent.parent;
            }
            false
        });
        persist.outside_press = None;
        persist.absorb_zone = families
            .iter()
            .rev()
            .find_map(|members| members.iter().rev().find(|entry| !entry.exiting && entry.takes_keys()).map(|top| (top, members)))
            .filter(|(top, _)| top.absorbs)
            .map(|(_, members)| {
                members.iter().flat_map(|member| std::iter::once(member.rect).chain(member.anchor).chain(member.peers.iter().copied())).collect()
            });
        order
    });
    areas::set_layer_order(ctx, &order);
    // An owner takes a close in the frame after the request at the latest; one nobody took is for a layer that is not shown.
    let now = pass::root_frame_nr(ctx);
    Service::<Closes>::run(ctx, |closes| closes.0.retain(|(.., queued, carried)| *carried || queued + 1 >= now));
    let top_modal = current.iter().rev().find(|entry| entry.blocks_window() && !entry.exiting);
    if let Some(entry) = top_modal.filter(|entry| !entry.window) {
        areas::set_modal_layer(ctx, entry.layer);
    }
    let any = {
        let here = top_modal.is_some();
        here || modal_in_other_viewports(ctx, viewport)
    };
    commands::set_modal_open(ctx, any, viewport, top_modal.map(|entry| entry.id));
}

fn modal_in_other_viewports(ctx: &Context, running: ViewportId) -> bool {
    pass::live_viewports(ctx).into_iter().filter(|viewport| *viewport != running).any(|viewport| with_persist(ctx, viewport, |persist| persist.modal))
}

/// Whether a modal layer is open in a viewport other than `viewport`: the global open-modal fact the input hook
/// reads (H3).
pub(crate) fn modal_elsewhere(ctx: &Context, viewport: ViewportId) -> bool {
    modal_in_other_viewports(ctx, viewport)
}

/// Records that the input hook dropped a press for `viewport` because a modal is open elsewhere.
pub(crate) fn note_dropped_press(ctx: &Context, viewport: ViewportId) {
    with_persist(ctx, viewport, |persist| persist.dropped_press = true);
}

/// Whether the input hook dropped a press for `viewport` since the last call; the window host raises the modal's
/// window for it.
pub(crate) fn take_dropped_press(ctx: &Context, viewport: ViewportId) -> bool {
    with_persist(ctx, viewport, |persist| std::mem::take(&mut persist.dropped_press))
}

/// What the layer stack says about key ownership for focus inside the scopes `scopes` (innermost first).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayerKeys {
    /// A modal layer is above the focus, in this viewport or another.
    Blocked,
    /// The focus is inside a layer of this key class.
    Class(OwnerClass),
    /// The focus is in no layer.
    Window,
}

/// The key ownership the layer stack gives the focus: blocked when a modal layer is stacked above the layer the
/// focus is in (or above the window when it is in none), else that layer's key class.
pub(crate) fn key_layer(ctx: &Context, scopes: &[Id]) -> LayerKeys {
    let viewport = pass::current_viewport(ctx);
    let stack = with_persist(ctx, viewport, |persist| persist.stack.clone());
    let (inside, modals) = with_pass(ctx, |pass| {
        let inside = scopes.iter().find_map(|scope| pass.last.iter().find(|entry| entry.id == *scope && !entry.exiting).cloned());
        let modals: Vec<Id> = pass.last.iter().filter(|entry| entry.blocks_window() && !entry.exiting).map(|entry| entry.id).collect();
        (inside, modals)
    });
    let at = |id: Id| stack.iter().position(|layer| *layer == id);
    let floor = inside.as_ref().and_then(|entry| at(entry.id));
    let above = modals.iter().any(|modal| match (at(*modal), floor, &inside) {
        (_, _, Some(entry)) if entry.id == *modal => false,
        (Some(modal_at), Some(floor), _) => modal_at > floor,
        (_, None, None) => true,
        _ => false,
    });
    if above || modal_in_other_viewports(ctx, viewport) {
        return LayerKeys::Blocked;
    }
    inside.map_or(LayerKeys::Window, |entry| LayerKeys::Class(entry.keys))
}

/// A viewport other than `viewport` that has a modal layer open: the window a blocked press raises.
pub(crate) fn modal_viewport(ctx: &Context, viewport: ViewportId) -> Option<ViewportId> {
    pass::live_viewports(ctx).into_iter().filter(|other| *other != viewport).find(|other| with_persist(ctx, *other, |persist| persist.modal))
}

/// Whether a layer of key class `class` that is not fading out was shown last frame.
pub(crate) fn class_open(ctx: &Context, class: OwnerClass) -> bool {
    with_pass(ctx, |pass| pass.last.iter().any(|entry| !entry.exiting && entry.keys == class))
}

// The facts the canvas reach under a modal reads (`runtime::sampler`).

/// The top live modal layer shown in `viewport` as of its last finished pass.
pub(crate) fn top_modal_in(ctx: &Context, viewport: ViewportId) -> Option<TopModal> {
    with_persist(ctx, viewport, |persist| persist.top_modal)
}

/// Whether the layer `id` is the top modal layer of some live viewport.
#[doc(hidden)]
pub fn is_top_modal(ctx: &Context, id: Id) -> bool {
    pass::live_viewports(ctx).into_iter().any(|viewport| top_modal_in(ctx, viewport).is_some_and(|modal| modal.id == id))
}

/// Whether a popup stands over a modal layer in any viewport.
pub(crate) fn popup_over_modal(ctx: &Context) -> bool {
    pass::live_viewports(ctx).into_iter().any(|viewport| with_persist(ctx, viewport, |persist| persist.popup_over_modal))
}

/// The modal layer whose body is running, the innermost one when layers nest: the layer a control inside it belongs to.
pub(crate) fn modal_drawing(ctx: &Context) -> Option<Id> {
    with_pass(ctx, |pass| {
        pass.drawing.iter().rev().copied().find(|id| pass.current.iter().chain(&pass.last).any(|entry| entry.id == *id && entry.rank.is_modal()))
    })
}
