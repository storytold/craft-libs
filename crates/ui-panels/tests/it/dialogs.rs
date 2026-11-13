//! The dialog host and stack.

use craft_behavior::{Metrics, PartKey, Sense};
use craft_host::target::WindowChrome;
use craft_ui::contract::overlay::show_beside_modal;
use craft_ui::contract::{Bind, Common, Component, Cx, MeasureCx, PartsCx, Props as _, Role, Show as _, ShowContainer as _};
use craft_ui::dialog::stories::hosted;
use craft_ui::dialog::{Alert, DialogHost, DialogId, DialogResult, DialogStack, HostedAlert, PromptKind, StackDialog, StandardPrompt, dialog_id};
use craft_ui::popover::Popover;
use craft_ui::runtime::overlay::{family, is_top_modal};
use craft_ui::runtime::windows::{WindowHit, WindowHost};
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::testing::{Fixture, Frames, Story, StoryData, StoryOutput, trial};
use egui::{Context, Id, LayerId, Order, Rect, Response, Ui, Vec2, ViewportId, vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A dialog with no content: it records whether it was the top of the stack when shown and finishes with
/// `finish` once set.
struct Probe<const N: usize> {
    key: &'static str,
    top: bool,
    finish: Option<DialogResult<u32>>,
    shows: usize,
}

impl<const N: usize> Probe<N> {
    fn new(key: &'static str) -> Self {
        Self { key, top: false, finish: None, shows: 0 }
    }
}

impl<const N: usize> StackDialog for Probe<N> {
    type Result = u32;

    fn id(&self) -> DialogId {
        dialog_id(self.key)
    }

    fn show(&mut self, _: &mut Ui, stack: &DialogStack) -> Option<DialogResult<u32>> {
        self.top = stack.is_top(StackDialog::id(self));
        self.shows += 1;
        self.finish.take()
    }
}

fn run(host: &mut DialogHost, frames: &mut Frames) {
    frames.frame(Vec::new(), |ui| host.run(ui));
}

#[trial]
fn cancelling_a_parent_cancels_its_children() {
    let mut frames = Frames::new();
    let mut host = DialogHost::new();
    let parent = host.open(Probe::<1>::new("parent"));
    host.open_child(Probe::<2>::new("child"), parent);
    run(&mut host, &mut frames);

    host.get_mut::<Probe<2>>().unwrap().finish = Some(DialogResult::Cancel);
    run(&mut host, &mut frames);
    assert!(host.is_open::<Probe<1>>() && !host.is_open::<Probe<2>>());
    assert_eq!(host.take::<Probe<2>>(), Some(DialogResult::Cancel));

    host.open_child(Probe::<2>::new("child"), parent);
    assert!(host.cancel::<Probe<1>>(&frames.ctx));
    assert!(host.is_empty() && host.stack().is_empty());
    assert_eq!(host.take::<Probe<1>>(), Some(DialogResult::Cancel));
    assert_eq!(host.take::<Probe<2>>(), Some(DialogResult::Cancel));
}

/// A story of a hosted alert whose title is the story's `title` knob.
fn titled_alert(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let title = *data.value("title", "Could not open");
    hosted(ui, data, title, || HostedAlert::new(Alert::new(title, "The file is damaged.")))
}

#[trial]
fn a_story_knob_reaches_its_open_hosted_dialog() {
    let mut fixture = Fixture::story(&Story::new("titled_alert", titled_alert));
    fixture.idle(6);
    assert!(fixture.by_label("Could not open").is_some(), "the story opens its dialog");
    assert!(fixture.take_outputs().last().is_some_and(|output| output.id.is_some()), "the story reports the open dialog's facts");
    *fixture.data().value("title", "Could not open") = "Could not save";
    fixture.idle(6);
    assert!(fixture.by_label("Could not save").is_some(), "the changed knob shows in the open dialog");
    assert!(fixture.by_label("Could not open").is_none(), "the dialog of the old knob value is gone");
}

#[trial]
fn a_hosted_alert_answers_ok_however_it_closes_and_opens_again() {
    let host = std::rc::Rc::new(std::cell::RefCell::new(DialogHost::new()));
    let shown = std::rc::Rc::clone(&host);
    let mut fixture = craft_ui::testing::Fixture::new(move |ui| shown.borrow_mut().run(ui));
    for key in [egui::Key::Enter, egui::Key::Escape] {
        host.borrow_mut().open(HostedAlert::new(Alert::new("Could not open", "The file is damaged.")));
        fixture.idle(6);
        assert!(fixture.by_label("Could not open").is_some(), "the alert shows on the host before {key:?}");
        fixture.key(key, egui::Modifiers::NONE).idle(4);
        assert!(host.borrow().is_empty(), "{key:?} closes the alert");
        assert_eq!(host.borrow_mut().take::<HostedAlert>(), Some(DialogResult::Ok(())), "{key:?} answers OK");
    }
}

/// The app's own dialogs, drawn in the host's walk: each an egui modal with an open egui popup, and a framework
/// popover inside while `popover` holds.
#[derive(Default)]
struct AppDialogs {
    shows: Cell<usize>,
    popover: Cell<bool>,
}

fn app_modal(id: DialogId) -> Id {
    Id::unique(("app modal", id))
}

fn app_popup(id: DialogId) -> Id {
    Id::unique(("app popup", id))
}

fn show_app_dialog(ctx: &Context, id: DialogId, app: &AppDialogs) {
    app.shows.set(app.shows.get() + 1);
    show_beside_modal(ctx, id, egui::Modal::new(app_modal(id)), |ui| {
        ui.allocate_space(vec2(600.0, 450.0));
        egui::Popup::new(app_popup(id), ctx.clone(), ui.min_rect().left_top(), ui.layer_id()).show(|ui| ui.label("app popup"));
        let mut open = app.popover.get();
        Popover::new("Framework popover", Bind::bound(&mut open)).show(ui, |ui, _| {
            ui.label("inside the popover");
        });
        app.popover.set(open);
    });
}

/// Shows every dialog of `host` in one walk, the app's through `show_app_dialog`.
fn walk(ui: &mut Ui, host: &RefCell<DialogHost>, app: &AppDialogs) {
    let mut walk = host.borrow_mut().walk(ui.ctx());
    loop {
        let step = host.borrow_mut().next_beside(ui, &mut walk);
        let Some(step) = step else { break };
        step.at_place(ui.ctx(), || show_app_dialog(ui.ctx(), step.id, app));
    }
}

/// A fixture that shows every dialog of `host` in one walk.
fn walking(host: &Rc<RefCell<DialogHost>>, app: &Rc<AppDialogs>) -> Fixture {
    let (host, app) = (Rc::clone(host), Rc::clone(app));
    Fixture::new(move |ui| walk(ui, &host, &app))
}

const BAND: PartKey = PartKey::new("band");

/// A style with no look.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct PlainStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// An app's own title bar: one drag area of the window.
#[derive(craft_ui::contract::Component)]
struct DragBar {
    #[common]
    common: Common,
}

impl Component for DragBar {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: DragBarStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        vec2(400.0, 32.0)
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(BAND, rect).sense(Sense::DRAG).exclusive(WindowHit::Move).pointer_only("the window command set moves the window");
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// A dialog the app draws itself stacks at its place on the host's stack: above the host's dialog it opened over,
/// with its egui popup above it, and under the host's dialog opened over it, whatever is pressed.
#[trial]
fn app_dialogs_draw_and_take_input_at_their_stack_place() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let app = Rc::new(AppDialogs::default());
    let mut fixture = walking(&host, &app);
    let own = dialog_id(("app", 1));
    host.borrow_mut().open(StandardPrompt::new(PromptKind::TrashMask));
    fixture.idle(6);
    host.borrow_mut().open_beside(own, None);
    fixture.idle(6);
    let ctx = fixture.ctx().clone();
    let centre = ctx.content_rect().center();
    let modal = LayerId::new(Order::Foreground, app_modal(own));
    let popup = LayerId::new(Order::Foreground, app_popup(own));
    assert!(is_top_modal(&ctx, own), "the app's dialog opened over the host's is the top modal");
    assert_eq!(ctx.layer_id_at(centre), Some(modal), "the app's dialog draws over the host's");
    let order: Vec<LayerId> = ctx.memory(|memory| memory.layer_ids().collect());
    let at = |layer| order.iter().position(|shown| *shown == layer);
    assert!(at(popup) > at(modal), "the egui popup stays above its dialog");
    let popup_rect = ctx.memory(|memory| memory.area_rect(popup.id)).expect("the popup shows");
    assert_eq!(ctx.layer_id_at(popup_rect.center()), Some(popup), "nothing covers the popup");

    host.borrow_mut().open_child(HostedAlert::new(Alert::new("Over the app", "The file is damaged.")), own);
    fixture.idle(6);
    assert!(!is_top_modal(&ctx, own), "the host's dialog opened over the app's is on top");
    assert_ne!(ctx.layer_id_at(centre), Some(modal));
    let card = ctx.memory(|memory| memory.area_rect(modal.id)).expect("the app's dialog shows");
    let corner = card.min + vec2(8.0, 8.0);
    let child = fixture.by_label("Over the app").expect("the child shows").rect;
    assert!(!child.contains(corner));
    fixture.click_at(corner).idle(4);
    assert!(!is_top_modal(&ctx, own), "a press on the app's dialog under the child doesn't raise it");
    assert_ne!(ctx.layer_id_at(centre), Some(modal));

    host.borrow_mut().close_beside(own);
    fixture.idle(4);
    assert_eq!(host.borrow_mut().take::<HostedAlert>(), Some(DialogResult::Cancel), "the child closes with the app's dialog");
    assert!(host.borrow().is_open::<StandardPrompt>(), "the dialog under it stays");
}

/// A hosted modal over the window's title bar shows drag strips above itself, which lift its family into the modal
/// chrome class; a dialog the app opens over it still stacks above it.
#[trial]
fn an_app_dialog_stacks_over_a_hosted_modal_with_drag_strips() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let app = Rc::new(AppDialogs::default());
    let (shown, drawn) = (Rc::clone(&host), Rc::clone(&app));
    let mut fixture = Fixture::new(move |ui| {
        DragBar { common: Common::new() }.id_salt("bar").show(ui);
        walk(ui, &shown, &drawn);
    });
    fixture.chrome(WindowChrome::Integrated);
    let own = dialog_id(("app", 1));
    host.borrow_mut().open(StandardPrompt::new(PromptKind::TrashMask));
    fixture.idle(6);
    let ctx = fixture.ctx().clone();
    let moves = || WindowHost::of(&ctx).caption_regions(ViewportId::ROOT).map_or(0, |regions| regions.moves.len());
    assert!(moves() > 1, "the hosted modal shows a drag strip over the bar");
    host.borrow_mut().open_beside(own, None);
    fixture.idle(6);
    assert!(is_top_modal(&ctx, own), "the app's dialog opened over the hosted modal is the top modal");
    let centre = ctx.content_rect().center();
    assert_eq!(ctx.layer_id_at(centre), Some(LayerId::new(Order::Foreground, app_modal(own))), "the app's dialog draws on top");
}

