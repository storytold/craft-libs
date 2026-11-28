//! Memory lifetimes, eviction by owner and the runtime sub-id derivation.

use craft_host::{AppInfo, Host, MemorySections, SectionStore};
use craft_ui::contract::{
    Bind, BindMut, Common, Component, Container, ContainerProps, Cx, Mount, NoValue, Outcome, Overlay, Props as _, Role, ShowContainer as _,
};
use craft_ui::runtime::memory::{Remember, SaveKey, saved_value, set_saved_value};
use craft_ui::style::StyleCx;
use craft_ui::testing::memory::{Stage, dispose_evicted, forget, forget_saved, global, records_written};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Id, Response, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const PLACE: SaveKey = SaveKey::new("craft_tests.place");
const KEPT: SaveKey = SaveKey::new("craft_tests.kept");

/// A host's settings store that counts its writes.
#[derive(Default)]
struct Counted {
    sections: MemorySections,
    writes: AtomicUsize,
}

impl Counted {
    fn writes(&self) -> usize {
        self.writes.load(Ordering::Relaxed)
    }
}

impl SectionStore for Counted {
    fn read(&self, key: &str) -> Option<toml::Table> {
        self.sections.read(key)
    }

    fn write(&self, key: &str, table: toml::Table) {
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.sections.write(key, table);
    }
}

fn counted_host() -> (Arc<Counted>, Arc<Host>) {
    let store = Arc::new(Counted::default());
    let host = Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).settings(store.clone()).build());
    (store, host)
}

/// The saved `u32` of `KEPT` a stage of `id` reads in one frame of `frames`.
fn read_kept(frames: &mut Frames, id: Id) -> u32 {
    let mut read = 0;
    frames.frame(vec![], |ui| {
        let mut stage = Stage::open(ui.ctx(), id);
        read = *stage.saved::<u32>(&KEPT);
        stage.finish();
    });
    read
}

/// The SAVED tier persists through the frame's host: a value written in one context is read by a new context over
/// the same host, an unchanged value (a rerun pass, later frames) is never stored again, and `forget_saved` forgets
/// the stored value too.
#[trial]
fn a_saved_slot_survives_a_new_context_with_the_same_host_and_is_stored_once_per_change() {
    let (store, host) = counted_host();
    let id = global("saver", ());
    let mut frames = Frames::new();
    frames.host(host.clone());
    let mut first = true;
    let passes = frames.frame(vec![], |ui| {
        if std::mem::take(&mut first) {
            ui.ctx().request_discard("the writing frame runs twice");
        }
        let mut stage = Stage::open(ui.ctx(), id);
        *stage.saved::<u32>(&KEPT) = 6;
        stage.finish();
    });
    assert_eq!(passes, 2, "the writing frame reran its pass");
    let read = [read_kept(&mut frames, id), read_kept(&mut frames, id)];
    assert_eq!(read, [6, 6]);
    assert_eq!(store.writes(), 1, "the value is stored once: a rerun pass and later frames that leave it unchanged store nothing");

    let mut again = Frames::new();
    again.host(host.clone());
    assert_eq!(read_kept(&mut again, id), 6, "a new context over the same host reads the stored value");
    forget_saved(&again.ctx, &KEPT);

    let mut third = Frames::new();
    third.host(host);
    assert_eq!(read_kept(&mut third, id), 0, "forget_saved forgot the stored value");
}

/// L-12: a value written with `set_saved_value` is read in the next pass by `saved_value` and by a stage's
/// `Remember::saved`, through the one slot, and is stored once per change.
#[trial]
fn set_saved_value_is_read_back_by_saved_value_and_stored_once() {
    let (store, host) = counted_host();
    let id = global("saved_value_reader", ());
    let mut frames = Frames::new();
    frames.host(host);
    frames.frame(vec![], |ui| set_saved_value(ui.ctx(), &KEPT, 7u32));
    let mut read = Vec::new();
    frames.frame(vec![], |ui| {
        read.push(saved_value::<u32>(ui.ctx(), &KEPT));
        set_saved_value(ui.ctx(), &KEPT, 7u32);
    });
    read.push(read_kept(&mut frames, id));
    assert_eq!(read, vec![7, 7], "saved_value and a stage read the value set_saved_value wrote");
    assert_eq!(store.writes(), 1, "writing the same value again stores nothing");
    frames.frame(vec![], |ui| set_saved_value(ui.ctx(), &KEPT, 8u32));
    assert_eq!(store.writes(), 2, "a changed value is stored once");
}

