//! Rules of the pipeline's steps: the order the hooks run in, sizing passes, `measure_in`, activation, ids, focus
//! on a part that goes away, the body size and the ink and app values a body hands down.

use craft_behavior::Sense;
use craft_ui::contract::{
    Common, Component, Container, ContainerProps, Cx, MeasureCx, OverlayUi, PaintUi, PartsCx, Props as _, Role, Show as _, ShowContainer as _, Sizing,
};
use craft_ui::runtime::env::{AppValues, EnvValue};
use craft_ui::style::StyleCx;
use craft_ui::testing::contract::{findings, steps};
use craft_ui::testing::focus::named_id;
use craft_ui::testing::memory::scoped;
use craft_ui::testing::{Frames, is_measuring, trial};
use craft_ui_test_kit::contract::{PlainStyle, Probe, Seen};
use egui::{Rect, Response, UiBuilder, Vec2};
use std::cell::RefCell;

type Log = RefCell<Vec<&'static str>>;

/// A style-affecting app value the body hands to its children.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, EnvValue)]
#[env(style, root = Tone(1))]
struct Tone(u8);

/// A body that logs every hook the pipeline calls and can hand `Tone` to its body.
#[derive(Component)]
#[component(container, overlay)]
struct Hooked<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    log: &'a Log,
    /// Where the calls of a measuring run go, when the trial tells the runs apart.
    measured: Option<(&'a Log, egui::Context)>,
    tone: Option<u8>,
}

impl<'a> Hooked<'a> {
    fn new(log: &'a Log) -> Self {
        Hooked { common: Common::new(), props: ContainerProps::new().sizing(Sizing::Content), log, measured: None, tone: None }
    }

    fn telling_runs_apart(mut self, measured: &'a Log, ctx: &egui::Context) -> Self {
        self.measured = Some((measured, ctx.clone()));
        self
    }

    fn mark(&self, hook: &'static str) {
        match &self.measured {
            Some((measured, ctx)) if is_measuring(ctx) => measured.borrow_mut().push(hook),
            _ => self.log.borrow_mut().push(hook),
        }
    }
}

impl Component for Hooked<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: HookedStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        self.mark("measure");
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, _: Rect, _: &mut PartsCx<'_, Self>) {
        self.mark("parts");
    }

    fn content_env(&self, _: &Cx<'_, Self>, values: &mut AppValues) {
        self.mark("content_env");
        if let Some(tone) = self.tone {
            values.set(Tone(tone));
        }
    }

    fn update(&mut self, _: &mut Cx<'_, Self>) {
        self.mark("update");
    }

    fn overlay(&mut self, _: &mut OverlayUi<'_>, _: &mut Cx<'_, Self>) {
        self.mark("overlay");
    }

    fn flags(&self) -> craft_ui::style::Flags {
        self.mark("flags");
        craft_ui::style::Flags::empty()
    }

    fn activated(&self, _: &Cx<'_, Self>) -> bool {
        self.mark("activated");
        false
    }

    fn paint(&mut self, _: &mut PaintUi<'_>, _: &Cx<'_, Self>) {
        self.mark("paint");
    }

    fn paint_over(&mut self, _: &mut PaintUi<'_>, _: &Cx<'_, Self>) {
        self.mark("paint_over");
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        self.mark("finish");
        cx.response.clone()
    }
}

impl Container for Hooked<'_> {}

#[trial]
fn hooks_run_in_pipeline_order_with_the_body_between_paint_and_paint_over() {
    let mut frames = Frames::new();
    let log: Log = RefCell::new(Vec::new());
    let measured: Log = RefCell::new(Vec::new());
    frames.frame(vec![], |ui| {
        log.borrow_mut().clear();
        measured.borrow_mut().clear();
        let ctx = ui.ctx().clone();
        Hooked::new(&log).telling_runs_apart(&measured, &ctx).show(ui, |ui, _| {
            let into = if is_measuring(ui.ctx()) { &measured } else { &log };
            into.borrow_mut().push("body");
        });
    });
    assert_eq!(
        *log.borrow(),
        ["measure", "parts", "update", "overlay", "flags", "content_env", "paint", "body", "paint_over", "activated", "finish"],
        "the real run"
    );
    let mut order = ["content_env", "body", "measure"].into_iter();
    assert!(!measured.borrow().is_empty(), "the measuring run called the hooks");
    assert!(
        measured.borrow().iter().all(|call| order.any(|allowed| allowed == *call)),
        "a measuring run calls only content_env, body, measure, in this order: {:?}",
        measured.borrow()
    );
}

