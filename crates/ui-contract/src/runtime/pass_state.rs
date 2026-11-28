//! The state the pipeline touches on every run, held by the thread that runs a pass of its context.
//!
//! Between passes a context's `PassState` sits in its context data. The begin hook of a context's outermost pass on
//! a thread moves it into this thread; the end hook of that pass moves it back. Inside a pass a service in it is a
//! `RefCell` borrow of a plain field: no lookup in context data, no handle and no lock. A read made outside every
//! pass of the context (an input hook, a test between frames) moves the state into the thread for that read alone.
//!
//! A pass a panic left open leaves the state with its thread: later reads and passes of that context on the thread
//! find it there, and the begin hook of any pass on the thread moves it back to its context once no pass of that
//! context runs. Another thread can't reach the state while a pass holds it: the runtime touches these services only
//! from the thread that runs the pass.

use crate::runtime::egui::memory::{FastMap, Service};
use crate::runtime::frame::{PassFacts, PerViewport, Stamped, Stored};
use egui::{Context, IdSet};
use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::sync::Arc;

/// A service the pipeline touches on every run: a field of [`PassState`].
pub(crate) trait PassLocal: Sized + 'static {
    fn cell(state: &PassState) -> &RefCell<Self>;
}

macro_rules! pass_state {
    (
        services { $($(#[$service_attr:meta])* $service:ident: $service_ty:ty,)* }
        records { $($record:ident: $record_ty:ty,)* }
        stamped { $($stamped:ident: $stamped_ty:ty,)* }
    ) => {
        /// The pass-local services of one context, one cell each, so two of them may be borrowed at once.
        #[derive(Default)]
        pub(crate) struct PassState {
            $($(#[$service_attr])* $service: RefCell<$service_ty>,)*
            $($record: RefCell<PerViewport<$record_ty>>,)*
            $($stamped: RefCell<PerViewport<Stamped<$stamped_ty>>>,)*
            /// The per-viewport records of the types with no field here (generic ones, and those of crates above the
            /// runtime), by type: each value is a `RefCell<PerViewport<_>>`.
            others: RefCell<FastMap<TypeId, Box<dyn Any + Send>>>,
        }

        $($(#[$service_attr])* impl PassLocal for $service_ty {
            fn cell(state: &PassState) -> &RefCell<Self> {
                &state.$service
            }
        })*

        $(
            impl PassLocal for PerViewport<$record_ty> {
                fn cell(state: &PassState) -> &RefCell<Self> {
                    &state.$record
                }
            }

            impl Stored for $record_ty {
                fn records<R>(ctx: &Context, f: impl FnOnce(&mut PerViewport<Self>) -> R) -> R {
                    with(ctx, f)
                }
            }
        )*

        $(
            impl PassLocal for PerViewport<Stamped<$stamped_ty>> {
                fn cell(state: &PassState) -> &RefCell<Self> {
                    &state.$stamped
                }
            }

            impl Stored for $stamped_ty {
                fn stamped<R>(ctx: &Context, f: impl FnOnce(&mut PerViewport<Stamped<Self>>) -> R) -> R {
                    with(ctx, f)
                }
            }
        )*
    };
}

pass_state! {
    services {
        frame: crate::runtime::frame::FrameState,
        measuring: crate::runtime::measure::Measuring,
        registry: crate::runtime::memory::Registry,
        closes: crate::runtime::overlay::Closes,
        scopes: crate::runtime::access::scope::Stack,
        root_tokens: crate::runtime::env::tokens::RootTokenCache,
        styles: crate::style::cache::StyleCache,
        style_groups: crate::style::cx::GroupCache,
        #[cfg(feature = "testing")]
        elisions: crate::runtime::egui::painted::Elisions,
        tooltip_grace: crate::contract::tooltip::Grace,
        gestures: crate::runtime::edit::Gestures,
        reveals: crate::contract::steps::scroll_owner::Reveals,
        scroll_moves: crate::contract::steps::scroll_owner::ScrollMoves,
        editor_records: crate::contract::steps::report::EditorRecords,
        jumps: crate::contract::body::scroll::Jumps,
        shrink_reports: crate::layout::body::ShrinkReports,
        least_reports: crate::layout::body::LeastReports,
        fill_leasts: crate::layout::body::FillLeasts,
        row_shortfalls: crate::layout::body::RowShortfalls,
        surface_frames: crate::primitives::SurfaceFrames,
        fill_counts: crate::layout::body::FillCounts,
        view_fills: crate::layout::body::ViewFills,
        held_widths: crate::layout::body::HeldWidths,
        first_lines: crate::layout::body::FirstLines,
        row_lines: crate::layout::body::RowLines,
        wrap_rows: crate::layout::body::WrapRows,
        flow_starts: crate::layout::body::FlowStarts,
        brackets: crate::layout::body::Brackets,
        cursors: crate::layout::body::Cursors,
        footer_slots: crate::contract::footer_slot::Slots,
        column_groups: crate::layout::column_group::Open,
        popups: crate::contract::overlay::popups::Popups,
        access_reported: crate::runtime::egui::access::Reported,
        windows: crate::runtime::windows::Shared,
        edit_cancels: crate::runtime::edit::CancelAll,
        drag_board: crate::runtime::drag::Board,
        commands: crate::runtime::commands::board::Board,
        inspect: crate::runtime::inspect::Log,
        recorder: crate::runtime::replay::Recorder,
        pen: crate::runtime::pen::InstalledPen,
        clipboard: crate::runtime::egui::clipboard::Installed,
        #[cfg(feature = "testing")]
        noted: crate::runtime::egui::widgets::noted::Noted,
        #[cfg(feature = "testing")]
        style_record: crate::testing::style::StyleRecord,
        #[cfg(feature = "testing")]
        type_runs: crate::runtime::measure::TypeRuns,
        #[cfg(feature = "testing")]
        memo_log: crate::runtime::measure::MemoLog,
        #[cfg(feature = "testing")]
        pass_totals: crate::runtime::measure::PassTotals,
        #[cfg(feature = "testing")]
        derived_builds: crate::runtime::measure::DerivedBuilds,
    }
    records {
        focus: crate::runtime::focus::State,
        wheel: crate::runtime::wheel::State,
        cursor: crate::runtime::cursor::State,
        layout: crate::runtime::layout_memory::Entries,
        lazy: crate::runtime::lazy::Store,
        derived: crate::runtime::derived::Store,
        arenas: crate::primitives::sensing::Arenas,
        envs: crate::runtime::env::ViewportEnvs,
        pointer: crate::runtime::pointer::Memory,
        shapes: crate::runtime::pointer::Shapes,
        overlay_persist: crate::runtime::overlay::Persist,
        announced: crate::runtime::access::live::Announced,
        motion: crate::runtime::motion::Wanted,
        drop_targets: crate::runtime::drag::Targets,
        keys: crate::runtime::keys::Stage,
        window_records: crate::runtime::windows::Record,
        raw_input: crate::runtime::egui::input::RawState,
    }
    stamped {
        reads: crate::runtime::reads::Reads,
        read_runs: crate::runtime::reads::Runs,
        ids: crate::contract::steps::id::IdPass,
        pass_pointer: Arc<crate::runtime::pointer::PassPointer>,
        overlay_pass: crate::runtime::overlay::Pass,
        shown_ids: IdSet,
        edge_step_taken: crate::contract::reorder::EdgeStepTaken,
    }
}

/// A context's pass state while a thread holds it.
struct Held {
    ctx: Context,
    state: Box<PassState>,
    /// The context's passes between their begin and end hooks on this thread, innermost last: nested viewport passes
    /// stack. A pass a panic unwound through stays until a pass of its viewport begins again.
    passes: RefCell<Vec<PassFacts>>,
}

impl Held {
    fn running(&self) -> bool {
        !self.passes.borrow().is_empty()
    }
}

struct HeldStates {
    entries: Vec<Held>,
    current: Cell<usize>,
}

impl HeldStates {
    fn take(&mut self, ctx: &Context) -> Option<Held> {
        self.get(ctx)?;
        Some(self.entries.swap_remove(self.current.get()))
    }

    /// Service reads in the same context reuse its slot. A nested context or a swap on release
    /// can change the slot, so verify its owner before reusing it.
    fn get(&self, ctx: &Context) -> Option<&Held> {
        if let Some(entry) = self.entries.get(self.current.get()).filter(|entry| entry.ctx == *ctx) {
            return Some(entry);
        }
        let at = self.entries.iter().position(|entry| entry.ctx == *ctx)?;
        self.current.set(at);
        Some(&self.entries[at])
    }
}

thread_local! {
    /// The pass states this thread holds: one per context with a pass running here, and one a panic left behind.
    static HELD: RefCell<HeldStates> = const { RefCell::new(HeldStates { entries: Vec::new(), current: Cell::new(0) }) };
}

scoped_tls::scoped_thread_local!(static RUNNING: Held);

/// Descendants use the binding of their outer component. A nested, different context
/// continues to find its own state among the thread's other held contexts.
pub(crate) fn in_component_tree() -> bool {
    RUNNING.is_set()
}

fn bound(ctx: &Context) -> bool {
    RUNNING.is_set() && RUNNING.with(|entry| entry.ctx == *ctx)
}

pub(crate) fn with_component_tree(ctx: &Context, run: impl FnOnce()) {
    let entry = HELD.with(|held| held.try_borrow_mut().ok()?.take(ctx));
    let Some(entry) = entry else { return run() };

    // The binding unwinds before this guard, so a panic also returns the state to its owner.
    struct Restore(Option<Held>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(entry) = self.0.take() {
                if entry.running() {
                    keep(entry);
                } else {
                    park(entry);
                }
            }
        }
    }
    let restore = Restore(Some(entry));
    if let Some(entry) = &restore.0 {
        RUNNING.set(entry, run);
    }
}

fn keep(entry: Held) {
    HELD.with(|held| {
        let mut held = held.borrow_mut();
        held.current.set(held.entries.len());
        held.entries.push(entry);
    });
}

/// The state of a context between passes; `None` while a thread holds it.
struct Parked(Option<Box<PassState>>);

impl Default for Parked {
    fn default() -> Self {
        Parked(Some(Box::default()))
    }
}

/// Runs `f` on the context's pass-local service of type `S`.
pub(crate) fn with<S: PassLocal, R>(ctx: &Context, f: impl FnOnce(&mut S) -> R) -> R {
    #[cfg(feature = "testing")]
    crate::runtime::layout_memory::guard::touch::<S>();
    with_state(ctx, |state| f(&mut S::cell(state).borrow_mut()))
}

/// Runs `f` on the context's records of `T`, one per viewport, for a type with no field in [`PassState`]. `f` makes
/// no call that reaches the context, so no other record of this kind is first used inside it.
pub(crate) fn with_records<T: Stored, R>(ctx: &Context, f: impl FnOnce(&mut PerViewport<T>) -> R) -> R {
    with_state(ctx, |state| {
        let key = TypeId::of::<PerViewport<T>>();
        if !state.others.borrow().contains_key(&key) {
            state.others.borrow_mut().insert(key, Box::new(RefCell::new(PerViewport::<T>::default())));
        }
        let others = state.others.borrow();
        #[expect(clippy::expect_used, reason = "the entry under a type's id is created above with that type")]
        let records = others.get(&key).and_then(|records| records.downcast_ref::<RefCell<PerViewport<T>>>()).expect("the records of the type");
        f(&mut records.borrow_mut())
    })
}

/// The facts of the innermost running pass of `ctx` on this thread, `None` outside every pass of it.
pub(crate) fn running(ctx: &Context) -> Option<PassFacts> {
    if RUNNING.is_set()
        && let Some(facts) = RUNNING.with(|entry| (entry.ctx == *ctx).then(|| entry.passes.borrow().last().copied()))
    {
        return facts;
    }
    note_unbound();
    HELD.with(|held| held.borrow().get(ctx).and_then(|entry| entry.passes.borrow().last().copied()))
}

/// Puts the pass the begin hook starts on the running stack of `ctx`, whose state this thread holds: a pass of the
/// same viewport still there never ended (a panic unwound through it).
pub(crate) fn begin_running(ctx: &Context, facts: PassFacts) {
    let held = with_entry(ctx, |entry| {
        let mut passes = entry.passes.borrow_mut();
        passes.retain(|pass| pass.viewport != facts.viewport);
        passes.push(facts);
    });
    debug_assert!(held.is_some(), "the begin hook holds the context's pass state");
}

/// Takes the innermost pass of `ctx` off its running stack: the end hook.
pub(crate) fn end_running(ctx: &Context) {
    with_entry(ctx, |entry| entry.passes.borrow_mut().pop());
}

/// Runs `f` on the entry of `ctx` this thread holds, bound or not.
fn with_entry<R>(ctx: &Context, f: impl FnOnce(&Held) -> R) -> Option<R> {
    if bound(ctx) {
        return Some(RUNNING.with(f));
    }
    HELD.with(|held| held.borrow().get(ctx).map(f))
}

/// Runs `f` on the context's pass state: inside its component tree, straight from the binding.
fn with_state<R>(ctx: &Context, f: impl FnOnce(&PassState) -> R) -> R {
    let mut call = Some(f);
    if RUNNING.is_set()
        && let Some(out) = RUNNING.with(|entry| if entry.ctx == *ctx { call.take().map(|f| f(&entry.state)) } else { None })
    {
        return out;
    }
    note_unbound();
    let mut out = None;
    visit(ctx, &mut |state| {
        if let Some(f) = call.take() {
            out = Some(f(state));
        }
    });
    #[expect(clippy::expect_used, reason = "visit calls its callback once on both the held and parked paths")]
    out.expect("the pass state was visited")
}

#[cfg(feature = "testing")]
thread_local! {
    static UNBOUND: Cell<u64> = const { Cell::new(0) };
}

/// How many reads of a pass state this thread made outside the component tree that binds it.
#[cfg(feature = "testing")]
#[doc(hidden)]
pub fn unbound_reads() -> u64 {
    UNBOUND.with(Cell::get)
}

#[inline]
fn note_unbound() {
    #[cfg(feature = "testing")]
    UNBOUND.with(|reads| reads.set(reads.get() + 1));
}

/// The state of a context its component tree does not bind: the thread's held entry, else the parked state, held for
/// this read alone. Shared by every service and caller.
fn visit(ctx: &Context, f: &mut dyn FnMut(&PassState)) {
    let found = HELD.with(|held| {
        let held = held.borrow();
        let Some(entry) = held.get(ctx) else { return false };
        f(&entry.state);
        true
    });
    if !found {
        // Outside every pass of the context: hold the state for this read alone.
        struct Release<'a>(&'a Context);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                release(self.0);
            }
        }
        hold(ctx);
        let _release = Release(ctx);
        HELD.with(|held| {
            let held = held.borrow();
            #[expect(clippy::expect_used, reason = "`hold` has just moved the context's state into this thread")]
            let entry = held.get(ctx).expect("the state was just taken");
            f(&entry.state)
        })
    }
}

/// Moves the context's state into this thread unless it holds it already: the begin hook. States of other contexts
/// with no pass running here (a pass a panic left open) go back to their contexts first.
pub(crate) fn hold(ctx: &Context) {
    if bound(ctx) {
        return;
    }
    let stale: Vec<Held> = HELD.with(|held| held.borrow_mut().entries.extract_if(.., |entry| entry.ctx != *ctx && !entry.running()).collect());
    for held in stale {
        park(held);
    }
    if HELD.with(|held| held.borrow().get(ctx).is_some()) {
        return;
    }
    let state = Service::<Parked>::run(ctx, |parked| parked.0.take());
    debug_assert!(state.is_some(), "another thread holds this context's pass state");
    let state = state.unwrap_or_default();
    keep(Held { ctx: ctx.clone(), state, passes: RefCell::new(Vec::new()) });
}

/// Moves the context's state back into its context data unless a pass of it still runs on this thread: the end hook.
pub(crate) fn release(ctx: &Context) {
    if bound(ctx) {
        return;
    }
    let idle = HELD.with(|held| {
        let mut held = held.borrow_mut();
        if held.get(ctx).is_some_and(Held::running) { None } else { held.take(ctx) }
    });
    if let Some(held) = idle {
        park(held);
    }
}

fn park(held: Held) {
    Service::<Parked>::run(&held.ctx, |parked| parked.0 = Some(held.state));
}

#[cfg(all(feature = "testing", any(test, feature = "suite")))]
pub(crate) mod binding_tests {
    use super::{PassState, begin_running, bound, end_running, hold, in_component_tree, release, running, with_component_tree, with_state};
    use crate::runtime::frame::PassFacts;
    use crate::testing::trial;
    use egui::{Context, ViewportId};

    fn address(ctx: &Context) -> *const PassState {
        with_state(ctx, std::ptr::from_ref)
    }

    #[trial]
    fn component_tree_restores_its_state_after_nested_context_access_and_unwind() {
        let outer = Context::default();
        let inner = Context::default();
        hold(&outer);
        begin_running(&outer, PassFacts { viewport: ViewportId::ROOT, pass_nr: 7, frame_nr: 7, now: 0.0, elapsed: 0.0 });
        let original = address(&outer);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_component_tree(&outer, || {
                assert!(bound(&outer));
                assert!(!bound(&inner));
                assert_eq!(running(&outer).map(|facts| facts.pass_nr), Some(7), "the tree reads its pass from the binding");
                assert!(running(&inner).is_none(), "a nested context has no pass running");
                assert_ne!(address(&inner), original);
                assert_eq!(address(&outer), original);
                std::panic::resume_unwind(Box::new("component unwound"));
            });
        }));
        assert!(unwound.is_err_and(|reason| reason.downcast_ref::<&str>() == Some(&"component unwound")));
        assert!(!in_component_tree());
        assert_eq!(address(&outer), original);
        assert_eq!(running(&outer).map(|facts| facts.pass_nr), Some(7), "the running pass outlives the unwound tree");
        end_running(&outer);
        release(&outer);
        assert!(running(&outer).is_none());
    }
}
