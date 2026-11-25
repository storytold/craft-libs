//! The focus service: stops, scopes and roving groups registered per pass, the focused stop per arena root,
//! focus on press, focus after removal and programmatic focus (spec 8.3, 9.4, 9.5).
//!
//! egui's focus sits on an arena root. The focused member or part is runtime data keyed by that root, and
//! every focus read of component code goes through `is_focused`.

mod nav;
mod policy;
mod press;
#[doc(hidden)]
pub mod registration;

pub(crate) use nav::{enter_row, leave_row, roving_move, row_has_stops};
pub(crate) use press::{held_draft, note_holds_drafts};
pub(crate) use registration::{Group, Stop};

use crate::runtime::egui::focus as egui_focus;
use crate::runtime::egui::memory::FastMap;
use crate::runtime::frame::PerViewport;
use crate::runtime::inspect;
use crate::style::log_once;
use craft_behavior::{DocKey, OwnerClass, PartRef};
use egui::{Context, Id, IdSet, ViewportId};
use smallvec::SmallVec;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub use crate::runtime::keys::Intents;
pub use craft_behavior::{Letters, Roving};
pub use policy::{ActivateKeys, EditSet, EndKey, EnterRule, FocusPolicy, FocusRequest, FocusTarget, Initial, KeepKeys, KeyUse, NudgeMods, TabReach};

/// The shape the focus ring takes around a part.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FocusShape {
    /// The part's rect.
    Rect,
    /// The circle inside the part's rect.
    Circle,
    /// A closed polygon, its points in screen points.
    Polygon(Vec<egui::Pos2>),
}

/// A focus stop: a component, or one of its parts.
pub(crate) type StopKey = (Id, Option<PartRef>);

/// A registered stop with its place in the scope tree.
#[derive(Clone, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub struct StopRec {
    #[doc(hidden)]
    pub stop: Stop,
    pub(crate) scope: Option<Id>,
    /// The component takes its place in its set from its roving group's members of this pass, which the group's owner
    /// writes into its node at its own report (`group_positions`). Marked at the component's report.
    pub(crate) group_position: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScopeRec {
    pub(crate) policy: FocusPolicy,
    pub(crate) parent: Option<Id>,
    pub(crate) label: Option<Arc<str>>,
    /// The stop that held focus when the scope was first pushed, or for a scope that does not trap the stop focus last
    /// entered it from, which `restore` returns to.
    pub(crate) opener: Option<StopKey>,
    /// The scope's layer is playing its exit: focus inside it leaves at once, to the opener or nowhere.
    pub(crate) closing: bool,
    /// For a layer's scope: whether this is its first pass since the layer appeared, as the layer's memory says;
    /// none for other scopes, which are new after a gap of a frame.
    pub(crate) entering: Option<bool>,
}

/// What the end hook recorded for the next input hook, which reads only `RawInput` and records.
#[derive(Clone, Debug, Default)]
pub(crate) struct FocusedRecord {
    pub(crate) keys: Option<KeyUse>,
    pub(crate) claims: Vec<craft_keymap::Chord>,
    /// The component of the focused stop: only its state-dependent claims hold.
    pub(crate) focused: Option<Id>,
    /// The focused stop is the app's own egui widget (`Stop::host`: one of the app's focusable egui widgets, or its
    /// canvas): it reads the presses it claims from egui.
    pub(crate) host: bool,
    /// The arena root of the focused stop when the stop is a non-text member of an arena (a part of a canvas's frame
    /// overlay): the root's state-dependent claims hold too, so a canvas tool's keys work with a part focused.
    pub(crate) root: Option<Id>,
    /// The container that runs the menu level the focused stop sits in: its state-dependent claims (the letters its
    /// rows' mnemonics name) hold too.
    pub(crate) menu: Option<Id>,
    /// The focused stop switches pages with Ctrl+Tab.
    pub(crate) pages: bool,
    /// The component that registered as the window's menu bar (`register_menu_bar`) in the last pass: it takes the
    /// menu-bar keys, and its state-dependent claims (the letters keytips badges take) hold beside the focused one's.
    pub(crate) menu_bar: Option<Id>,
    /// The registered menu bar takes only the Alt+letter chords it claims as mnemonics (`register_menu_bar_by_claim`).
    pub(crate) menu_bar_by_claim: bool,
    /// The window's document canvas (`KeyUse::chrome_key`) the last pass registered, while no stop has focus and no
    /// modal layer is open: its state-dependent claims hold, as its tool takes the arrows, Enter and Escape with nothing
    /// focused.
    pub(crate) canvas: Option<Id>,
    /// egui focus is on a widget no framework component made, or an egui popup is open, and no framework press or
    /// drag is captured: an app's own focused widget or popup keeps the keys a focused component keeps (Escape, the
    /// plain Enter, the clipboard events), and no window intent comes of them. Written by the end hook after its focus
    /// moves.
    pub(crate) foreign: bool,
    /// A bare Tab is egui's: an egui popup is open (`foreign`), or egui focus is on the app's own widget and its egui
    /// key filter keeps Tab (a multi-line text edit that locks focus, a code editor). Any other bare Tab is the
    /// framework's move over its one order, which holds the app's widgets (`take_app_widgets`). Written by the end
    /// hook.
    pub(crate) tab_in_egui: bool,
    /// egui focus is on a widget no framework component made (`State::app_widget`: its stop of the app's own does not
    /// count, the app's canvas does), and no framework press or drag is captured: egui moves focus among the app's own
    /// widgets with the arrows. Written by the end hook.
    pub(crate) host_focus: bool,
    /// `host_focus`, on a text edit the app drew itself. Written by the end hook.
    pub(crate) host_text: bool,
    /// egui holds a label text selection: a copy or cut event is egui's, which copies the selection. Written by the
    /// end hook.
    pub(crate) label_selection: bool,
}

/// A change to egui's focus, applied after the state lock is released.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Act {
    Focus(Id, egui_focus::KeyFilter),
    Surrender(Id),
}

/// What the begin hook reads from egui before it takes the state lock.
pub(crate) struct BeginIn {
    pub(crate) focused: Option<Id>,
    pub(crate) press: Option<press::PressAt>,
    pub(crate) nav: crate::runtime::keys::NavKeys,
    pub(crate) tab_all_controls: bool,
    /// Which stops Tab visits inside a dialog's trap (`RootEnv::dialog_tab`).
    pub(crate) dialog_tab: TabReach,
    pub(crate) frame: u64,
}