/// One walk shows every open dialog once per pass, the host's and the app's alike.
#[trial]
fn the_walk_shows_each_dialog_once_per_pass() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let app = Rc::new(AppDialogs::default());
    let mut fixture = walking(&host, &app);
    let own = dialog_id(("app", 1));
    host.borrow_mut().open(Probe::<1>::new("below"));
    host.borrow_mut().open_beside(own, None);
    host.borrow_mut().open_child(Probe::<2>::new("over"), own);
    let mut passes = 0;
    for _ in 0..5 {
        passes += fixture.frame(Vec::new()).passes();
    }
    let host = host.borrow();
    let (below, over) = (host.get::<Probe<1>>().expect("open").shows, host.get::<Probe<2>>().expect("open").shows);
    assert_eq!((below, app.shows.get(), over), (passes, passes, passes));
    assert!(host.get::<Probe<2>>().is_some_and(|probe| probe.top), "the host's dialog over the app's is on top");
}

/// A framework popup open inside a dialog the app draws itself closes when that dialog closes.
#[trial]
fn a_framework_popup_in_an_app_dialog_closes_with_it() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let app = Rc::new(AppDialogs::default());
    let mut fixture = walking(&host, &app);
    let own = dialog_id(("app", 1));
    app.popover.set(true);
    host.borrow_mut().open_beside(own, None);
    fixture.idle(6);
    let ctx = fixture.ctx().clone();
    assert_eq!(family(&ctx, own).len(), 2, "the popover shows in the app's dialog, opened from inside it");
    host.borrow_mut().close_beside(own);
    fixture.idle(2);
    host.borrow_mut().open_beside(own, None);
    fixture.idle(30);
    assert!(!app.popover.get(), "the popover closed with its dialog");
    assert_eq!(family(&ctx, own).len(), 1);
}