/// A component left unshown for a frame and shown again well inside the eviction grace (a backstage page left and
/// reopened) appears again through every pass of the frame it comes back in, and only then; its shown entries stay.
#[trial]
fn a_component_shown_again_after_a_frame_without_it_appears_again() {
    let mut frames = Frames::new();
    let id = global("page", ());
    let passes = RefCell::new(Vec::<bool>::new());
    let mut show = |ui: &mut Ui| {
        let mut stage = Stage::open(ui.ctx(), id);
        passes.borrow_mut().push(stage.appeared());
        *stage.memory::<u32>() += 1;
        stage.finish();
    };
    frames.frame(vec![], &mut show);
    frames.frame(vec![], &mut show);
    assert_eq!(passes.borrow().last(), Some(&false), "a component shown every frame appears once");
    frames.frame(vec![], |_| {});
    passes.borrow_mut().clear();
    frames.frame(vec![], &mut show);
    let back = passes.take();
    assert!(!back.is_empty() && back.iter().all(|&appeared| appeared), "every pass of the frame it is shown again in: {back:?}");
    frames.frame(vec![], |ui| {
        let mut stage = Stage::open(ui.ctx(), id);
        passes.borrow_mut().push(stage.appeared());
        assert!(*stage.memory::<u32>() >= 3, "the shown entry stayed inside the grace");
        stage.finish();
    });
    assert_eq!(passes.take(), vec![false], "the next frame is no appearance");
}

/// The lifetimes of 7.2, through `Remember`: shown entries go with their owner; session and saved ones only
/// through `forget` and `forget_saved`; a press entry goes when its press ends.
#[trial]
fn lifetimes_follow_their_owner_until_forgotten() {
    let mut frames = Frames::new();
    let id = global("owner", ());
    let mut appeared = Vec::new();
    for _ in 0..2 {
        frames.frame(vec![], |ui| {
            let mut stage = Stage::open(ui.ctx(), id);
            appeared.push(stage.appeared());
            *stage.memory::<u32>() += 1;
            *stage.memory_at::<u32>("part") += 10;
            *stage.keep::<u32>() = 5;
            *stage.saved::<u32>(&PLACE) = 6;
            *stage.press_slot::<u32>() = Some(8);
            stage.finish();
        });
    }
    assert_eq!(appeared, vec![true, false], "appeared is true on the first pass only");
    frames.frame(vec![], |ui| {
        let mut stage = Stage::open(ui.ctx(), id);
        assert_eq!(*stage.memory::<u32>(), 2);
        assert_eq!(*stage.memory_at::<u32>("part"), 20, "memory_at keeps its own slot");
        assert_eq!(*stage.press_slot::<u32>(), Some(8), "a press entry lives while its press does");
        *stage.press_slot::<u32>() = None;
        stage.finish();
    });
    frames.frame(vec![], |ui| {
        let mut stage = Stage::open(ui.ctx(), id);
        assert_eq!(*stage.press_slot::<u32>(), None, "an ended press leaves no entry");
        stage.finish();
    });
    frames.idle(60, |_| {});
    dispose_evicted(&frames.ctx);
    frames.frame(vec![], |ui| {
        let mut stage = Stage::open(ui.ctx(), id);
        assert!(stage.appeared(), "a re-shown component appears again");
        assert_eq!(*stage.memory::<u32>(), 0, "the shown entry went with its owner");
        assert_eq!(*stage.memory_at::<u32>("part"), 0, "and the memory_at entry");
        assert_eq!(*stage.keep::<u32>(), 5, "the session entry stayed");
        assert_eq!(*stage.saved::<u32>(&PLACE), 6, "the saved entry stayed");
        stage.finish();
    });
    forget(&frames.ctx, id);
    frames.frame(vec![], |ui| {
        let mut stage = Stage::open(ui.ctx(), id);
        assert_eq!(*stage.keep::<u32>(), 0, "forget removes the session entry");
        assert_eq!(*stage.saved::<u32>(&PLACE), 6, "and not the saved one");
        stage.finish();
    });
    forget_saved(&frames.ctx, &PLACE);
    frames.frame(vec![], |ui| {
        let mut stage = Stage::open(ui.ctx(), id);
        assert_eq!(*stage.saved::<u32>(&PLACE), 0, "forget_saved removes the saved entry written through Remember::saved");
        stage.finish();
    });
}