/// A sizing pass of the parent `Ui` allocates the size and runs nothing else.
#[trial]
fn a_sizing_pass_registers_nothing_and_its_id_is_the_real_passs() {
    let sized_id = {
        let mut frames = Frames::new();
        let seen = RefCell::new(Seen::default());
        let mut id = None;
        frames.frame(vec![], |ui| {
            id = Some(ui.scope_builder(UiBuilder::new().sizing_pass(), |ui| Probe::new(&seen).show(ui)).inner.id);
        });
        assert_eq!(seen.borrow().updates, 0, "a sizing pass runs no update");
        id
    };
    let mut frames = Frames::new();
    let seen = RefCell::new(Seen::default());
    let mut real_id = None;
    frames.frame(vec![], |ui| {
        real_id = Some(scoped(ui, |ui| Probe::new(&seen).show(ui)).id);
    });
    assert_eq!(sized_id, real_id, "the Auto ordinal of a sizing pass is the real pass's");

    // The id a sizing pass used is free for the real show in the same pass.
    let mut frames = Frames::new();
    let seen = RefCell::new(Seen::default());
    let shared = named_id("shared");
    frames.frame(vec![], |ui| {
        ui.scope_builder(UiBuilder::new().sizing_pass(), |ui| Probe::new(&seen).id(shared).show(ui));
        Probe::new(&seen).id(shared).show(ui);
    });
    assert!(findings(&frames.ctx).is_empty(), "a sizing pass registers no id: {:?}", findings(&frames.ctx));
}

/// A leaf whose width follows `Tone`.
#[derive(Component)]
struct Chip {
    #[common]
    common: Common,
}

impl Component for Chip {
    const ROLE: Role = Role::None;
    type Style = PlainStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: ChipStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::splat(f32::from(cx.env::<Tone>().0) * 10.0))
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[trial]
fn a_style_value_set_in_content_env_changes_a_body_childs_style() {
    let mut frames = Frames::new();
    let log: Log = RefCell::new(Vec::new());
    let (mut outside, mut inside) = (0.0, 0.0);
    frames.frame(vec![], |ui| {
        outside = Chip { common: Common::new() }.id_salt("outside").show(ui).rect.width();
        let mut body = Hooked::new(&log).id_salt("scoped");
        body.tone = Some(5);
        body.show(ui, |ui, _| inside = Chip { common: Common::new() }.id_salt("inside").show(ui).rect.width());
    });
    assert!(inside > outside, "the body's child read the body's value: {outside} -> {inside}");
}

/// The step log, a record only trials and the debug inspector read, is kept in frames a test fixture drives; a frame
/// an app drives itself, such as the gallery's, keeps none.
#[trial]
fn the_step_log_is_kept_in_fixture_frames_only() {
    let mut fixture = Frames::new();
    let seen = RefCell::new(Seen::default());
    fixture.frame(Vec::new(), |ui| {
        Probe::new(&seen).show(ui);
    });
    let id = seen.borrow().id.expect("the probe ran");
    assert!(!steps(&fixture.ctx, id).is_empty(), "a fixture frame records the probe's steps");

    let app = Frames::new();
    let seen = RefCell::new(Seen::default());
    let mut output = app.ctx.run_ui(craft_ui::testing::windows::RawInput::default(), |ui| {
        Probe::new(&seen).show(ui);
    });
    output.textures_delta.clear();
    let id = seen.borrow().id.expect("the probe ran");
    assert!(steps(&app.ctx, id).is_empty(), "an app frame records no steps");
}