/// The row groups of one collection's visible rows in a pass: every one is a flat roving group of the same owner
/// and direction, so the collection registers them as one record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RowGroups {
    owner: Id,
    rtl: bool,
    /// Sorted.
    ids: Vec<Id>,
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) stops: Vec<StopRec>,
    stop_ids: IdSet,
    /// The egui roots of `stops`.
    stop_roots: IdSet,
    pub(crate) last_stops: Vec<StopRec>,
    pub(crate) scopes: FastMap<Id, ScopeRec>,
    pub(crate) last_scopes: FastMap<Id, ScopeRec>,
    pub(crate) scope_order: Vec<Id>,
    pub(crate) last_scope_order: Vec<Id>,
    pub(super) open: Vec<(Id, usize)>,
    /// Where a scope's body begins in `stops`; the header's stops come before it, the footer's after the body's.
    pub(super) body_from: FastMap<Id, usize>,
    pub(crate) groups: FastMap<Id, Group>,
    pub(crate) last_groups: FastMap<Id, Group>,
    pub(crate) row_sets: Vec<RowGroups>,
    pub(crate) last_row_sets: Vec<RowGroups>,
    pub(crate) focused: FastMap<Id, StopKey>,
    pub(crate) focus_now: Option<Id>,
    pub(super) pending: Option<FocusTarget>,
    pub(super) docs: FastMap<DocKey, Id>,
    /// Last pass's `docs`: a focus request made between passes resolves against the components that last showed.
    pub(super) last_docs: FastMap<DocKey, Id>,
    pub(super) revealed: Option<StopKey>,
    /// The stop a request of the running pass named before any pass registered it: it takes the focus when it
    /// registers later in the pass. A component asks for its own focus from `update`, and its stops register after.
    pub(super) awaited: Option<StopKey>,
    pub(super) seen: FastMap<Id, u64>,
    /// A scope moved focus to its initial stop in the running pass.
    pub(super) entered: bool,
    /// A component asked for the focus in the running pass (`request`), as an editor that appears under a press does.
    pub(super) asked: bool,
    pub(crate) visible: bool,
    pub(crate) record: FocusedRecord,
    pub(super) menu_bar: Option<Id>,
    /// The registered menu bar takes only the Alt+letter chords it claims as mnemonics.
    pub(super) menu_bar_by_claim: bool,
    /// The menu bar that took focus and the stop it took focus from, for the bar or a menu pick to give back.
    pub(super) bar_return: Option<(Id, StopKey)>,
    pub(crate) press: press::PressState,
    /// The frame whose begin hook ran last; a move in the end hook belongs to the next frame.
    pub(super) frame: u64,
    /// The stop focus last moved onto, the frame whose passes see the move, and what moved it there. It stays until focus
    /// moves again: while the stop keeps the focus, it is how the stop got it (`origin`).
    pub(super) arrived: Option<(u64, StopKey, FocusOrigin)>,
    /// How many of egui's focus candidates of the running pass (`Memory::focus_candidates`) `take_app_widgets` has
    /// read.
    pub(super) candidates_seen: usize,
    /// The widget the runtime gave egui's focus, under its own press rule, until the end hook settles it
    /// (`settle_given`): the focus of an app's own widget is left to egui's rule there.
    given: Option<Id>,
    /// The app's document canvas whose focus is off egui's press rule (`hold_canvas_focus`).
    held_canvas: Option<Id>,
    /// The canvases the running pass declared with `host_canvas`, which the end hook looks up among egui's widgets.
    host_canvases: SmallVec<[Id; 1]>,
    /// A text widget that gave its Escape on to the enclosing scopes and lets focus go at the end of the pass.
    surrender_after_scopes: Option<Id>,
    /// A text widget in a scope that gives focus back to its opener, held one more pass: the scope the Escape closes is
    /// closing by the next pass, and its restoration needs the widget to still hold the focus.
    surrender_deferred: Option<Id>,
}

fn viewport(ctx: &Context) -> ViewportId {
    crate::runtime::egui::pass::current_viewport(ctx)
}

pub(crate) fn with<R>(ctx: &Context, f: impl FnOnce(&mut State) -> R) -> R {
    PerViewport::<State>::with(ctx, viewport(ctx), f)
}

pub(crate) fn with_viewport<R>(ctx: &Context, viewport: ViewportId, f: impl FnOnce(&mut State) -> R) -> R {
    PerViewport::<State>::with(ctx, viewport, f)
}

/// Counts the focus writes of the process. A component's `Facts` carry the count they were read at, so a write
/// during its pass (a request, a layer taking focus into itself) shows as a changed count and the facts are read
/// again. Another context's write only causes an extra read.
static WRITES: AtomicU64 = AtomicU64::new(0);

pub(crate) fn writes() -> u64 {
    WRITES.load(Ordering::Acquire)
}

pub(crate) fn apply(ctx: &Context, acts: Vec<Act>) {
    if !acts.is_empty() {
        WRITES.fetch_add(1, Ordering::AcqRel);
    }
    for act in acts {
        match act {
            Act::Focus(id, filter) => {
                egui_focus::focus_with_filter(ctx, id, filter);
                with(ctx, |state| state.given = Some(id));
            }
            Act::Surrender(id) => egui_focus::surrender(ctx, id),
        }
    }
}

/// Registers a stop of the running pass in the innermost open scope, in registration order: after the app's own
/// widgets drawn before it (`take_app_widgets`). The app's own widget `stop` names (`host_canvas`) takes the place of
/// its stop as one of those widgets, if the pass took it already; focus the runtime gave that stop follows the canvas's
/// keys from then on.
pub(crate) fn register_stop(ctx: &Context, stop: Stop) {
    take_app_widgets(ctx, Some(stop.root));
    let held = stop.host && egui_focus::has_focus(ctx, stop.root);
    let acts = with(ctx, |state| {
        if stop.host
            && let Some(rec) = state.stops.iter_mut().find(|rec| rec.stop.host && rec.stop.id == stop.id)
        {
            state.stop_roots.insert(stop.root);
            let acts = if held && state.given == Some(stop.root) { vec![nav::focus_act(&stop)] } else { Vec::new() };
            rec.stop = stop;
            return acts;
        }
        state.push_stop(stop)
    });
    apply(ctx, acts);
}

impl State {
    /// Appends `stop` to the running pass's stops in the innermost open scope; it takes the focus a request of the pass
    /// awaits for it.
    fn push_stop(&mut self, stop: Stop) -> Vec<Act> {
        let scope = self.open.last().map(|(id, _)| *id);
        self.stop_ids.insert(stop.id);
        self.stop_roots.insert(stop.root);
        let rec = StopRec { stop, scope, group_position: false };
        let acts = if self.awaited == Some(rec.stop.key()) {
            self.awaited = None;
            self.focus_stop(rec.clone())
        } else {
            Vec::new()
        };
        self.stops.push(rec);
        acts
    }
}

/// Registers the app's own egui widgets that egui listed as focus candidates since the last take of the running pass
/// (`Memory::focus_candidates`, in the order they registered with egui), each as a stop of the app's own
/// (`Stop::host`) with the keys of a pointer control, a text stop for a text edit: every focusable egui widget the app
/// draws is a Tab and F6 stop among the framework's, where it was drawn. A widget a component made is none: its
/// component registers its stops (`pointer::made_by_component`, or the root of a stop of the pass), and so is
/// `except`, the root of the stop being registered. Every registration that depends on order takes them first, and
/// the end hook takes the last ones, so a stop range (`mark`, `stop_count`) holds the app's widgets drawn in it.
fn take_app_widgets(ctx: &Context, except: Option<Id>) {
    let seen = with(ctx, |state| state.candidates_seen);
    let fresh = egui_focus::candidates_since(ctx, seen);
    if fresh.is_empty() {
        return;
    }
    let viewport = viewport(ctx);
    let app: Vec<egui::FocusCandidate> = fresh
        .iter()
        .filter(|candidate| Some(candidate.id) != except && !crate::runtime::pointer::made_by_component(ctx, viewport, candidate.id))
        .copied()
        .collect();
    let acts = with(ctx, |state| {
        state.candidates_seen = seen + fresh.len();
        let mut acts = Vec::new();
        for candidate in app {
            if state.stop_roots.contains(&candidate.id) {
                continue;
            }
            let stop = Stop::new(candidate.id, KeyUse::of(craft_behavior::Input::Pointer)).host();
            acts.extend(state.push_stop(if candidate.text { stop.text() } else { stop }));
        }
        acts
    });
    apply(ctx, acts);
}

/// Registers the app's own egui widget `id`, drawn in the running pass, as its window's document canvas: its focus stop
/// takes `keys` and is marked as the canvas (`KeyUse::chrome_key`), in place of the stop it has as one of the app's
/// focusable egui widgets, at the same place in Tab order. The window then shows a document, so with
/// `RootEnv::tab_toggles_chrome` a bare Tab with nothing focused toggles its panels, as with a framework canvas. Focus
/// reaches the canvas by Tab, F6, `request` or the app's own egui focus request; while it has focus, Tab and the arrows
/// route as `keys` says (the arrows as nudges, Tab moving focus on or toggling the panels unless it keeps Tab). `keys`
/// is `KeyUse::CANVAS`, or a variant such as `KeyUse::CANVAS.with_keep(KeepKeys::Tab).with_edits(..)` while Tab and the
/// clipboard are the canvas's own. The canvas reads its keys from egui: with nothing focused it is the key target, and
/// the chords it claims (`KeyUse::with_claims`; Alt+numpad code entry leaves a claimed Alt+digit alone) and the
/// clipboard events and edit chords of its edit set stay in egui's input for the app, and nothing toggles or fires with
/// them; Page Up and Down, Home and End arrive as its `Intent::Move`, as while it has the focus. A bare Tab with nothing
/// focused is never the canvas's, even when it keeps Tab: it moves focus into the window or toggles the panels, so the
/// canvas cannot trap keyboard navigation. With the canvas focused, the same keys stay in egui, a Tab it keeps and
/// Escape too, as for any focused widget of the app's own; Escape with nothing focused is the window's cancel.
///
/// A press that nothing takes leaves the canvas focused, whoever gave it the focus and whatever the app's
/// `surrender_focus_on`: a press on the app's own widgets that take no focus or on empty space moves no focus. The canvas
/// loses the focus to a component or widget that takes it, Tab, F6, a dialog, the app's own request or surrender, and
/// when it is no longer drawn.
///
/// Call it in each pass the canvas shows, before or after the canvas is drawn, with the id of the focusable, enabled
/// egui widget the app draws for it in every such pass (`Sense::click()`, `Sense::drag()`; the bare flags `Sense::CLICK`
/// and `Sense::DRAG` are not focusable). egui drops the focus of an id that no such widget of the pass has, so Tab would
/// lose the focus at the canvas and start over at the window's first stop. A pass that declares a canvas and draws no
/// such widget reports the finding `canvas-without-widget`, and debug builds log it once; a canvas drawn in an invisible
/// `Ui` (an egui sizing pass) is not reported as disabled. A pass that hides or disables the canvas makes no call.
pub fn host_canvas(ctx: &Context, id: Id, keys: KeyUse) {
    if !crate::runtime::measure::active(ctx) {
        with(ctx, |state| state.host_canvases.push(id));
    }
    register_stop(ctx, Stop::new(id, keys.with_chrome_key(true)).host());
}

