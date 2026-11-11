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
use egui::Context;
use std::cell::RefCell;
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
        elisions: crate::runtime::egui::painted::Elisions,
        tooltip_grace: crate::contract::tooltip::Grace,
        gestures: crate::runtime::edit::Gestures,
        reveals: crate::contract::steps::scroll_owner::Reveals,
        scroll_moves: crate::contract::steps::scroll_owner::ScrollMoves,
        jumps: crate::contract::body::scroll::Jumps,
        shrink_reports: crate::layout::body::ShrinkReports,
        least_reports: crate::layout::body::LeastReports,
        fill_counts: crate::layout::body::FillCounts,
        cursors: crate::layout::body::Cursors,
        column_groups: crate::layout::column_group::Open,
        popups: crate::contract::overlay::popups::Popups,
        access_reported: crate::runtime::egui::access::Reported,
        windows: crate::runtime::windows::Shared,
        edit_cancels: crate::runtime::edit::CancelAll,
        drag_board: crate::runtime::drag::Board,
        commands: crate::runtime::commands::board::Board,
        inspect: crate::runtime::inspect::Log,
        recorder: crate::runtime::replay::Recorder,
        #[cfg(feature = "testing")]
        noted: crate::runtime::egui::widgets::noted::Noted,
        #[cfg(feature = "testing")]
        style_record: crate::testing::style::StyleRecord,
        #[cfg(feature = "testing")]
        type_runs: crate::runtime::measure::TypeRuns,
    }
    records {
        focus: crate::runtime::focus::State,
        wheel: crate::runtime::wheel::State,
        cursor: crate::runtime::cursor::State,
        layout: crate::runtime::layout_memory::Entries,
        lazy: crate::runtime::lazy::Store,
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
    }
    stamped {
        reads: crate::runtime::reads::Reads,
        read_runs: crate::runtime::reads::Runs,
        ids: crate::contract::steps::id::IdPass,
        pass_pointer: Arc<crate::runtime::pointer::PassPointer>,
        overlay_pass: crate::runtime::overlay::Pass,
        shown_ids: std::collections::HashSet<egui::Id>,
    }
}

/// A context's pass state while a thread holds it.
struct Held {
    ctx: Context,
    state: Box<PassState>,
}

thread_local! {
    /// The pass states this thread holds: one per context with a pass running here, and one a panic left behind.
    static HELD: RefCell<Vec<Held>> = const { RefCell::new(Vec::new()) };
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
    let held = HELD.with(|held| {
        let held = held.borrow();
        match held.iter().find(|held| held.ctx == *ctx) {
            Some(held) => Ok(f(&mut S::cell(&held.state).borrow_mut())),
            None => Err(f),
        }
    });
    match held {
        Ok(out) => out,
        Err(f) => {
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
                let held = held.iter().find(|held| held.ctx == *ctx).expect("the state was just taken");
                f(&mut S::cell(&held.state).borrow_mut())
            })
        }
    }
}

/// Moves the context's state into this thread unless it holds it already: the begin hook. States of other contexts
/// with no pass running here (a pass a panic left open) go back to their contexts first.
pub(crate) fn hold(ctx: &Context) {
    let stale: Vec<Held> = HELD.with(|held| {
        let mut held = held.borrow_mut();
        let (stale, kept) = std::mem::take(&mut *held).into_iter().partition(|held| held.ctx != *ctx && frame::running(&held.ctx).is_none());
        *held = kept;
        stale
    });
    for held in stale {
        park(held);
    }
    if HELD.with(|held| held.borrow().iter().any(|held| held.ctx == *ctx)) {
        return;
    }
    let state = Service::<Parked>::run(ctx, |parked| parked.0.take());
    debug_assert!(state.is_some(), "another thread holds this context's pass state");
    let state = state.unwrap_or_default();
    HELD.with(|held| held.borrow_mut().push(Held { ctx: ctx.clone(), state }));
}

/// Moves the context's state back into its context data unless a pass of it still runs on this thread: the end hook.
pub(crate) fn release(ctx: &Context) {
    if frame::running(ctx).is_some() {
        return;
    }
    let held = HELD.with(|held| {
        let mut held = held.borrow_mut();
        held.iter().position(|held| held.ctx == *ctx).map(|at| held.swap_remove(at))
    });
    if let Some(held) = held {
        park(held);
    }
}

fn park(held: Held) {
    Service::<Parked>::run(&held.ctx, |parked| parked.0 = Some(held.state));
}
