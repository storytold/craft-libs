//! A modal layer's cover and a single-frame tap (pointer arrives, presses and releases in one pass) inside it:
//! egui decides the press against the last pass's widget record, so a press whose widgets are not in that record
//! yet takes the cover's absorber or nothing, and its release, which the one-press rule holds for the next frame,
//! lands after the modal gate closed: the press arrives nowhere. Photocraft measured (pin e7e877ac5, 2026-11-20):
//! a kittest click is absorbed in the first 1-2 passes after open and lands at 3-6, while the framework's own
//! components take the press; after a reopen the window shifts.

use craft_ui::contract::{Common, Component, Container, ContainerProps, Cx, Mount, Overlay, Props as _, Role, Show as _, ShowContainer as _};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::{PlainStyle, Probe, Seen};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

/// A modal layer whose body the caller fills.
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct Modal {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Component for Modal {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = egui::Response;

    fn style(_: &craft_ui::style::StyleCx<'_>, _: ModalStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(240.0, 160.0))
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

impl Container for Modal {}

/// A scene: a host button behind the modal, and a modal (while open) holding a contract Probe from its first body
/// run and a host-drawn egui button from `show_after` body runs on: the app content a frame behind its cover, as a
/// host's own drawn content is. Each keeps its rect and counts its clicks.
pub(crate) struct Scene {
    open: Cell<bool>,
    show_after: Cell<usize>,
    runs: Cell<usize>,
    put_front: Cell<bool>,
    behind: RefCell<(Rect, u32)>,
    front: RefCell<(Rect, u32)>,
    host: RefCell<(Rect, u32)>,
    probe: RefCell<Seen>,
}

impl Scene {
    pub(crate) fn new() -> Self {
        Scene {
            open: Cell::new(false),
            show_after: Cell::new(0),
            runs: Cell::new(0),
            put_front: Cell::new(false),
            behind: RefCell::new((Rect::NOTHING, 0)),
            front: RefCell::new((Rect::NOTHING, 0)),
            host: RefCell::new((Rect::NOTHING, 0)),
            probe: RefCell::default(),
        }
    }

    pub(crate) fn open(&self) -> &Cell<bool> {
        &self.open
    }

    /// Draw a `Front` button at the place the modal's content will hold; its click opens the modal.
    pub(crate) fn put_front(&self, on: bool) {
        self.put_front.set(on);
    }

    /// Show the host-drawn button only from body run `n` on: its cover shows a pass before it.
    pub(crate) fn show_after(&self, n: usize) {
        self.show_after.set(n);
    }

    pub(crate) fn host_rect(&self) -> Rect {
        self.host.borrow().0
    }

    pub(crate) fn host_clicked(&self) -> u32 {
        self.host.borrow().1
    }

    pub(crate) fn behind_clicked(&self) -> u32 {
        self.behind.borrow().1
    }

    pub(crate) fn front_rect(&self) -> Rect {
        self.front.borrow().0
    }

    pub(crate) fn front_clicked(&self) -> u32 {
        self.front.borrow().1
    }

    pub(crate) fn probe_activations(&self) -> u32 {
        self.probe.borrow().activations
    }

    pub(crate) fn show(&mut self, ui: &mut Ui) {
        let back = ui.button("Behind");
        self.behind.borrow_mut().0 = back.rect;
        if back.clicked() {
            self.behind.borrow_mut().1 += 1;
        }
        if self.put_front.get() {
            let front = ui.put(Rect::from_min_size(Pos2::new(380.0, 290.0), Vec2::new(60.0, 24.0)), egui::Button::new("Front"));
            self.front.borrow_mut().0 = front.rect;
            if front.clicked() {
                self.front.borrow_mut().1 += 1;
                self.open.set(true);
            }
        }
        if !self.open.get() {
            self.runs.set(0);
            return;
        }
        self.runs.set(self.runs.get() + 1);
        let props = ContainerProps::new().mount(Mount::Layer(Overlay::modal()));
        Modal { common: Common::new(), props }.id_salt("modal").show(ui, |ui, _| {
            if self.runs.get() >= self.show_after.get().max(1) {
                let host = ui.button("Host button");
                self.host.borrow_mut().0 = host.rect;
                if host.clicked() {
                    self.host.borrow_mut().1 += 1;
                }
            }
            let seen = &self.probe;
            Probe::new(seen).size(Vec2::new(80.0, 24.0)).id_salt("leaf").show(ui);
        });
    }
}

/// The place the host button sits: from a run that shows it from the first body run.
fn host_at() -> Pos2 {
    let mut warm = Frames::new();
    let mut scene = Scene::new();
    scene.open().set(true);
    warm.frame(vec![], |ui| scene.show(ui));
    warm.frame(vec![], |ui| scene.show(ui));
    scene.host_rect().center()
}

/// One frame after the modal shows, its cover is in egui's record and its host content is not yet: a single-frame
/// tap inside the modal must still reach the host's button, as it does in any later pass. Photocraft: the cover
/// absorbs it today.
#[trial]
fn a_single_frame_tap_on_content_shown_after_its_cover_reaches_that_content() {
    let at = host_at();
    let mut frames = Frames::new();
    let mut scene = Scene::new();
    scene.show_after(2);
    scene.open().set(true);
    frames.frame(vec![], |ui| scene.show(ui)); // the cover shows; the host content does not
    frames.frame(vec![moved(at), press(at), release(at)], |ui| scene.show(ui)); // the tap: content shows this pass
    for _ in 0..4 {
        frames.frame(vec![], |ui| scene.show(ui));
    }
    assert_eq!(scene.host_clicked(), 1, "the tap lands on the content that showed one pass after its cover");
    assert_eq!(scene.behind_clicked(), 0, "and on nothing behind the cover");
}

/// The framework's own component, shown with the cover, takes the same tap: photocraft measured it; the fix must
/// keep it.
#[trial]
fn the_same_tap_reaches_the_framework_component_shown_with_the_cover() {
    let at = host_at();
    let mut frames = Frames::new();
    let mut scene = Scene::new();
    scene.show_after(9); // the host button stays out, so the probe sits where it would be
    scene.open().set(true);
    frames.frame(vec![], |ui| scene.show(ui));
    frames.frame(vec![moved(at), press(at), release(at)], |ui| scene.show(ui));
    for _ in 0..4 {
        frames.frame(vec![], |ui| scene.show(ui));
    }
    assert!(scene.probe_activations() >= 1, "the probe under the tap position takes the press");
}

/// A later tap, when the content is in the record: it lands, the way the early one now must too.
#[trial]
fn a_later_single_frame_tap_inside_the_modal_reaches_the_host_button() {
    let at = host_at();
    let mut frames = Frames::new();
    let mut scene = Scene::new();
    scene.show_after(2);
    scene.open().set(true);
    for _ in 0..4 {
        frames.frame(vec![], |ui| scene.show(ui));
    }
    frames.frame(vec![moved(at), press(at), release(at)], |ui| scene.show(ui));
    for _ in 0..4 {
        frames.frame(vec![], |ui| scene.show(ui));
    }
    assert_eq!(scene.host_clicked(), 1, "the settled tap lands");
    assert_eq!(scene.behind_clicked(), 0, "and nothing behind the cover");
}

/// The cover keeps absorbing outside presses: the same single-frame tap on the button behind takes nothing.
#[trial]
fn the_cover_still_absorbs_a_single_frame_tap_on_the_content_behind_the_modal() {
    let mut frames = Frames::new();
    let mut scene = Scene::new();
    scene.open().set(true);
    for _ in 0..4 {
        frames.frame(vec![], |ui| scene.show(ui));
    }
    let at = scene.behind.borrow().0.center();
    frames.frame(vec![moved(at), press(at), release(at)], |ui| scene.show(ui));
    for _ in 0..4 {
        frames.frame(vec![], |ui| scene.show(ui));
    }
    assert_eq!(scene.behind_clicked(), 0, "the button behind the cover takes no press");
    assert_eq!(scene.host_clicked(), 0, "nor does anything reach the modal's content");
}

/// A reopen: a single-frame tap one pass after the modal shows again, while its host content is a pass behind its
/// cover, must land too - the pass window photocraft measured after a reopen.
#[trial]
fn a_single_frame_tap_one_pass_after_a_reopen_reaches_the_late_content() {
    let at = host_at();
    let mut frames = Frames::new();
    let mut scene = Scene::new();
    scene.show_after(2);
    scene.open().set(true);
    for _ in 0..5 {
        frames.frame(vec![], |ui| scene.show(ui));
    }
    scene.open().set(false);
    for _ in 0..5 {
        frames.frame(vec![], |ui| scene.show(ui));
    }
    scene.open().set(true);
    frames.frame(vec![], |ui| scene.show(ui)); // shows again: cover, no host content yet
    frames.frame(vec![moved(at), press(at), release(at)], |ui| scene.show(ui));
    for _ in 0..4 {
        frames.frame(vec![], |ui| scene.show(ui));
    }
    assert_eq!(scene.host_clicked(), 1, "the tap after the reopen lands");
    assert_eq!(scene.behind_clicked(), 0, "and nothing behind the cover");
}

/// A tap that arrives before the content exists at its place decides on what was there (the probe holds the slot
/// until the button shows): it stays that click, and the re-tap rule keeps out. The same tap after the content
/// shows lands on the content - measured on both sides of the boundary.
#[trial]
fn a_tap_before_the_content_exists_decides_on_what_was_there_and_a_later_tap_lands() {
    let at = host_at();
    let mut frames = Frames::new();
    let mut scene = Scene::new();
    scene.show_after(3);
    scene.open().set(true);
    frames.frame(vec![], |ui| scene.show(ui)); // the cover shows, the probe holds the slot
    frames.frame(vec![moved(at), press(at), release(at)], |ui| scene.show(ui)); // the tap: no button there yet
    frames.frame(vec![], |ui| scene.show(ui));
    frames.frame(vec![], |ui| scene.show(ui));
    assert_eq!(scene.host_clicked(), 0, "the early tap does not reach a button that did not exist");
    assert_eq!(scene.probe_activations(), 1, "it decides on the widget that was under it");
    frames.frame(vec![moved(at), press(at), release(at)], |ui| scene.show(ui)); // the tap again: the button shows this pass
    for _ in 0..4 {
        frames.frame(vec![], |ui| scene.show(ui));
    }
    assert_eq!(scene.host_clicked(), 1, "the later tap lands on the content");
    assert_eq!(scene.behind_clicked(), 0, "and nothing behind the cover");
}

/// The press that opens the modal keeps its click: the button that sits where the modal's content will show is
/// clicked once, and the opening tap is not re-tapped inside the new modal.
#[trial]
fn the_click_that_opens_the_modal_is_not_re_tapped_into_it() {
    let mut frames = Frames::new();
    let mut scene = Scene::new();
    scene.put_front(true);
    for _ in 0..3 {
        frames.frame(vec![], |ui| scene.show(ui));
    }
    let at = scene.front_rect().center();
    frames.frame(vec![moved(at), press(at), release(at)], |ui| scene.show(ui));
    for _ in 0..6 {
        frames.frame(vec![], |ui| scene.show(ui));
    }
    assert!(scene.open().get(), "the click opened the modal");
    assert_eq!(scene.front_clicked(), 1, "the opener takes its click once");
    assert_eq!(scene.host_clicked(), 0, "and nothing was re-tapped inside the new modal");
}

/// A press and release in separate frames is not the measured case: with the content a pass behind, it decides
/// on what was there (the probe) and the re-tap rule keeps out - today's behaviour, pinned to stay.
#[trial]
fn a_press_and_release_in_separate_frames_decides_on_what_was_there() {
    let at = host_at();
    let mut frames = Frames::new();
    let mut scene = Scene::new();
    scene.show_after(3);
    scene.open().set(true);
    frames.frame(vec![], |ui| scene.show(ui)); // the cover shows, the probe holds the slot
    frames.frame(vec![moved(at), press(at)], |ui| scene.show(ui)); // press: no button there yet
    frames.frame(vec![release(at)], |ui| scene.show(ui)); // its release a frame later
    for _ in 0..4 {
        frames.frame(vec![], |ui| scene.show(ui));
    }
    assert_eq!(scene.host_clicked(), 0, "the split press is not re-tapped onto late content");
    assert_eq!(scene.probe_activations(), 1, "it decides on the widget that was under it");
    assert_eq!(scene.behind_clicked(), 0, "and nothing behind the cover");
}