/// The stop of the document canvas `viewport` registered in its last pass (`KeyUse::chrome_key`), framework or the
/// app's own (`Stop::host`). Read before the begin hook rotates the registrations, by the input hook and the begin
/// hook's Tab moves.
pub(crate) fn canvas_stop(ctx: &Context, viewport: ViewportId) -> Option<Stop> {
    with_viewport(ctx, viewport, |state| state.stops.iter().find(|rec| rec.stop.keys.chrome_key).map(|rec| rec.stop.clone()))
}

/// The document canvas the running viewport registered in its last pass (`KeyUse::chrome_key`): the window's key
/// target while nothing has the focus. Read by the begin hook, after it rotated the registrations.
pub(crate) fn document_canvas(ctx: &Context) -> Option<StopRec> {
    with(ctx, |state| state.last_stops.iter().find(|rec| rec.stop.keys.chrome_key).cloned())
}

/// Whether `rec`, the focused stop, is a canvas control's: a non-text member of the document canvas's arena, as the
/// end hook recorded the focus (`FocusedRecord::root`). Its leftover Escape is the window's, as the canvas's own is.
pub(crate) fn is_canvas_member(ctx: &Context, rec: &StopRec) -> bool {
    with(ctx, |state| {
        state.record.focused == Some(rec.stop.id)
            && state
                .record
                .root
                .is_some_and(|root| state.stops.iter().chain(&state.last_stops).any(|canvas| canvas.stop.keys.chrome_key && canvas.stop.root == root))
    })
}

/// Whether a bare Tab in `viewport` has no stop to move the focus to: the window's last pass registered no stop (the
/// app's focusable egui widgets are stops too, `take_app_widgets`) and nothing held the focus at its end. Such a Tab
/// stays in egui's input for the app's own binding. Read by the input hook, before the begin hook rotates the
/// registrations.
pub(crate) fn tab_has_no_stop(ctx: &Context, viewport: ViewportId) -> bool {
    with_viewport(ctx, viewport, |state| state.stops.is_empty() && state.focus_now.is_none())
}

/// How many stops the running pass has registered so far.
pub(crate) fn stop_count(ctx: &Context) -> usize {
    take_app_widgets(ctx, None);
    with(ctx, |state| state.stops.len())
}

/// Where the running pass's registrations stand, for a [`Transcript`] of what a run registers after it.
#[derive(Clone, Copy)]
pub(crate) struct Mark {
    stops: usize,
    scopes: usize,
    row_sets: usize,
    /// The innermost open scope.
    outer: Option<Id>,
}

/// What one run of a child registered with the focus service: its stops, the scopes it pushed, the roving groups
/// its stops belong to and its collections' row groups. A child out of view registers it again without running
/// (`runtime::lazy`), so its stops keep their place in Tab and F6 order. Two transcripts are equal when their replays
/// register the same records under the same open scope (`Transcript::placed`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Transcript {
    stops: Vec<StopRec>,
    scopes: Vec<(Id, ScopeRec)>,
    groups: Vec<Group>,
    row_sets: Vec<RowGroups>,
    /// The scope that was open around the run: a record under it goes under the scope open around the replay.
    outer: Option<Id>,
}

impl Transcript {
    /// The components of the stops.
    pub(crate) fn stop_ids(&self) -> impl Iterator<Item = Id> + '_ {
        self.stops.iter().map(|rec| rec.stop.id)
    }

    /// The transcript as a replay registers it with `outer` the innermost open scope: a record under the scope that
    /// was open around the run goes under `outer`, a replayed scope is no new scope (`entering` is false), and its
    /// opener is the focus service's, not the run's (`opener`).
    fn placed(&self, outer: Option<Id>) -> Transcript {
        let place = |scope: Option<Id>| if scope == self.outer { outer } else { scope };
        Transcript {
            stops: self
                .stops
                .iter()
                .map(|rec| StopRec { stop: rec.stop.clone(), scope: place(rec.scope), group_position: rec.group_position })
                .collect(),
            scopes: self
                .scopes
                .iter()
                .map(|(id, scope)| {
                    (*id, ScopeRec { parent: place(scope.parent), entering: scope.entering.map(|_| false), opener: None, ..scope.clone() })
                })
                .collect(),
            groups: self.groups.clone(),
            row_sets: self.row_sets.clone(),
            outer,
        }
    }
}

impl Transcript {
    /// The first record in which the replays of `self` and `other` differ (`placed`): the kind, its index and both
    /// sides, `None` on a side that has fewer records. `None` when they register the same.
    pub(crate) fn first_difference(&self, other: &Self) -> Option<String> {
        fn first<T: PartialEq + fmt::Debug>(kind: &str, left: &[T], right: &[T]) -> Option<String> {
            let index =
                left.iter().zip(right).position(|(a, b)| a != b).or_else(|| (left.len() != right.len()).then(|| left.len().min(right.len())))?;
            Some(format!("{kind} {index}: left {:?} / right {:?}", left.get(index), right.get(index)))
        }
        let [a, b] = [self.placed(None), other.placed(None)];
        first("stop", &a.stops, &b.stops)
            .or_else(|| first("scope", &a.scopes, &b.scopes))
            .or_else(|| first("roving group", &a.groups, &b.groups))
            .or_else(|| first("row groups", &a.row_sets, &b.row_sets))
    }
}

impl PartialEq for Transcript {
    fn eq(&self, other: &Self) -> bool {
        self.first_difference(other).is_none()
    }
}

impl Eq for Transcript {}

/// Where the running pass's registrations stand.
pub(crate) fn mark(ctx: &Context) -> Mark {
    take_app_widgets(ctx, None);
    with(ctx, |state| Mark {
        stops: state.stops.len(),
        scopes: state.scope_order.len(),
        row_sets: state.row_sets.len(),
        outer: state.open.last().map(|(id, _)| *id),
    })
}

/// What the running pass registered since `mark`, which a run that closed every scope it opened left.
pub(crate) fn transcript(ctx: &Context, mark: Mark) -> Transcript {
    take_app_widgets(ctx, None);
    with(ctx, |state| {
        let stops = state.stops.get(mark.stops..).unwrap_or_default().to_vec();
        let scopes = state.scope_order.get(mark.scopes..).unwrap_or_default();
        let scopes = scopes.iter().filter_map(|id| Some((*id, state.scopes.get(id)?.clone()))).collect();
        let mut groups: Vec<Group> = Vec::new();
        for group in stops.iter().filter_map(|rec| state.groups.get(&rec.stop.group?)) {
            if !groups.iter().any(|known| known.id == group.id) {
                groups.push(*group);
            }
        }
        let row_sets = state.row_sets.get(mark.row_sets..).unwrap_or_default().to_vec();
        Transcript { stops, scopes, groups, row_sets, outer: mark.outer }
    })
}

