//! `Pages`: a hidden page is measured only, and its measure is reused while nothing it read changed.

use craft_ui::contract::{Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, Pages, Props as _, Role, Show as _};
use craft_ui::style::StyleCx;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Ui, Vec2};
use std::cell::{Cell, RefCell};

/// A leaf as wide as its bound value, or as `fixed` when given; it sets its value to `set_to` when given.
#[derive(Component)]
struct Width<'a> {
    #[common]
    common: Common,
    value: Bind<'a, f32>,
    fixed: Option<f32>,
    set_to: Option<f32>,
}

impl Component for Width<'_> {
    const ROLE: Role = Role::None;
    type Style = PlainStyle;
    type Output = EditResponse<f32>;

    fn style(_: &StyleCx<'_>, _: WidthStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(self.fixed.unwrap_or_else(|| cx.value().copied().unwrap_or(0.0)), 10.0)
    }

    fn bind(&mut self) -> Option<BindMut<'_, f32>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(to) = self.set_to.take() {
            cx.set(to);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<f32> {
        cx.edit_response()
    }
}

/// Two pages: page 0 shows a fixed-width leaf that can edit `value`; page 1, hidden, is as wide as `value`.
struct Scene {
    /// The value, kept in place across frames: a settled reader knows a bound value by where it lives.
    value: RefCell<f32>,
    /// What page 0's leaf sets the value to in the next pass that shows it.
    set_to: Cell<Option<f32>>,
    /// The runs of the hidden page's closure.
    hidden_runs: Cell<usize>,
    /// The width the pages took in the last pass.
    width: Cell<f32>,
}

impl Scene {
    fn new() -> Scene {
        Scene { value: RefCell::new(60.0), set_to: Cell::new(None), hidden_runs: Cell::new(0), width: Cell::new(0.0) }
    }

    fn show(&self, ui: &mut Ui) {
        let pages = [0_u8, 1];
        let shown = ui.scope(|ui| {
            Pages::new("trial", &pages, &0).show(ui, |ui, page| {
                if *page == 1 {
                    self.hidden_runs.set(self.hidden_runs.get() + 1);
                    let mut value = self.value.borrow_mut();
                    Width { common: Common::new(), value: Bind::bound(&mut *value), fixed: None, set_to: None }.id_salt("reader").show(ui);
                } else {
                    let set_to = self.set_to.take();
                    let mut value = self.value.borrow_mut();
                    Width { common: Common::new(), value: Bind::bound(&mut *value), fixed: Some(20.0), set_to }.id_salt("editor").show(ui);
                }
            });
        });
        self.width.set(shown.response.rect.width());
    }

    /// The hidden page's runs in one frame with no input.
    fn frame(&self, frames: &mut Frames) -> usize {
        self.hidden_runs.set(0);
        frames.frame(vec![], |ui| self.show(ui));
        self.hidden_runs.get()
    }
}

/// The pages take the size of the largest, a hidden one included; in a settled frame the hidden page's measure is
/// reused and its closure runs in no pass.
#[trial]
fn a_settled_hidden_page_runs_no_body() {
    let scene = Scene::new();
    let mut frames = Frames::new();
    frames.idle(30, |ui| scene.show(ui));
    assert!(scene.width.get() > 20.0, "the pages are as wide as the hidden page, wider than the shown one");
    for frame in 0..4 {
        assert_eq!(scene.frame(&mut frames), 0, "settled frame {frame} ran the hidden page");
    }
}

/// An edit of a value the hidden page read measures that page again in the frame of the edit, once, and the pages take
/// its new size there. The edit is a `set`, as a click on a control makes it.
#[trial]
fn an_edit_of_a_value_a_hidden_page_read_measures_it_again() {
    let scene = Scene::new();
    let mut frames = Frames::new();
    frames.idle(30, |ui| scene.show(ui));
    let before = scene.width.get();
    scene.set_to.set(Some(before * 3.0));
    assert_eq!(scene.frame(&mut frames), 1, "the edit measured the hidden page again, once");
    assert!(scene.width.get() > before, "the pages took the hidden page's new width in the frame of the edit");
    frames.idle(30, |ui| scene.show(ui));
    assert_eq!(scene.frame(&mut frames), 0, "the hidden page settles again");
}
