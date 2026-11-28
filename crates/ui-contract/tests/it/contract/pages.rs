//! `Pages`: a hidden page is measured only, once in each pass.

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
    /// The value both pages bind.
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

    /// The hidden page's runs in one frame with no input, and the frame's passes.
    fn frame(&self, frames: &mut Frames) -> (usize, usize) {
        self.hidden_runs.set(0);
        let passes = frames.frame(vec![], |ui| self.show(ui));
        (self.hidden_runs.get(), passes)
    }
}

/// The pages take the size of the largest, a hidden one included, and each pass measures the hidden page once: the
/// page closure reads app data the framework does not track, so no pass takes the size an earlier pass measured.
#[trial]
fn a_hidden_page_is_measured_once_in_each_pass() {
    let scene = Scene::new();
    let mut frames = Frames::new();
    frames.idle(30, |ui| scene.show(ui));
    assert!(scene.width.get() > 20.0, "the pages are as wide as the hidden page, wider than the shown one");
    for frame in 0..4 {
        let (runs, passes) = scene.frame(&mut frames);
        assert_eq!(runs, passes, "settled frame {frame} measured the hidden page once in each pass");
    }
}

/// An edit of a value the hidden page read earlier in the pass runs the pass again, which measures that page again,
/// and the pages take its new size in the frame of the edit. The edit is a `set`, as a click on a control makes it.
#[trial]
fn an_edit_of_a_value_a_hidden_page_read_measures_it_again() {
    let scene = Scene::new();
    let mut frames = Frames::new();
    frames.idle(30, |ui| scene.show(ui));
    let before = scene.width.get();
    scene.set_to.set(Some(before * 3.0));
    let (runs, passes) = scene.frame(&mut frames);
    assert!(passes > 1, "the edit of a value read earlier in the pass ran the pass again");
    assert_eq!(runs, passes, "each pass measured the hidden page once");
    assert!(scene.width.get() > before, "the pages took the hidden page's new width in the frame of the edit");
}