/// Registers `transcript` again in the running pass, in place of the run that left it: in the innermost open scope,
/// as the run would have. A replayed scope is no new scope: it moves no focus when it shows again.
pub(crate) fn replay(ctx: &Context, transcript: &Transcript) {
    take_app_widgets(ctx, None);
    let frame = crate::runtime::egui::pass::frame_nr(ctx, viewport(ctx));
    let focused = egui_focus::focused(ctx);
    with(ctx, |state| {
        let placed = transcript.placed(state.open.last().map(|(id, _)| *id));
        for (id, mut scope) in placed.scopes {
            // A replayed scope keeps the opener the last pass held: an overlay's scope, the one kind that notes an opener,
            // took it in the pass that first showed the layer, which ran.
            scope.opener = opener(state, id, None, focused);
            if scope.entering.is_none() {
                state.seen.insert(id, frame);
            }
            state.scopes.insert(id, scope);
            state.scope_order.push(id);
        }
        for rec in placed.stops {
            state.stop_ids.insert(rec.stop.id);
            state.stop_roots.insert(rec.stop.root);
            state.stops.push(rec);
        }
        for group in placed.groups {
            state.groups.insert(group.id, group);
        }
        state.row_sets.extend(placed.row_sets);
    });
}

/// The component of the stop that holds focus and the component a request or Tab move asked its scroll owner to
/// reveal. The focused stop, not the egui widget: the members of an arena share their root widget.
pub(crate) fn targets(ctx: &Context) -> [Option<Id>; 2] {
    [focused_stop(ctx).map(|key| key.0), with(ctx, |state| state.revealed.map(|key| key.0))]
}

/// Whether the component `id` registered a stop this pass that Tab or a request can reach, its own or one of its
/// parts': a segmented control's stops are its segments. A press-only stop (the selection of a display text) is not one.
pub(crate) fn has_reachable_stop(ctx: &Context, id: Id) -> bool {
    with(ctx, |state| state.stops.iter().any(|rec| rec.stop.id == id && !rec.stop.press_only))
}

/// Where a request for the component `id` goes when it has no stop of its own: the first stop registered since
/// `from` (a `stop_count`), which a component inside it holds.
pub(crate) fn first_stop_inside(ctx: &Context, id: Id, from: usize) -> Option<FocusTarget> {
    take_app_widgets(ctx, None);
    with(ctx, |state| {
        if state.stop_ids.contains(&id) {
            return None;
        }
        state.stops.get(from..)?.first().map(target_of)
    })
}

/// The first stop the running pass registered in `range` (of `stop_count`s) that Tab and a request reach.
pub(crate) fn first_stop_in(ctx: &Context, range: std::ops::Range<usize>) -> Option<FocusTarget> {
    with(ctx, |state| state.stops.get(range.start..range.end.min(state.stops.len()))?.iter().find(|rec| !rec.stop.press_only).map(target_of))
}

/// The request target of the stop `rec`.
fn target_of(rec: &StopRec) -> FocusTarget {
    match rec.stop.part {
        Some(part) => FocusTarget::Part(rec.stop.id, part),
        None => FocusTarget::Id(rec.stop.id),
    }
}

/// Whether focus is on a stop the running pass registered in `range` (of `stop_count`s).
pub(crate) fn focus_in(ctx: &Context, range: std::ops::Range<usize>) -> bool {
    let Some(focused) = focused_stop(ctx) else { return false };
    with(ctx, |state| {
        state.stops.get(range.start..range.end.min(state.stops.len())).is_some_and(|recs| recs.iter().any(|rec| rec.stop.key() == focused))
    })
}

/// Marks the stops registered from here on in the innermost open scope as its body's: they come first in Tab
/// order, then the footer's, then the header's, and initial focus goes to the first of them, not to the title
/// row's close action. The first mark of a pass stands.
pub(crate) fn begin_body(ctx: &Context) {
    take_app_widgets(ctx, None);
    with(ctx, |state| {
        if let Some((scope, _)) = state.open.last().copied() {
            let at = state.stops.len();
            state.body_from.entry(scope).or_insert(at);
        }
    });
}

/// Registers a roving group of the running pass.
pub(crate) fn register_group(ctx: &Context, group: Group) {
    with(ctx, |state| {
        state.groups.insert(group.id, group);
    });
}

/// Registers the row groups `ids` of the collection `owner` for the running pass.
pub(crate) fn register_row_groups(ctx: &Context, owner: Id, rtl: bool, mut ids: Vec<Id>) {
    ids.sort_unstable_by_key(|id| id.value());
    with(ctx, |state| state.row_sets.push(RowGroups { owner, rtl, ids }));
}

/// Marks the component `id`'s own stop of this pass in the roving group `group` as taking its place in the set from
/// the group's members (`group_positions`). A component with no such stop this pass keeps the place it reported.
pub(crate) fn takes_group_position(ctx: &Context, id: Id, group: Id) {
    with(ctx, |state| {
        if let Some(rec) = state.stops.iter_mut().rev().find(|rec| rec.stop.id == id && rec.stop.part.is_none() && rec.stop.group == Some(group)) {
            rec.group_position = true;
        }
    });
}

/// The places of the members of the roving group `group` that take theirs from the group (`takes_group_position`),
/// among this pass's members: the components with their own stop in it, in registration order. The group's owner
/// asks at its report, after its body's members have run.
pub(crate) fn group_positions(ctx: &Context, group: Id) -> Vec<(Id, crate::runtime::access::SetPosition)> {
    with(ctx, |state| {
        let members: Vec<&StopRec> = state.stops.iter().filter(|rec| rec.stop.group == Some(group) && rec.stop.part.is_none()).collect();
        let count = members.len();
        members
            .iter()
            .enumerate()
            .filter(|(_, rec)| rec.group_position)
            .map(|(index, rec)| (rec.stop.id, crate::runtime::access::SetPosition::new(index).of(count)))
            .collect()
    })
}

/// Opens the scope `id` for the stops registered until the matching [`pop_scope`].
pub(crate) fn push_scope(ctx: &Context, id: Id, policy: FocusPolicy, label: Option<Arc<str>>) {
    push_scope_from(ctx, id, policy, label, None, false, None);
}

/// `push_scope` for a scope whose opener the caller noted before egui's modal gate took focus away from it, and
/// which `closing` marks as playing its exit. A layer's scope passes whether its layer `appears` in this pass.
pub(crate) fn push_scope_from(
    ctx: &Context,
    id: Id,
    policy: FocusPolicy,
    label: Option<Arc<str>>,
    noted: Option<StopKey>,
    closing: bool,
    entering: Option<bool>,
) {
    take_app_widgets(ctx, None);
    let focused = egui_focus::focused(ctx);
    with(ctx, |state| {
        let parent = state.open.last().map(|(parent, _)| *parent);
        let opener = opener(state, id, noted, focused);
        state.scopes.insert(id, ScopeRec { policy, parent, label, opener, closing, entering });
        state.scope_order.push(id);
        let first = state.stops.len();
        state.open.push((id, first));
    });
}

/// The opener of the scope `id` registered in the running pass, by a run or a replay alike: taken once, when the scope
/// first appears, as the stop `noted` or else the stop of the focused widget `focused`; then kept from the pass before,
/// with what focus entering it noted there (`note_entry`). It is never taken again from the focus of a later pass,
/// which may be a stop inside the scope itself.
fn opener(state: &State, id: Id, noted: Option<StopKey>, focused: Option<Id>) -> Option<StopKey> {
    match state.last_scopes.get(&id) {
        Some(last) => last.opener,
        None => noted.or_else(|| focused.map(|root| state.focused.get(&root).copied().unwrap_or((root, None)))),
    }
}

/// The innermost scope open in the running pass: the scope whose body is being shown.
#[doc(hidden)]
pub fn open_scope(ctx: &Context) -> Option<Id> {
    with(ctx, |state| state.open.last().map(|(id, _)| *id))
}

/// Closes the innermost open scope. A scope pushed for the first time (or again after a gap of a frame)
/// moves focus to its initial stop, in this pass.
pub(crate) fn pop_scope(ctx: &Context) {
    take_app_widgets(ctx, None);
    let frame = crate::runtime::egui::pass::frame_nr(ctx, viewport(ctx));
    let acts = with(ctx, |state| {
        nav::order_regions(state);
        nav::initial_focus(state, frame)
    });
    apply(ctx, acts);
}

