//! The dialog host and stack.

use craft_behavior::{Metrics, PartKey, Sense};
use craft_host::target::{Os, WindowChrome};
use craft_quit::{CloseAnswer, CloseScope};
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PartsCx, Props as _, Role, Show as _};
use craft_ui::dialog::stories::hosted;
use craft_ui::dialog::{
    Alert, DialogHost, DialogId, DialogResult, DialogStack, HostedAlert, PromptKind, StackDialog, StandardPrompt, StartupStage, UnsavedChangesDialog,
    dialog_id,
};
use craft_ui::runtime::windows::{WindowHit, WindowHost};
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::testing::{Fixture, Frames, Story, StoryData, StoryOutput, trial};
use egui::{Modifiers, Rect, Response, Ui, Vec2, ViewportId, vec2};
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

    fn confirm(&mut self, _: &egui::Context) -> Option<u32> {
        Some(N as u32)
    }
}

fn run(host: &mut DialogHost, frames: &mut Frames) {
    frames.frame(Vec::new(), |ui| host.run(ui));
}

/// A fixture that runs `host` each pass.
fn running(host: &Rc<RefCell<DialogHost>>) -> Fixture {
    let shown = Rc::clone(host);
    Fixture::new(move |ui| shown.borrow_mut().run(ui))
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

/// OK on a dialog waits for the child open over it: the host refuses the parent's confirm until the child closes.
#[trial]
fn a_parent_with_an_open_child_refuses_confirm() {
    let mut frames = Frames::new();
    let mut host = DialogHost::new();
    let parent = host.open(Probe::<1>::new("parent"));
    let child = host.open_child(Probe::<2>::new("child"), parent);
    run(&mut host, &mut frames);
    assert!(!host.confirm_by_id(&frames.ctx, parent), "the parent's OK waits for its child");
    assert!(host.is_open::<Probe<1>>() && host.is_open::<Probe<2>>());
    assert!(host.confirm_by_id(&frames.ctx, child));
    assert!(host.confirm_by_id(&frames.ctx, parent), "with the child closed, the parent confirms");
    assert_eq!(host.take::<Probe<1>>(), Some(DialogResult::Ok(1)));
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

/// A hosted modal over the window's title bar shows drag strips above itself, which lift its family into the modal
/// chrome class.
#[trial]
fn a_hosted_modal_over_the_title_bar_shows_drag_strips() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| {
        DragBar { common: Common::new() }.id_salt("bar").show(ui);
        shown.borrow_mut().run(ui);
    });
    fixture.chrome(WindowChrome::Integrated);
    host.borrow_mut().open(StandardPrompt::new(PromptKind::TrashMask));
    fixture.idle(6);
    let ctx = fixture.ctx().clone();
    let moves = WindowHost::of(&ctx).caption_regions(ViewportId::ROOT).map_or(0, |regions| regions.moves.len());
    assert!(moves > 1, "the hosted modal shows a drag strip over the bar");
}

/// `run` shows every open dialog once per pass, a frame's second pass included, and the child over its parent is on
/// top.
#[trial]
fn the_run_shows_each_dialog_once_per_pass() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let rerun = Rc::new(Cell::new(false));
    let (shown, again) = (Rc::clone(&host), Rc::clone(&rerun));
    let mut fixture = Fixture::new(move |ui| {
        shown.borrow_mut().run(ui);
        if again.get() && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the frame runs twice");
        }
    });
    let below = host.borrow_mut().open(Probe::<1>::new("below"));
    host.borrow_mut().open_child(Probe::<2>::new("over"), below);
    let mut passes = 0;
    for frame in 0..5 {
        rerun.set(frame % 2 == 1);
        passes += fixture.frame(Vec::new()).passes();
    }
    assert!(passes > 5, "some frames ran a second pass");
    let host = host.borrow();
    let (below, over) = (host.get::<Probe<1>>().expect("open"), host.get::<Probe<2>>().expect("open"));
    assert_eq!((below.shows, over.shows), (passes, passes));
    assert!(over.top && !below.top, "the child over its parent is on top");
}