/// A slot that a stage owned by another component writes back goes with that owner: a popup body's or a
/// settled body's slots stay while their container is shown, even after the component that made them stops.
#[trial]
fn a_slot_written_by_an_owning_stage_goes_with_that_owner() {
    let mut frames = Frames::new();
    let (base, container) = (global("made_by", ()), global("kept_by", ()));
    frames.frame(vec![], |ui| {
        let mut stage = Stage::open(ui.ctx(), base);
        *stage.memory::<u32>() += 1;
        stage.finish();
    });
    let mut last = 0;
    frames.idle(60, |ui| {
        let mut stage = Stage::open_owned(ui.ctx(), base, container);
        *stage.memory::<u32>() += 10;
        last = *stage.memory::<u32>();
        stage.finish();
    });
    dispose_evicted(&frames.ctx);
    assert_eq!(last % 10, 1, "the slot its maker wrote stayed with the container that keeps showing it");
}

/// Write-back records a slot once: a stage that keeps writing the same slots records nothing after its first
/// frame, and a stage owned by another component than its base records what it writes every frame.
#[trial]
fn a_settled_stage_records_no_slot_again() {
    let mut frames = Frames::new();
    let (base, container) = (global("settled", ()), global("settled_owner", ()));
    let show = |ui: &mut egui::Ui| {
        let mut stage = Stage::open(ui.ctx(), base);
        *stage.memory::<u32>() += 1;
        stage.finish();
    };
    frames.frame(vec![], show);
    let first = records_written(&frames.ctx);
    assert!(first > 0, "the first frame records the slot and its seen mark");
    frames.idle(3, show);
    assert_eq!(records_written(&frames.ctx), first, "settled frames record nothing");
    frames.idle(3, |ui| {
        let mut stage = Stage::open_owned(ui.ctx(), base, container);
        *stage.memory::<u32>() += 1;
        stage.finish();
    });
    assert!(records_written(&frames.ctx) >= first + 3, "an owning stage records what it writes in every frame");
}

/// A layer with a bound open state whose output has an unshown form: closed, it runs no pipeline.
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct Sheet<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    open: Bind<'a, bool>,
}

struct SheetOut(Response);

impl Outcome for SheetOut {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.0
    }

    fn unshown(response: Response) -> Option<Self> {
        Some(SheetOut(response))
    }
}

impl Component for Sheet<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = SheetOut;

    fn style(_: &StyleCx<'_>, _: SheetStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(120.0, 60.0))
    }

    fn finish(self, cx: Cx<'_, Self>) -> SheetOut {
        SheetOut(cx.response.clone())
    }
}

impl Container for Sheet<'_> {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        Some(self.open.reborrow())
    }
}

/// A container that shows nothing ends its shown life at once: closed, its body runs no more, and its next open is a
/// first pass with nothing kept, well inside the eviction grace. While it shows, its body keeps its entries.
#[trial]
fn a_closed_container_ends_its_shown_life_at_once_and_a_shown_one_keeps_it() {
    let open = Cell::new(true);
    // Each body run: whether it is the body's first pass, and how many passes its kept count has seen.
    let runs = RefCell::new(Vec::<(bool, u32)>::new());
    let mut show = |ui: &mut Ui| {
        let mut is_open = open.get();
        let props = ContainerProps::new().mount(Mount::Layer(Overlay::floating()));
        Sheet { common: Common::new(), props, open: Bind::bound(&mut is_open) }.id_salt("sheet").show(ui, |_, cx| {
            let kept = cx.memory::<u32>();
            *kept += 1;
            let kept = *kept;
            runs.borrow_mut().push((cx.appeared(), kept));
        });
    };
    let mut frames = Frames::new();
    frames.idle(4, &mut show);
    let shown = runs.borrow().clone();
    assert!(shown.first().is_some_and(|run| run.0), "the first open is a first pass");
    assert!(shown.last().is_some_and(|run| !run.0 && run.1 >= 3), "a shown body keeps its entries: {shown:?}");

    open.set(false);
    frames.idle(2, &mut show);
    // Past the exit fade, and less than the 0.5 s an evicted owner waits.
    frames.advance(0.25);
    frames.idle(2, &mut show);
    let closed = runs.borrow().len();
    frames.idle(3, &mut show);
    assert_eq!(runs.borrow().len(), closed, "a closed container runs no body");

    open.set(true);
    frames.idle(1, &mut show);
    let reopened = runs.borrow()[closed..].to_vec();
    assert!(reopened.first().is_some_and(|run| run.0 && run.1 == 1), "the reopen is a first pass with nothing kept: {reopened:?}");
}