/// Registers the component `id` as the menu bar that can take the menu-bar keys of the next input hook (spec 9.7).
/// Its claims hold as the focused component's do, so a bar that is not a focus stop (keytips) keeps the letters it
/// takes from the focused field. The last registration of a pass wins. A settled bar registers again when it replays
/// its last run, and the keys it takes run it (`keys::routed_to`).
///
/// With `by_claim` the bar has no mnemonic letters of its own: an Alt+letter reaches it only as a chord it claims, and
/// any other stays the app's shortcut. A menu bar's titles name their letters, so it registers without.
pub(crate) fn register_menu_bar(ctx: &Context, id: Id, by_claim: bool) {
    crate::runtime::replay::journal(ctx, || (id, by_claim), |ctx, &(id, by_claim)| register_menu_bar(ctx, id, by_claim));
    with(ctx, |state| {
        state.menu_bar = Some(id);
        state.menu_bar_by_claim = by_claim;
    });
}

/// Whether a stop with these keys takes typed letters: a text field or a shortcut recorder. The letters a keytip or a
/// menu mnemonic would take are then the stop's own. A document canvas's editor part (`KeyUse::is_document_editor`)
/// does not count: the menu bar and keytips work over the document, as a word processor's do, and its letters still
/// type while no lone Alt or F10 has started them.
pub fn takes_letters(keys: &KeyUse) -> bool {
    matches!(keys.owner, OwnerClass::Text | OwnerClass::Recorder) && !keys.is_document_editor()
}

/// A registered menu bar takes the keyboard (F10, a lone Alt): focus moves to `target`, a stop of the bar, in this
/// pass, and the stop that held it is remembered for [`give_back_bar`]. With nothing focused, that is the window's
/// document canvas, the key target then: a shortcut pressed on the bar (Space, the canvas's momentary Hand) matches
/// there and gives the keyboard back to it, as Escape does. Focus already inside the bar leaves the remembered stop
/// as it is.
#[doc(hidden)]
pub fn take_for_bar(ctx: &Context, bar: Id, target: FocusTarget) {
    let held = focused_stop(ctx).filter(|(id, _)| *id != bar);
    let in_bar = held.is_none() && is_focused(ctx, bar);
    with(ctx, |state| {
        let canvas = || state.stops.iter().chain(&state.last_stops).find(|rec| rec.stop.keys.chrome_key).map(|rec| rec.stop.key());
        let held = held.or_else(|| (!in_bar).then(canvas).flatten());
        if held.is_some() || state.bar_return.is_none() {
            state.bar_return = held.map(|held| (bar, held));
        }
    });
    request(ctx, target);
}

/// Returns focus to the stop a menu bar took it from, in this pass. `false` when there is none.
#[doc(hidden)]
pub fn give_back_bar(ctx: &Context) -> bool {
    let Some((_, (id, part))) = with(ctx, |state| state.bar_return.take()) else { return false };
    request(ctx, part.map_or(FocusTarget::Id(id), |part| FocusTarget::Part(id, part)));
    true
}

/// The stop a menu bar took the focus from, while the bar's stop `focused` holds it: where a shortcut pressed on the
/// bar matches and goes.
pub(crate) fn bar_return_of(ctx: &Context, focused: Id) -> Option<StopKey> {
    with(ctx, |state| state.bar_return.as_ref().filter(|(bar, _)| *bar == focused).map(|(_, key)| *key))
}

/// The stop the keyboard goes back to while `focused` holds it inside open menus or on a menu bar, and whether a menu
/// bar took it from that stop: a shortcut pressed there matches at that stop, and a text part keeps its selection for
/// the menus' picks. `None` when the focus is in neither, or a menu opened with nothing focused.
pub(crate) fn returning(ctx: &Context, focused: Id) -> Option<(StopKey, bool)> {
    match focused_rec(ctx).and_then(|rec| out_of_menus(ctx, rec.scope)) {
        Some(out) => Some((out.to, out.bar)),
        None => bar_return_of(ctx, focused).map(|held| (held, true)),
    }
}

/// Where focus goes once every menu around the scope `from` closes, as `out_of_menus` finds it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OutOfMenus {
    /// The stop focus goes to.
    pub(crate) to: StopKey,
    /// `to` is the stop a menu bar took the focus from, which `give_back_bar` gives it back to.
    pub(crate) bar: bool,
    /// The outermost menu of the chain: the layer whose closing ends it.
    pub(crate) root: Id,
}

/// Where focus goes once the menus around the scope `from` close (`State::out_of_menus`), and from a menu bar reached
/// that way on to the stop the bar took the focus from. A shortcut pressed in a menu matches there, a component inside
/// a menu resolves its bound command from there, and a pick returns focus there (`return_from_pick`), as with the
/// menus closed.
pub(crate) fn out_of_menus(ctx: &Context, from: Option<Id>) -> Option<OutOfMenus> {
    let (to, root) = with(ctx, |state| state.out_of_menus(from).map(|(rec, root)| (rec.stop.key(), root)))?;
    Some(match bar_return_of(ctx, to.0) {
        Some(held) => OutOfMenus { to: held, bar: true, root },
        None => OutOfMenus { to, bar: false, root },
    })
}

/// What an activation inside the layer `layer` closes: `layer` and the menus the walk out of it crosses
/// (`State::walk_out`), innermost first. A popup opened from a stop that is no menu row, such as a field inside a
/// menu, closes alone.
pub(crate) fn menus_closed_from(ctx: &Context, layer: Id) -> Vec<Id> {
    with(ctx, |state| state.walk_out(layer).crossed)
}

/// Forgets the stop a menu bar took focus from: focus left the bar by other means, so a later Escape has nowhere
/// to return to.
#[doc(hidden)]
pub fn forget_bar_return(ctx: &Context) {
    with(ctx, |state| state.bar_return = None);
}

/// What a pick closes before its command runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub enum PickedIn {
    /// The layer the pick ran in and the menus the walk out of it crosses (`menus_closed_from`).
    Menus(Id),
    /// The modal layer the pick was made in (the command palette).
    Layer(Id),
}

/// A pick runs, or another activation closes the menus: in this pass, before the picked command fires, focus
/// returns from what the pick closes to the stop that held it before the outermost menu or the layer opened, or before
/// a menu bar took it. The command then reaches that stop, as an Edit command picked from a menu reaches the text
/// field it edits. Focus in nothing the pick closes stays.
#[doc(hidden)]
pub fn return_from_pick(ctx: &Context, picked: PickedIn) {
    let focused = egui_focus::focused(ctx);
    let acts = with(ctx, |state| {
        state.focus_now = focused;
        state.picked_focus(picked).map(|rec| state.focus_stop(rec)).unwrap_or_default()
    });
    apply(ctx, acts);
}

/// The stop focus returns to when the scope `scope` closes, and its scope chain, innermost first.
pub(crate) fn return_stop(ctx: &Context, scope: Id) -> Option<(StopRec, Vec<Id>)> {
    let rec = with(ctx, |state| state.return_stop(scope))?;
    let chain = scope_chain(ctx, &rec);
    Some((rec, chain))
}

/// Registers the component that `FocusTarget::Doc(key)` focuses.
pub(crate) fn register_doc(ctx: &Context, key: DocKey, id: Id) {
    with(ctx, |state| {
        state.docs.insert(key, id);
    });
}

/// Moves focus to `target`, leaves modality unchanged and asks the enclosing scroll owner to reveal it. Called
/// while a pass runs, it moves focus in that pass, so the rest of the pass reads the new focus; called between
/// passes, it moves focus at the next pass's begin hook; a pointer press of the pass moves it no further at the pass's
/// end. The one public path to programmatic focus. `false` in a measuring
/// run, which moves nothing: a caller that keeps a pending request keeps it until a run takes it.
pub fn request(ctx: &Context, target: FocusTarget) -> bool {
    if !crate::runtime::measure::effect(ctx, "focus::request") {
        return false;
    }
    if crate::runtime::frame::in_pass(ctx) {
        with(ctx, |state| state.asked = true);
        request_now(ctx, target);
        return true;
    }
    with(ctx, |state| state.pending = Some(target));
    crate::runtime::egui::repaint::request_of(ctx, crate::runtime::egui::pass::current_viewport(ctx));
    true
}

