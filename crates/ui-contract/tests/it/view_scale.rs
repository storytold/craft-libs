//! The view scale (`EnvView::view_scale`): the product of the transform scales of the bodies around a scope, which a
//! child measures for, starting over at 1.0 in a layer opened inside them.

use craft_behavior::CanvasTransform;
use craft_ui::contract::{
    Common, Component, Container, ContainerProps, Cx, MemoProps, MemoView, Mount, Overlay, Props as _, Role, Show as _, ShowContainer as _, Sizing,
};
use craft_ui::runtime::env::Env;
use craft_ui::style::StyleCx;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, MemoRun, last_frame_memo_runs, trial};
use craft_ui_test_kit::contract::{PlainStyle, Probe, Seen};
use egui::{Pos2, Rect, Response, Ui, Vec2};
use std::cell::{Cell, RefCell};

/// A container whose body shows through the transform its props give, or opens a layer.
#[derive(Component)]
#[component(container, unregistered)]
struct Zoomed {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Zoomed {
    fn through(transform: CanvasTransform) -> Self {
        Zoomed { common: Common::new(), props: ContainerProps::new().sizing(Sizing::Fill).transform(transform) }
    }

    fn layer() -> Self {
        Zoomed { common: Common::new(), props: ContainerProps::new().mount(Mount::Layer(Overlay::modal())) }
    }
}

impl Component for Zoomed {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ZoomedStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(120.0, 60.0))
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Zoomed {}

thread_local! {
    /// The view scale the `Scaled` view measured for in its last run.
    static MEASURED: Cell<f32> = const { Cell::new(0.0) };
}

/// A view that sizes its content by the view scale, as text that keeps its size on screen does.
#[derive(MemoProps)]
struct Scaled {
    width: f32,
}

fn scaled(props: &mut Scaled, ui: &mut Ui) {
    let scale = Env::of(ui).view().view_scale();
    MEASURED.with(|measured| measured.set(scale));
    let seen = RefCell::new(Seen::default());
    Probe::new(&seen).size(Vec2::new(props.width / scale, 20.0)).id_salt("content").show(ui);
}

/// How many times the `Scaled` view ran in the last frame.
fn runs(frames: &Frames) -> usize {
    last_frame_memo_runs(&frames.ctx).into_iter().filter(|(name, _, run)| name.contains("Scaled") && *run == MemoRun::Ran).count()
}

/// What each scope read.
#[derive(Default)]
struct Read {
    outside: f32,
    outer: f32,
    inner: f32,
    layer: Option<f32>,
}

/// ge.C12 (q51): nested transformed bodies multiply their scales; a layer opened inside them reads 1.0; a child that
/// measures from the view scale runs again when the zoom changes and not when the view pans.
#[trial]
fn view_scale_multiplies_through_zoomed_bodies_and_starts_over_in_a_layer() {
    let outer = Cell::new(CanvasTransform::new(2.0, Vec2::ZERO));
    let inner = CanvasTransform::new(1.5, Vec2::new(4.0, 2.0));
    let read = RefCell::new(Read::default());
    let scale_of = |ui: &Ui| Env::of(ui).view().view_scale();
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(500.0, 400.0)), |ui| {
            read.borrow_mut().outside = scale_of(ui);
            Zoomed::through(outer.get()).id_salt("outer").show(ui, |ui, _| {
                read.borrow_mut().outer = scale_of(ui);
                Zoomed::through(inner).id_salt("inner").show(ui, |ui, _| {
                    read.borrow_mut().inner = scale_of(ui);
                    let _ = MemoView::new("scaled", Scaled { width: 80.0 }, scaled).show(ui);
                    Zoomed::layer().id_salt("popup").show(ui, |ui, _| {
                        read.borrow_mut().layer = Some(scale_of(ui));
                    });
                });
            });
        });
    };
    frames.idle(30, &mut show);
    {
        let read = read.borrow();
        assert_eq!(read.outside, 1.0, "outside every transform the scale is 1");
        assert!((read.outer - outer.get().scale).abs() < 1e-4, "one transform gives its scale: {}", read.outer);
        assert!((read.inner - read.outer * inner.scale).abs() < 1e-4, "a nested transform multiplies: {}", read.inner);
        assert_eq!(read.layer, Some(1.0), "a layer opened inside the zoomed bodies starts over");
    }
    assert!((MEASURED.with(Cell::get) - read.borrow().inner).abs() < 1e-4, "the view measured for the scale around it");
    assert_eq!(runs(&frames), 0, "a settled view replays");

    outer.set(CanvasTransform::new(outer.get().scale, Vec2::new(25.0, 10.0)));
    frames.idle(1, &mut show);
    assert_eq!(runs(&frames), 0, "a pan leaves the view's measure as it was");

    outer.set(CanvasTransform::new(3.0, outer.get().offset));
    frames.idle(1, &mut show);
    assert!(runs(&frames) > 0, "a zoom runs the view again");
    assert!((read.borrow().inner - 3.0 * inner.scale).abs() < 1e-4, "the product follows the zoom");
    assert!((MEASURED.with(Cell::get) - read.borrow().inner).abs() < 1e-4, "and the view measured for it");
}