/// The startup queue opens its modals one at a time, in stage order, each once nothing else is open.
#[trial]
fn a_startup_modal_waits_for_the_open_dialogs_and_opens_in_stage_order() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = running(&host);
    host.borrow_mut().open(Probe::<1>::new("open"));
    host.borrow_mut().enqueue(StartupStage::Update, Probe::<4>::new("update"));
    host.borrow_mut().enqueue(StartupStage::Warning, Probe::<3>::new("warning"));
    fixture.idle(3);
    {
        let host = host.borrow();
        assert!(host.is_open::<Probe<1>>() && !host.is_open::<Probe<3>>() && !host.is_open::<Probe<4>>(), "the queue waits for the open dialog");
    }

    host.borrow_mut().get_mut::<Probe<1>>().expect("open").finish = Some(DialogResult::Ok(1));
    fixture.idle(3);
    {
        let host = host.borrow();
        assert!(!host.is_open::<Probe<1>>() && host.is_open::<Probe<3>>(), "the warning opens once the dialog closes");
        assert!(!host.is_open::<Probe<4>>(), "the update queued first waits for the earlier stage");
    }

    host.borrow_mut().get_mut::<Probe<3>>().expect("open").finish = Some(DialogResult::Ok(3));
    fixture.idle(3);
    assert!(host.borrow().is_open::<Probe<4>>(), "the update opens after the warning");
}

/// On macOS Cmd+D answers Don't Save on the unsaved-changes question and the app sees no press; Linux has no such
/// chord.
#[trial]
fn cmd_d_answers_dont_save_on_macos() {
    for (os, answers) in [(Os::Mac, true), (Os::Linux, false)] {
        let host = Rc::new(RefCell::new(DialogHost::new()));
        let app_saw_d = Rc::new(Cell::new(false));
        let (shown, saw) = (Rc::clone(&host), Rc::clone(&app_saw_d));
        let mut fixture = Fixture::new(move |ui| {
            let pressed =
                ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::D, pressed: true, .. })));
            saw.set(saw.get() || pressed);
            shown.borrow_mut().run(ui);
        });
        fixture.os(os);
        host.borrow_mut().open(UnsavedChangesDialog::new(CloseScope::Close, "poster.psd"));
        fixture.idle(6);
        fixture.key(egui::Key::D, Modifiers::COMMAND).idle(4);
        let result = host.borrow_mut().take::<UnsavedChangesDialog>();
        if answers {
            assert_eq!(
                result.and_then(DialogResult::ok).map(|answer| answer.answer),
                Some(CloseAnswer::DontSave),
                "{os:?}: Cmd+D answers Don't Save"
            );
            assert!(!app_saw_d.get(), "{os:?}: the press reaches nothing but the question");
        } else {
            assert!(result.is_none() && host.borrow().is_open::<UnsavedChangesDialog>(), "{os:?}: Ctrl+D leaves the question open");
        }
    }
}

/// Each open of a dialog has its own serial, kept while it stays open, though its id is the same at every open: a
/// reopened dialog is told apart from the one cancelled before it.
#[trial]
fn each_open_of_a_dialog_has_its_own_serial() {
    let mut frames = Frames::new();
    let mut host = DialogHost::new();
    let id = host.open(Probe::<1>::new("picker"));
    let first = host.open_serial(id).expect("an open dialog has a serial");
    run(&mut host, &mut frames);
    assert_eq!(host.open_serial(id), Some(first), "the serial stays while the dialog is open");
    assert!(host.cancel_by_id(&frames.ctx, id));
    assert_eq!(host.open_serial(id), None, "a closed dialog has none");
    let again = host.open(Probe::<1>::new("picker"));
    assert_eq!(again, id, "the reopened dialog has the same id");
    assert_ne!(host.open_serial(id), Some(first), "the reopened dialog has a serial of its own");
}