/// Takes the one-shot request of the component `id` (`Props::request_focus`) right after its stops registered in the
/// running pass: its own stop, else its first part stop, takes the focus as a layer's initial stop does, so the press
/// of the pass does not take it back. A component with no stop this pass (a disabled one) takes none, and a
/// measuring run moves nothing.
pub(crate) fn take_request(ctx: &Context, id: Id, request: FocusRequest) {
    if !crate::runtime::measure::effect(ctx, "focus::take_request") {
        return;
    }
    if request == FocusRequest::WhenNone && egui_focus::focused(ctx).is_some() {
        return;
    }
    let acts = with(ctx, |state| {
        let own = state.stops.iter().find(|rec| rec.stop.key() == (id, None));
        let Some(rec) = own.or_else(|| state.stops.iter().find(|rec| rec.stop.id == id)).cloned() else { return Vec::new() };
        state.entered = true;
        state.focus_stop(rec)
    });
    apply(ctx, acts);
}

/// Whether focus is on the component `id` or on one of its parts (a canvas's editor part), from the focus record of
/// the stop focus is on. An app asks it of a component it shows, such as whether its document canvas has focus.
pub fn has(ctx: &Context, id: Id) -> bool {
    focused_rec(ctx).is_some_and(|rec| rec.stop.id == id)
}

/// Applies `target` now, for an AccessKit `Focus` action, which moves focus in the pass that carries it.
pub(crate) fn request_now(ctx: &Context, target: FocusTarget) {
    let acts = with(ctx, |state| state.apply_target(target));
    apply(ctx, acts);
}

/// Whether egui key focus is on the widget `root` for another component's stop: an arena member, whose stop's widget
/// is its root's (`Stop::root`).
pub(crate) fn held_for_member(ctx: &Context, root: Id) -> bool {
    egui_focus::focused(ctx) == Some(root) && focused_stop(ctx).is_some_and(|(id, _)| id != root)
}

/// Whether `at` (screen points) lies in a popup layer opened above the layer of the widget that holds egui focus: a
/// press or a wheel there is the popup's, not the focused widget's. The layer under `at` is the layer test's; the
/// focused widget's layer is egui's last-pass record of it, since a stop records no layer of its own.
pub(crate) fn popup_above_focus(ctx: &Context, at: egui::Pos2) -> bool {
    let focus_layer = egui_focus::focused(ctx).and_then(|widget| ctx.read_response(widget)).map(|response| response.layer_id);
    crate::runtime::egui::areas::layer_at(ctx, at)
        .zip(focus_layer)
        .is_some_and(|(layer, below)| crate::runtime::overlay::popup_above(ctx, layer, below))
}

/// The stop focus is on, in the viewport whose pass is running: the focused member or part of the egui
/// root, else the root itself.
pub(crate) fn focused_stop(ctx: &Context) -> Option<StopKey> {
    let root = egui_focus::focused(ctx)?;
    with(ctx, |state| {
        let recorded = state.focused.get(&root).copied();
        // A text part's egui widget is not its component's id: the stop registered under that root names it.
        let registered = || state.stops.iter().chain(&state.last_stops).find(|rec| rec.stop.root == root).map(|rec| rec.stop.key());
        Some(recorded.or_else(registered).unwrap_or((root, None)))
    })
}

/// The egui widgets that hold focus for last pass's press-only stops, which only a press reaches, in stop order.
#[cfg(feature = "testing")]
pub(crate) fn press_only_roots(ctx: &Context) -> Vec<Id> {
    with(ctx, |state| state.last_stops.iter().filter(|rec| rec.stop.press_only).map(|rec| rec.stop.root).collect())
}

/// The stop this pass's press takes focus for. The end hook moves focus there, after a layer the press opens has
/// recorded its opener.
pub(crate) fn pressed_stop(ctx: &Context) -> Option<StopKey> {
    with(ctx, |state| state.pressed_stop())
}

/// The registration of the stop `key`.
pub(crate) fn stop_rec(ctx: &Context, key: StopKey) -> Option<StopRec> {
    with(ctx, |state| state.find_stop(key))
}

/// The registration of the stop focus is on.
#[doc(hidden)]
pub fn focused_rec(ctx: &Context) -> Option<StopRec> {
    crate::runtime::replay::note_input(ctx, crate::runtime::inputs::Input::Focus);
    let focused = egui_focus::focused(ctx);
    with(ctx, |state| {
        state.focus_now = focused;
        state.stop_of_focus()
    })
}

/// Whether the focus stop is a child of a collection row's roving group.
pub(crate) fn in_row_child(ctx: &Context) -> bool {
    with(ctx, |state| {
        let group = state.stop_of_focus().and_then(|rec| rec.stop.group);
        group.and_then(|group| state.group(group)).is_some_and(|group| group.row)
    })
}

/// Whether the stop `stop` is a child or part of a collection row's roving group: it activates itself.
pub(crate) fn is_row_child(ctx: &Context, stop: &Stop) -> bool {
    with(ctx, |state| stop.group.and_then(|group| state.group(group)).is_some_and(|group| group.row))
}

/// Whether the focus stop is the component `id`: the focused member or item of its arena, or a root that
/// holds focus while none of its members is the stop. A root's own focused part counts as the root.
pub fn is_focused(ctx: &Context, id: Id) -> bool {
    focused_stop(ctx).is_some_and(|(focused, _)| focused == id)
}

/// What moved focus onto a stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FocusOrigin {
    /// A pointer press on the stop.
    Press,
    /// The keyboard (Tab, arrows, Page keys) or a focus request.
    Request,
    /// A scope shown for the first time moved focus to its initial stop (a dialog's first field): a request too, so
    /// it counts as an arrival by key, and a text part selects its whole text so typing replaces it.
    Open,
    /// The focused child of a collection row went away and handed focus back to its row: neither the keyboard nor a
    /// request chose it, so it is no arrival by key and no scroll owner reveals it.
    Removal,
}

/// What a component reads of focus in one pass.
#[derive(Clone, Copy, Default)]
#[doc(hidden)]
pub struct Facts {
    /// `is_focused`.
    #[doc(hidden)]
    pub focused: bool,
    /// `focus_visible`.
    pub(crate) visible: bool,
    /// `active_part`.
    pub(crate) part: Option<PartRef>,
    /// The part focus moved onto in this frame, when the component holds focus on it.
    pub(crate) arrived: Option<PartRef>,
    /// What moved focus onto `arrived`.
    pub(crate) arrived_origin: Option<FocusOrigin>,
    /// The count of focus writes when the facts were read; see `writes`.
    pub(crate) at: u64,
}

/// The focus facts of the component `id`, read once.
pub(crate) fn facts(ctx: &Context, id: Id) -> Facts {
    let at = writes();
    match focused_stop(ctx) {
        Some((focused, part)) if focused == id => {
            let (visible, arrived, arrived_origin) = with(ctx, |state| {
                let arrival = state.arrived.filter(|(frame, key, _)| *frame == state.frame && key.0 == id);
                (state.visible, arrival.and_then(|(_, key, _)| key.1), arrival.map(|(.., origin)| origin))
            });
            Facts { focused: true, visible, part, arrived, arrived_origin, at }
        }
        _ => Facts { at, ..Facts::default() },
    }
}

/// What moved focus onto the focused stop `rec`, for as long as it keeps the focus; `None` when the focus service did
/// not move it there (an app's own widget took egui focus).
pub(crate) fn origin(ctx: &Context, rec: &StopRec) -> Option<FocusOrigin> {
    let key = rec.stop.key();
    with(ctx, |state| state.arrived.filter(|(_, arrived, _)| *arrived == key).map(|(.., origin)| origin))
}

/// `is_focused` and focus arrived by keyboard, so the ring shows.
pub(crate) fn focus_visible(ctx: &Context, id: Id) -> bool {
    is_focused(ctx, id) && with(ctx, |state| state.visible)
}

/// The part of `id` that is the focus stop, when one is.
pub(crate) fn active_part(ctx: &Context, id: Id) -> Option<PartRef> {
    focused_stop(ctx).filter(|(focused, _)| *focused == id).and_then(|(_, part)| part)
}

/// The text part that commits its draft in this pass because a press moved focus away from it (8.3).
pub(crate) fn commit_now(ctx: &Context) -> Option<Id> {
    with(ctx, |state| state.press.commit_now)
}

