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

use crate::runtime::egui::memory::Service;
use crate::runtime::frame::{self, PerViewport, Stamped, Stored};
use egui::{Context, IdSet};
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
        row_shortfalls: crate::layout::body::RowShortfalls,
        bar_rules: crate::contract::BarRules,
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
    fn get(&self, ctx: &Context) -> Option<&PassState> {
        if let Some(entry) = self.entries.get(self.current.get()).filter(|entry| entry.ctx == *ctx) {
            return Some(&entry.state);
        }
        let at = self.entries.iter().position(|entry| entry.ctx == *ctx)?;
        self.current.set(at);
        Some(&self.entries[at].state)
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
                if frame::running(&entry.ctx).is_some() {
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
    let mut call = Some(f);
    let mut out = None;
    visit(ctx, &mut |state| {
        if let Some(f) = call.take() {
            out = Some(f(&mut S::cell(state).borrow_mut()));
        }
    });
    #[expect(clippy::expect_used, reason = "visit calls its callback once on both the held and parked paths")]
    out.expect("the pass state was visited")
}

/// The thread-local lookup and the parked-state slow path are shared by every service and caller.
fn visit(ctx: &Context, f: &mut dyn FnMut(&PassState)) {
    if RUNNING.is_set()
        && RUNNING.with(|entry| {
            if entry.ctx != *ctx {
                return false;
            }
            f(&entry.state);
            true
        })
    {
        return;
    }
    let found = HELD.with(|held| {
        let held = held.borrow();
        let Some(state) = held.get(ctx) else { return false };
        f(state);
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
            let state = held.get(ctx).expect("the state was just taken");
            f(state)
        })
    }
}

/// Moves the context's state into this thread unless it holds it already: the begin hook. States of other contexts
/// with no pass running here (a pass a panic left open) go back to their contexts first.
pub(crate) fn hold(ctx: &Context) {
    if bound(ctx) {
        return;
    }
    let stale: Vec<Held> =
        HELD.with(|held| held.borrow_mut().entries.extract_if(.., |entry| entry.ctx != *ctx && frame::running(&entry.ctx).is_none()).collect());
    for held in stale {
        park(held);
    }
    if HELD.with(|held| held.borrow().get(ctx).is_some()) {
        return;
    }
    let state = Service::<Parked>::run(ctx, |parked| parked.0.take());
    debug_assert!(state.is_some(), "another thread holds this context's pass state");
    let state = state.unwrap_or_default();
    keep(Held { ctx: ctx.clone(), state });
}

/// Moves the context's state back into its context data unless a pass of it still runs on this thread: the end hook.
pub(crate) fn release(ctx: &Context) {
    if bound(ctx) || frame::running(ctx).is_some() {
        return;
    }
    if let Some(held) = HELD.with(|held| held.borrow_mut().take(ctx)) {
        park(held);
    }
}

fn park(held: Held) {
    Service::<Parked>::run(&held.ctx, |parked| parked.0 = Some(held.state));
}

#[cfg(all(feature = "testing", any(test, feature = "suite")))]
pub(crate) mod binding_tests {
    use super::{PassState, bound, hold, in_component_tree, visit, with_component_tree};
    use crate::testing::trial;
    use egui::Context;

    fn address(ctx: &Context) -> *const PassState {
        let mut address = std::ptr::null();
        visit(ctx, &mut |state| address = std::ptr::from_ref(state));
        address
    }

    #[trial]
    fn component_tree_restores_its_state_after_nested_context_access_and_unwind() {
        let outer = Context::default();
        let inner = Context::default();
        hold(&outer);
        let original = address(&outer);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_component_tree(&outer, || {
                assert!(bound(&outer));
                assert!(!bound(&inner));
                assert_ne!(address(&inner), original);
                assert_eq!(address(&outer), original);
                std::panic::resume_unwind(Box::new("component unwound"));
            });
        }));
        assert!(unwound.is_err_and(|reason| reason.downcast_ref::<&str>() == Some(&"component unwound")));
        assert!(!in_component_tree());
        assert_eq!(address(&outer), original);
    }
}
