//! The style step and what the pipeline keeps of a component's inputs: app replacements, style-affecting app
//! values, faces, the shown value and memory eviction.

use craft_ui::contract::{Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Props as _, Remember as _, Role, Show as _};
use craft_ui::style::StyleCx;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::{PlainStyle, Probe, Seen};
use egui::{Context, Response, Vec2};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

static CLONES: AtomicU32 = AtomicU32::new(0);

/// A value that counts its clones.
#[derive(Debug, PartialEq)]
struct Counted(u8);

impl Clone for Counted {
    fn clone(&self) -> Self {
        CLONES.fetch_add(1, Ordering::SeqCst);
        Counted(self.0)
    }
}

/// A leaf with a value binding.
#[derive(Component)]
struct Holder<'a> {
    #[common]
    common: Common,
    value: Bind<'a, Counted>,
}

impl Component for Holder<'_> {
    const ROLE: Role = Role::None;
    type Style = PlainStyle;
    type Output = EditResponse<Counted>;

    fn style(_: &StyleCx<'_>, _: HolderStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::splat(10.0))
    }

    fn bind(&mut self) -> Option<BindMut<'_, Counted>> {
        Some(self.value.reborrow())
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<Counted> {
        cx.edit_response()
    }
}

/// Showing and measuring a component reads its binding through a held copy: the value is cloned only when it
/// changed.
#[trial]
fn showing_and_measuring_clone_the_bound_value_only_when_it_changed() {
    let mut frames = Frames::new();
    let mut value = Counted(1);
    let before = CLONES.load(Ordering::SeqCst);
    for _ in 0..3 {
        frames.frame(vec![], |ui| {
            Holder { common: Common::new(), value: Bind::bound(&mut value) }.id_salt("holder").show(ui);
        });
    }
    assert_eq!(CLONES.load(Ordering::SeqCst) - before, 1, "three shows of one value cloned it once");
    frames.frame(vec![], |ui| {
        let mut holder = Holder { common: Common::new(), value: Bind::bound(&mut value) }.id_salt("holder");
        holder.measure_in(ui, craft_behavior::Proposal::Ideal);
    });
    assert_eq!(CLONES.load(Ordering::SeqCst) - before, 1, "measuring an unchanged value clones nothing");
    value.0 = 2;
    frames.frame(vec![], |ui| {
        Holder { common: Common::new(), value: Bind::bound(&mut value) }.id_salt("holder").show(ui);
    });
    assert_eq!(CLONES.load(Ordering::SeqCst) - before, 2, "a changed value is cloned once");
}

static DROPPED: AtomicU32 = AtomicU32::new(0);
static CALLED_CONTEXT: AtomicU32 = AtomicU32::new(0);
static CTX: Mutex<Option<Context>> = Mutex::new(None);

/// A slot whose `Drop` calls a `Context` API, as a slot holding a texture handle does.
#[derive(Clone, Default)]
struct Tracker(Option<Arc<()>>);

impl Drop for Tracker {
    fn drop(&mut self) {
        let Some(token) = self.0.take() else { return };
        if Arc::strong_count(&token) > 1 {
            return;
        }
        DROPPED.fetch_add(1, Ordering::SeqCst);
        if let Ok(guard) = CTX.lock()
            && let Some(ctx) = guard.as_ref()
        {
            let _ = ctx.pixels_per_point();
            CALLED_CONTEXT.fetch_add(1, Ordering::SeqCst);
        }
    }
}

/// A leaf that keeps a slot in shown memory.
#[derive(Component)]
struct Keeper {
    #[common]
    common: Common,
}

impl Component for Keeper {
    const ROLE: Role = Role::None;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: KeeperStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::splat(10.0))
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let tracker: &mut Tracker = cx.memory();
        tracker.0.get_or_insert_with(|| Arc::new(()));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The pipeline drops evicted slots at its next run, outside every lock.
#[trial]
fn an_evicted_slot_drops_once_in_the_next_pipeline_run_outside_any_lock() {
    let mut frames = Frames::new();
    if let Ok(mut ctx) = CTX.lock() {
        *ctx = Some(frames.ctx.clone());
    }
    let seen = RefCell::new(Seen::default());
    for _ in 0..3 {
        frames.frame(vec![], |ui| {
            Keeper { common: Common::new() }.id_salt("keeper").show(ui);
            Probe::new(&seen).id_salt("other").show(ui);
        });
    }
    assert_eq!(DROPPED.load(Ordering::SeqCst), 0, "a shown slot is not evicted");
    for _ in 0..60 {
        frames.frame(vec![], |ui| {
            Probe::new(&seen).id_salt("other").show(ui);
        });
    }
    if let Ok(mut ctx) = CTX.lock() {
        *ctx = None;
    }
    assert_eq!(DROPPED.load(Ordering::SeqCst), 1, "the evicted slot's Drop ran exactly once");
    assert_eq!(CALLED_CONTEXT.load(Ordering::SeqCst), 1, "and could call a Context API");
}