/// Whether the component `owner` opened a layer that keeps the focus on it (a dropdown) and the layer is shown: its
/// list takes Escape before the component does.
pub(crate) fn holds_anchored_layer(ctx: &Context, owner: Id) -> bool {
    with(ctx, |state| {
        state
            .last_scopes
            .values()
            .any(|scope| scope.policy.initial == Initial::KeepAnchor && !scope.closing && scope.opener.is_some_and(|(id, _)| id == owner))
    })
}

/// What moved the key focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FocusMove {
    /// Keyboard navigation: Tab, Shift+Tab or F6.
    Keys,
    /// A press elsewhere, or code.
    Other,
}

/// What moved the focus in this pass: `Keys` when Tab, Shift+Tab or F6 moved it.
pub(crate) fn move_this_pass(ctx: &Context) -> FocusMove {
    if with(ctx, |state| state.press.by_keys) { FocusMove::Keys } else { FocusMove::Other }
}

/// Whether a Tab (`Some(true)`) or Shift+Tab (`Some(false)`) moved focus off the widget `widget` in this pass.
pub(crate) fn tabbed_off(ctx: &Context, widget: Id) -> Option<bool> {
    with(ctx, |state| state.press.tabbed_from.filter(|(from, _)| *from == widget).map(|(_, forward)| forward))
}

/// Sets modality: keyboard use shows focus rings, a pointer press hides them.
pub(crate) fn set_visible(ctx: &Context, viewport: ViewportId, visible: bool) {
    with_viewport(ctx, viewport, |state| state.visible = visible);
}

/// What the input hook may read: the viewport's focus record from its last end hook.
pub(crate) fn record(ctx: &Context, viewport: ViewportId) -> FocusedRecord {
    with_viewport(ctx, viewport, |state| state.record.clone())
}

/// The menu bar the last pass of `viewport` registered for the menu-bar keys.
pub(crate) fn menu_bar(ctx: &Context, viewport: ViewportId) -> Option<Id> {
    with_viewport(ctx, viewport, |state| state.record.menu_bar)
}

/// Whether a text field has the keys in `viewport`, as its last end hook recorded: the focused stop types text, or
/// egui focus is on a text edit the app drew itself.
pub(crate) fn typing(ctx: &Context, viewport: ViewportId) -> bool {
    with_viewport(ctx, viewport, |state| state.record.host_text || state.record.keys.is_some_and(|keys| keys.owner == OwnerClass::Text))
}

/// Whether the last `request` or move of focus landed on component `id`, once, and on which of its parts: the
/// component then asks the enclosing scroll owner to reveal it.
pub(crate) fn take_reveal_of(ctx: &Context, id: Id) -> Option<Option<PartRef>> {
    with(ctx, |state| {
        state.revealed.filter(|key| key.0 == id).map(|key| {
            state.revealed = None;
            key.1
        })
    })
}

/// The stops registered so far in the running pass that keyboard focus reaches, in Tab order, among those of the
/// components `of` takes: each member of a roving group, whatever the platform's Tab setting.
pub(crate) fn tab_order(ctx: &Context, of: impl Fn(Id) -> bool) -> Vec<StopKey> {
    with(ctx, |state| state.tab_order(|rec| of(rec.stop.id)))
}

/// The component the last `request` or Tab move asked the enclosing scroll owner to reveal.
#[cfg_attr(not(feature = "testing"), expect(dead_code, reason = "the component pipeline (F3) registers and reads it"))]
pub(crate) fn take_reveal(ctx: &Context) -> Option<StopKey> {
    with(ctx, |state| state.revealed.take())
}

/// The begin hook: applies a pending request and this pass's Tab and F6 moves, then classifies the press.
pub(crate) fn begin(ctx: &Context, tab_all_controls: bool, dialog_tab: TabReach) {
    let input = BeginIn {
        focused: egui_focus::focused(ctx),
        press: press::press_at(ctx),
        nav: crate::runtime::keys::take_nav(ctx),
        tab_all_controls,
        dialog_tab,
        frame: crate::runtime::egui::pass::frame_nr(ctx, viewport(ctx)),
    };
    let focused = input.focused;
    let (acts, announcements, host_focus) = with(ctx, |state| {
        state.awaited = None;
        let host_focus = state.record.host_focus;
        let (acts, announcements) = nav::begin(state, input);
        (acts, announcements, host_focus)
    });
    // egui's direction (its arrows, a Tab the key stage left to egui) moves focus from the widget it holds when that is
    // an app's own; never in a pass whose focus the framework moves.
    if !host_focus || !acts.is_empty() {
        egui_focus::clear_directional_focus(ctx);
    }
    hold_canvas_focus(ctx, focused);
    apply(ctx, acts);
    for text in announcements {
        crate::runtime::frame::announce(ctx, text);
    }
}

/// Takes the focus `focused` off egui's press rule while it is on the app's document canvas, as the last pass declared
/// it: whoever gave that focus, only `State::apply_press` decides what a press does to it. A canvas the app no longer
/// declares is one of the app's own widgets, whose focus is egui's again.
fn hold_canvas_focus(ctx: &Context, focused: Option<Id>) {
    let (held, left) = with(ctx, |state| {
        let held = focused.filter(|id| nav::declared_canvas(&state.last_stops, *id));
        let before = std::mem::replace(&mut state.held_canvas, held);
        (held, before.filter(|id| held.is_none() && focused == Some(*id)))
    });
    if let Some(id) = held {
        egui_focus::never_surrender(ctx, id);
    } else if let Some(id) = left {
        egui_focus::leave_to_egui(ctx, id);
    }
}

/// Releases the text widget `widget` at the end of this pass, after the enclosing scopes have had its Escape: a scope
/// deciding whether to close still finds the focused stop. No key is replayed and no other widget loses focus.
pub(crate) fn surrender_after_scopes(ctx: &Context, widget: Id) {
    with(ctx, |state| state.surrender_after_scopes = Some(widget));
}

/// Reports each canvas the running pass declared (`host_canvas`) whose id no focusable, enabled egui widget of the pass
/// has. Read at the pass's end, since a canvas may be declared before it is drawn. A canvas hidden for one pass and still
/// declared counts: egui drops its focus at the end of that pass. Only a visible widget counts as disabled: egui disables
/// every widget of an invisible `Ui`, as the sizing pass of an `Area`, a `Window` or a `Grid` has.
fn report_undrawn_canvases(ctx: &Context) {
    for id in with(ctx, |state| state.host_canvases.clone()) {
        let fault = match egui_focus::drawn_widget(ctx, id) {
            None => "no egui widget with the id was drawn",
            Some(widget) if !widget.sense.is_focusable() => "its widget's sense takes no focus",
            Some(widget) if widget.visible && !widget.enabled => "its widget is disabled, which takes no focus",
            Some(_) => continue,
        };
        if cfg!(debug_assertions) {
            log_once(fault.to_owned(), || log::warn!("`host_canvas` declared the canvas {id:?}: {fault}"));
        }
        if inspect::reports(ctx) {
            let subject = format!("{id:?}: {fault}");
            inspect::finding(ctx, inspect::Finding { rule: "canvas-without-widget", component: "host_canvas", subject });
        }
    }
}

/// Leaves the focus the runtime gave an app's own widget (`State::app_widget`) to egui's rule, at the end of a pass that
/// drew the widget with no pointer button down: until then the press, and the click its release makes, of the gesture
/// that gave the focus never takes it back. The record of a focus that has moved, or that egui dropped
/// because no pass drew the widget, ends here too.
fn settle_given(ctx: &Context, app_widget: impl Fn(Id) -> bool) {
    let Some(id) = with(ctx, |state| state.given) else { return };
    if egui_focus::has_focus(ctx, id) {
        let drawn = egui_focus::drawn_widget(ctx, id).is_some();
        if !drawn || (app_widget(id) && crate::runtime::egui::input::button_down(ctx)) {
            return;
        }
        if app_widget(id) {
            egui_focus::leave_to_egui(ctx, id);
        }
    }
    with(ctx, |state| state.given = None);
}

/// The end hook: applies the press's focus move, returns focus to the opener of a scope that went away,
/// moves it off a stop that was not registered, and records what the next input hook reads.
pub(crate) fn end(ctx: &Context) {
    let viewport = viewport(ctx);
    take_app_widgets(ctx, None);
    report_undrawn_canvases(ctx);
    // A field held a pass for the scope it sits in lets go now, unless that scope is going away with focus in the field:
    // nav::end below then gives the focus back to the scope's opener from where the field holds it.
    if let Some(widget) = with(ctx, |state| state.surrender_deferred.take())
        && !with(ctx, |state| state.restored_focus(Some(widget)).is_some())
    {
        egui_focus::surrender(ctx, widget);
    }
    if let Some(widget) = with(ctx, |state| state.surrender_after_scopes.take()) {
        // A field in a scope that restores focus to an opener (a popover, a dialog) may be in the scope its Escape closes,
        // which is only closing by the next pass: it is let go then, once the restoration has had its chance.
        let restoring = with(ctx, |state| {
            let key = state.focused.get(&widget).copied().unwrap_or((widget, None));
            let mut at = state.stops.iter().find(|rec| rec.stop.key() == key).and_then(|rec| rec.scope);
            while let Some(id) = at {
                let Some(scope) = state.scopes.get(&id).or_else(|| state.last_scopes.get(&id)) else { break };
                if scope.opener.is_some() {
                    return true;
                }
                at = scope.parent;
            }
            false
        });
        if restoring {
            with(ctx, |state| state.surrender_deferred = Some(widget));
        } else {
            egui_focus::surrender(ctx, widget);
        }
    }
    let app_widget = |id: Id| {
        let made = crate::runtime::pointer::made_by_component(ctx, viewport, id);
        with(ctx, |state| state.app_widget(id, made))
    };
    let focused = egui_focus::focused(ctx);
    let on_app_widget = focused.is_some_and(app_widget);
    let frame = crate::runtime::egui::pass::frame_nr(ctx, viewport);
    let acts = with(ctx, |state| nav::end(state, focused, on_app_widget, frame));
    apply(ctx, acts);
    settle_given(ctx, app_widget);
    // Escape over a captured framework press or drag cancels it (`frame::begin_pass`), whatever egui has focused.
    let captured = crate::runtime::pointer::press_captured(ctx) || crate::runtime::drag::running(ctx);
    let host_focus = !captured && with(ctx, |state| state.focus_now).is_some_and(app_widget);
    let popup = !captured && crate::runtime::egui::areas::popup_open(ctx);
    let foreign = host_focus || popup;
    let tab_in_egui = popup || (host_focus && egui_focus::focused_keeps_tab(ctx));
    let label_selection = egui_focus::label_selection(ctx);
    let host_text = host_focus && ctx.text_edit_focused();
    // A modal layer keeps the canvas's claims from it, as it keeps the arrows (`overlay::end_pass` ran before).
    let modal = crate::runtime::overlay::top_modal_in(ctx, viewport).is_some();
    with(ctx, |state| {
        state.record.canvas = state.record.canvas.filter(|_| !modal);
        state.record.foreign = foreign;
        state.record.tab_in_egui = tab_in_egui;
        state.record.host_focus = host_focus;
        state.record.host_text = host_text;
        state.record.label_selection = label_selection;
    });
}

/// The scopes `rec` sits in, innermost first.
pub(crate) fn scope_chain(ctx: &Context, rec: &StopRec) -> Vec<Id> {
    with(ctx, |state| {
        let mut chain = Vec::new();
        let mut at = rec.scope;
        while let Some(id) = at {
            chain.push(id);
            at = state.last_scopes.get(&id).and_then(|record| record.parent);
        }
        chain
    })
}

/// Where key focus is, as the tracked input `Focus` compares it: the focused widget and whether its ring shows.
pub(crate) fn focus_state(ctx: &Context) -> (Option<Id>, bool) {
    (egui_focus::focused(ctx), with(ctx, |state| state.visible))
}

/// Whether focus sits inside a menu and arrived by keyboard.
#[doc(hidden)]
pub fn keyboard_in_menu(ctx: &Context) -> bool {
    focused_rec(ctx).is_some_and(|rec| menu_scope(ctx, rec.scope).is_some()) && with(ctx, |state| state.visible)
}

/// Whether the focused stop took the focus when its scope opened (a menu's first row as the menu showed) and has kept
/// it since: no key, press or request (an AccessKit `Focus`, the app's [`request`]) has moved it. A menu row reads it
/// to tell a key that reached the row its menu's opening focused, which may not be meant for it, from a key on a row
/// the user or assistive technology chose.
pub fn focus_came_with_open(ctx: &Context) -> bool {
    focused_rec(ctx).is_some_and(|rec| origin(ctx, &rec) == Some(FocusOrigin::Open))
}

/// The innermost menu scope around the component whose body is running: the menu a row being shown sits in.
#[doc(hidden)]
pub fn running_menu(ctx: &Context) -> Option<Id> {
    menu_scope(ctx, open_scope(ctx))
}

/// The innermost menu scope around the scope `from`, `from` included.
pub(crate) fn menu_scope(ctx: &Context, from: Option<Id>) -> Option<Id> {
    with(ctx, |state| state.menu_scope(from))
}

/// Whether the focused stop sits inside `scope` or a scope opened from inside it; `None` when nothing is focused.
#[doc(hidden)]
pub fn focus_in_scope(ctx: &Context, scope: Id) -> Option<bool> {
    let rec = focused_rec(ctx)?;
    Some(with(ctx, |state| state.within(rec.scope, scope)))
}

/// Whether the focused stop's innermost layer-like scope (groups and panes skipped) is a dialog whose Escape closes it
/// and which is the top layer, so the dialog's own Escape rule decides this pass's Escape. A text field holding a
/// draft there passes Escape on: one Escape cancels the dialog and its typing goes with it, as Word's dialogs do. A
/// field in a popup or menu inside the dialog, or in no dialog (a ribbon box), keeps the Escape to revert its draft.
/// Only a modal dialog counts: a modeless window's edits belong to the app, so its field reverts its own draft first
/// and the next Escape closes the window.
pub(crate) fn escape_closes_dialog(ctx: &Context) -> bool {
    escaping_dialog(ctx).is_some()
}

/// Whether the dialog that [`escape_closes_dialog`] finds keeps the focused field's typing on Escape
/// (`FocusPolicy::escape_keeps_typing`): the field then commits its draft instead of cancelling it.
pub(crate) fn escape_keeps_typing(ctx: &Context) -> bool {
    escaping_dialog(ctx).is_some_and(|policy| policy.escape_keeps_typing)
}

/// The policy of the modal dialog on top whose Escape closes it and which holds the focused stop
/// ([`escape_closes_dialog`]).
fn escaping_dialog(ctx: &Context) -> Option<FocusPolicy> {
    let rec = focused_rec(ctx)?;
    let (id, policy) = with(ctx, |state| {
        let mut at = rec.scope;
        while let Some(id) = at {
            let scope = state.scopes.get(&id).or_else(|| state.last_scopes.get(&id))?;
            match scope.policy.kind {
                craft_behavior::ScopeKind::Group | craft_behavior::ScopeKind::Pane => at = scope.parent,
                craft_behavior::ScopeKind::Dialog => {
                    return (scope.policy.cancel == craft_behavior::CancelRule::Close).then_some((id, scope.policy));
                }
                _ => return None,
            }
        }
        None
    })?;
    let modal_on_top =
        crate::runtime::overlay::top(ctx) == Some(id) && crate::runtime::overlay::entry(ctx, id).is_some_and(|entry| entry.rank.is_modal());
    modal_on_top.then_some(policy)
}

/// Whether the focus is in the focus scope of the layer `layer`, or, for an `Initial::KeepAnchor` scope, on the stop
/// the layer opened from: that scope leaves the focus on its opener while its list is in use. `None` when nothing
/// is focused. The layer's dismissal and its Escape both read it.
pub(crate) fn focus_in_layer(ctx: &Context, layer: Id) -> Option<bool> {
    if focus_in_scope(ctx, layer)? {
        return Some(true);
    }
    let opener = with(ctx, |state| {
        let scope = state.scopes.get(&layer).or_else(|| state.last_scopes.get(&layer))?;
        (scope.policy.initial == Initial::KeepAnchor).then_some(scope.opener).flatten()
    });
    Some(opener.is_some_and(|stop| focused_stop(ctx) == Some(stop)))
}
