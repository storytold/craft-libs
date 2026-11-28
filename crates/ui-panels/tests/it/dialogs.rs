//! The dialog host and stack.

use craft_behavior::{Metrics, MotionMode, PartKey, Sense};
use craft_host::target::{Os, WindowChrome};
use craft_quit::{CloseAnswer, CloseScope};
use craft_ui::contract::overlay::{Dismiss, OpenFilter, Outside, any_open};
use craft_ui::contract::{Bind, Common, Component, Cx, MeasureCx, PartsCx, Props as _, Role, Show as _, ShowContainer as _};
use craft_ui::dialog::stories::hosted;
use craft_ui::dialog::{
    Alert, ChildDialog, Dialog, DialogHost, DialogId, DialogResult, DialogStack, HostedAlert, JobAnswer, JobProgress, Prompt, PromptChoice,
    PromptKind, RecoveredDocument, RecoveryAnswer, RecoveryChoice, RecoveryDialog, StackDialog, StandardPrompt, StartupStage, UnsavedChangesDialog,
    dialog_id,
};
use craft_ui::display::Text;
use craft_ui::popover::Popover;
use craft_ui::runtime::commands::{CommandSet as _, EditCommands};
use craft_ui::runtime::frame::take_output;
use craft_ui::runtime::windows::{WindowHit, WindowHost};
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::testing::{Fixture, Frames, Story, StoryData, StoryOutput, pointer, trial};
use craft_ui::text::TextField;
use egui::{Modifiers, Pos2, Rect, Response, Ui, Vec2, ViewportId, vec2};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// A dialog with no content: it records whether it was the top of the stack when shown and finishes with
/// `finish` once set.
struct Probe<const N: usize> {
    key: &'static str,
    instance: Option<DialogId>,
    top: bool,
    finish: Option<DialogResult<u32>>,
    shows: usize,
}

impl<const N: usize> Probe<N> {
    fn new(key: &'static str) -> Self {
        Self { key, instance: None, top: false, finish: None, shows: 0 }
    }
}

impl<const N: usize> StackDialog for Probe<N> {
    type Result = u32;

    fn id(&self) -> DialogId {
        dialog_id(self.key)
    }

    fn instance(&self) -> Option<DialogId> {
        self.instance
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

/// A dialog that asks for a `Probe<3>` child in the show after `ask` is set, and finishes with `finish` once set.
struct Asker {
    ask: bool,
    child: Option<ChildDialog>,
    finish: Option<DialogResult<u32>>,
}

impl StackDialog for Asker {
    type Result = u32;

    fn id(&self) -> DialogId {
        dialog_id("asker")
    }

    fn show(&mut self, _: &mut Ui, _: &DialogStack) -> Option<DialogResult<u32>> {
        if std::mem::take(&mut self.ask) {
            self.child = Some(ChildDialog::new(Probe::<3>::new("asked")));
        }
        self.finish.take()
    }

    fn take_child(&mut self) -> Option<ChildDialog> {
        self.child.take()
    }
}

/// A child a dialog asks for in its show opens over it, once, and shows from the next pass; a dialog that finishes in
/// the show that asked takes its request with it.
#[trial]
fn a_child_a_dialog_asks_for_opens_over_it_once() {
    let mut frames = Frames::new();
    let mut host = DialogHost::new();
    let asker = host.open(Asker { ask: false, child: None, finish: None });
    run(&mut host, &mut frames);
    assert!(!host.is_open::<Probe<3>>(), "no request, no child");

    host.get_mut::<Asker>().unwrap().ask = true;
    run(&mut host, &mut frames);
    assert!(host.is_open::<Probe<3>>(), "the request opens the child in the run of the show that asked");
    assert_eq!(host.get::<Probe<3>>().unwrap().shows, 0, "a child opened during the run shows from the next pass");
    let parent = host.open_dialogs().into_iter().find(|open| open.id == dialog_id("asked")).and_then(|open| open.parent);
    assert_eq!(parent, Some(asker));

    run(&mut host, &mut frames);
    assert_eq!(host.get::<Probe<3>>().unwrap().shows, 1);
    assert_eq!(host.open_dialogs().len(), 2, "the request is taken once");

    assert!(host.cancel::<Probe<3>>(&frames.ctx));
    let dialog = host.get_mut::<Asker>().unwrap();
    dialog.ask = true;
    dialog.finish = Some(DialogResult::Cancel);
    run(&mut host, &mut frames);
    assert!(host.is_empty(), "a dialog that finished does not leave its child open");
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

/// A hosted dialog that leaves its open state to the dialog shell (no `opened` binding); its OK closes it.
struct SelfClosing;

impl StackDialog for SelfClosing {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("self-closing")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let shown = Dialog::new("Reopen probe").buttons([FooterButton::new("OK", DialogRole::Accept)]).show(ui, |_, _| {});
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }
}

/// A dialog whose shell closed itself (its OK wrote its own open state closed) shows again when the host opens it
/// again under the same id: the host forgets the open state of a dialog it removes.
#[trial]
fn a_dialog_that_closed_itself_shows_when_opened_again() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = running(&host);
    for round in 0..2 {
        host.borrow_mut().open(SelfClosing);
        fixture.idle(6);
        assert!(fixture.by_label("Reopen probe").is_some(), "open {round}: the dialog shows");
        let ok = fixture.by_label("OK").expect("the OK button is shown");
        fixture.click(&ok).idle(4);
        assert!(host.borrow().is_empty(), "open {round}: OK closes the dialog");
        assert_eq!(host.borrow_mut().take::<SelfClosing>(), Some(DialogResult::Ok(())), "open {round}: OK answers");
        assert!(fixture.by_label("Reopen probe").is_none(), "open {round}: the closed dialog is gone");
    }
}

/// Automation answers the recovery prompt through its `choice` field: Discard is reachable, Recover is refused while
/// the document is blocked, and a value that is no answer is refused.
#[trial]
fn automation_answers_a_recovery_prompt_with_discard() {
    let ctx = egui::Context::default();
    let mut host = DialogHost::new();
    let age = Duration::from_secs(300);

    let blocked = RecoveredDocument::new("poster", "poster.psd", age).blocked("Stop recording first");
    let id = host.open(RecoveryDialog::new(blocked));
    assert!(host.set_json_field(&ctx, id, "choice", json!("recover")).is_err(), "a blocked document refuses recover");
    assert!(host.set_json_field(&ctx, id, "choice", json!("maybe")).is_err(), "an unknown choice is refused");
    assert!(host.set_json_field(&ctx, id, "other", json!("discard")).is_err(), "an unknown field is refused");
    assert_eq!(host.set_json_field(&ctx, id, "choice", json!("discard")), Ok(()));
    assert!(host.confirm_by_id(&ctx, id));
    assert!(matches!(
        host.take::<RecoveryDialog>(),
        Some(DialogResult::Ok(RecoveryChoice { answer: RecoveryAnswer::Discard, apply_to_all: false, .. }))
    ));

    let id = host.open(RecoveryDialog::new(RecoveredDocument::new("poster", "poster.psd", age)));
    assert!(host.confirm_by_id(&ctx, id));
    assert!(
        matches!(host.take::<RecoveryDialog>(), Some(DialogResult::Ok(RecoveryChoice { answer: RecoveryAnswer::Recover, apply_to_all: false, .. }))),
        "Recover is the default choice"
    );
}

/// A job window that answered Cancel shows again when the app draws it for the next job.
#[trial]
fn a_job_window_shows_again_for_the_next_job_after_cancel() {
    let drawing = Rc::new(Cell::new(true));
    let answered = Rc::new(Cell::new(None));
    let (on, got) = (Rc::clone(&drawing), Rc::clone(&answered));
    let mut fixture = Fixture::new(move |ui| {
        if on.get()
            && let Some(answer) = JobProgress::new("Exporting", "Page 1 of 3").show(ui).answer
        {
            got.set(Some(answer));
        }
    });
    fixture.idle(6);
    assert!(fixture.by_label("Exporting").is_some(), "the window shows for the first job");
    let cancel = fixture.by_label("Cancel").expect("the window has Cancel");
    fixture.click(&cancel).idle(4);
    assert_eq!(answered.get(), Some(JobAnswer::Cancel));

    drawing.set(false);
    fixture.idle(3);
    assert!(fixture.by_label("Exporting").is_none(), "the window is gone while no job wants it");
    drawing.set(true);
    fixture.idle(6);
    assert!(fixture.by_label("Exporting").is_some(), "the window shows for the next job");
}

/// A prompt with no default answer (a job window's Cancel and Continue in Background) opens with no button focused, so
/// Enter answers nothing; Escape still answers Cancel.
#[trial]
fn a_prompt_without_a_default_answer_opens_with_no_button_focused() {
    let answered = Rc::new(Cell::new(None));
    let got = Rc::clone(&answered);
    let mut fixture = Fixture::new(move |ui| {
        if let Some(answer) = JobProgress::new("Exporting", "Page 1 of 3").can_hide(true).show(ui).answer {
            got.set(Some(answer));
        }
    });
    fixture.idle(6);
    assert!(fixture.by_label("Continue in Background").is_some(), "the window has a second answer");
    assert_eq!(fixture.focused(), None, "no button has the focus");
    fixture.key(egui::Key::Enter, Modifiers::NONE).idle(2);
    assert_eq!(answered.get(), None, "Enter answers nothing");
    fixture.key(egui::Key::Escape, Modifiers::NONE).idle(2);
    assert_eq!(answered.get(), Some(JobAnswer::Cancel));
}

/// A job window is not a dialog that asks: an app that draws it only while no such dialog is open draws it in every
/// pass, and the window still counts as a modal layer. A prompt with an answer counts as asking.
#[trial]
fn a_job_window_is_a_modal_layer_that_asks_nothing() {
    let (passes, drawn) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (all, shown) = (Rc::clone(&passes), Rc::clone(&drawn));
    let mut fixture = Fixture::new(move |ui| {
        all.set(all.get() + 1);
        if !any_open(ui.ctx(), OpenFilter::Asks) {
            JobProgress::new("Exporting", "Page 1 of 3").show(ui);
            shown.set(shown.get() + 1);
        }
    });
    fixture.idle(6);
    assert_eq!(drawn.get(), passes.get(), "the window is drawn in every pass");
    assert!(fixture.by_label("Exporting").is_some(), "the window shows");
    assert!(any_open(fixture.ctx(), OpenFilter::Modal), "the window is a modal layer");
    assert!(!any_open(fixture.ctx(), OpenFilter::Asks), "the window asks nothing");

    let mut fixture = Fixture::new(|ui| {
        Prompt::new("Replace the file?", "A file with this name exists.").answers([FooterButton::new("Replace", DialogRole::Accept)]).show(ui);
    });
    fixture.idle(6);
    assert!(any_open(fixture.ctx(), OpenFilter::Asks), "a prompt with an answer asks");
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

/// A real child dialog must be visible and accessible over its hosted parent.
#[trial]
fn a_hosted_child_dialog_is_visible() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = running(&host);
    let parent = host.borrow_mut().open(SelfClosing);
    fixture.idle(6);
    host.borrow_mut().open_child(HostedAlert::new(Alert::new("Child notice", "Child message")), parent);
    fixture.idle(6);
    assert!(fixture.by_label("Child notice").is_some(), "the child is accessible");
    assert!(fixture.texts().iter().any(|text| text.text == "Child message"), "the child is painted");
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

/// The platform's chords answer Don't Save on the unsaved-changes question and the app sees no press: Cmd+D on
/// macOS, Ctrl+D or Alt with the button's letter ("No") on Linux. macOS has no Alt chord.
#[trial]
fn the_platform_chords_answer_dont_save() {
    let cases = [
        (Os::Mac, egui::Key::D, Modifiers::COMMAND, true),
        (Os::Linux, egui::Key::D, Modifiers::COMMAND, true),
        (Os::Linux, egui::Key::N, Modifiers::ALT, true),
        (Os::Mac, egui::Key::D, Modifiers::ALT, false),
    ];
    for (os, key, mods, answers) in cases {
        let host = Rc::new(RefCell::new(DialogHost::new()));
        let app_saw_key = Rc::new(Cell::new(false));
        let (shown, saw) = (Rc::clone(&host), Rc::clone(&app_saw_key));
        let mut fixture = Fixture::new(move |ui| {
            let pressed =
                ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Key { key: k, pressed: true, .. } if *k == key)));
            saw.set(saw.get() || pressed);
            shown.borrow_mut().run(ui);
        });
        fixture.os(os);
        host.borrow_mut().open(UnsavedChangesDialog::new(CloseScope::Close, "poster.psd"));
        fixture.idle(6);
        fixture.key(key, mods).idle(4);
        let result = host.borrow_mut().take::<UnsavedChangesDialog>();
        if answers {
            assert_eq!(
                result.and_then(DialogResult::ok).map(|answer| answer.answer),
                Some(CloseAnswer::DontSave),
                "{os:?}: {mods:?}+{key:?} answers Don't Save"
            );
            assert!(!app_saw_key.get(), "{os:?}: the press reaches nothing but the question");
        } else {
            assert!(result.is_none() && host.borrow().is_open::<UnsavedChangesDialog>(), "{os:?}: {mods:?}+{key:?} leaves the question open");
        }
    }
}

/// A long document name must remain within the unsaved question's dialog.
#[trial]
fn unsaved_question_keeps_long_text_inside_its_dialog() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = running(&host);
    fixture.room(vec2(820.0, 520.0));
    host.borrow_mut()
        .open(UnsavedChangesDialog::new(CloseScope::Close, "WWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW.pdf"));
    fixture.idle(6);
    // The question is an alert dialog (upstream's role for the unsaved question).
    let dialog = fixture.by_role(Role::ALERT_DIALOG).first().expect("the question").rect;
    assert!(dialog.left() >= 0.0 && dialog.right() <= 820.0, "the question stays on screen: {dialog:?}");
    let runs = fixture.texts();
    let message = runs.iter().find(|run| run.text.contains("WWWWWWWW")).expect("the document name is painted");
    assert!(message.rect.left() >= dialog.left() && message.rect.right() <= dialog.right(), "message {:?} exceeds dialog {dialog:?}", message.rect);
}

/// The unsaved-changes buttons run Yes, No, Cancel on Windows and Don't Save, Cancel, Save on macOS; on every OS Escape
/// answers Cancel, and the Yes and No letters answer Save and Don't Save.
#[trial]
fn the_unsaved_question_orders_its_buttons_by_platform_and_answers_by_role() {
    let order = |os: Os, names: [&str; 3]| {
        let host = Rc::new(RefCell::new(DialogHost::new()));
        let shown = Rc::clone(&host);
        let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
        fixture.os(os);
        host.borrow_mut().open(UnsavedChangesDialog::new(CloseScope::Close, "poster.psd"));
        fixture.idle(6);
        let lefts = names.map(|name| fixture.by_label(name).unwrap_or_else(|| panic!("{os:?}: {name} shows")).rect.left());
        assert!(lefts.windows(2).all(|pair| pair[0] < pair[1]), "{os:?}: {names:?} run left to right, at {lefts:?}");
    };
    order(Os::Windows, ["Yes", "No", "Cancel"]);
    order(Os::Mac, ["Don't Save", "Cancel", "Save"]);

    for (key, expected) in [(egui::Key::Escape, CloseAnswer::Cancel), (egui::Key::Y, CloseAnswer::Save), (egui::Key::N, CloseAnswer::DontSave)] {
        let host = Rc::new(RefCell::new(DialogHost::new()));
        let shown = Rc::clone(&host);
        let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
        fixture.os(Os::Windows);
        host.borrow_mut().open(UnsavedChangesDialog::new(CloseScope::Close, "poster.psd"));
        fixture.idle(6);
        fixture.key(key, Modifiers::NONE).idle(4);
        let answered = host.borrow_mut().take::<UnsavedChangesDialog>().and_then(DialogResult::ok).map(|answer| answer.answer);
        assert_eq!(answered, Some(expected), "{key:?} answers {expected:?}");
    }
}

/// The unsaved-changes dialog exposes Role::ALERT_DIALOG, with its title as its accessible name and its consequence
/// message and save-failure reason in its AccessKit description.
#[trial]
fn the_unsaved_dialog_exposes_alert_dialog_role_and_folds_reason_into_description() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));

    // 1. Without reason: Role::ALERT_DIALOG, label is "Unsaved changes", description contains the closing question.
    host.borrow_mut().open(UnsavedChangesDialog::new(CloseScope::Close, "poster.psd"));
    fixture.idle(6);
    let dialogs = fixture.by_role(Role::ALERT_DIALOG);
    assert_eq!(dialogs.len(), 1, "the unsaved changes dialog exposes Role::ALERT_DIALOG");
    let dialog = dialogs[0];
    assert_eq!(fixture.tree().name_of(dialog.node), Some("Unsaved changes"), "accessible name is the dialog title");
    let desc_without_reason = fixture.tree().description_of(dialog.node).expect("dialog has an accessible description").to_owned();
    assert!(desc_without_reason.contains("poster.psd"), "description mentions the document name: {desc_without_reason}");
    assert!(desc_without_reason.contains("before closing"), "description conveys closing consequence: {desc_without_reason}");

    // Cancel to close
    assert!(host.borrow_mut().cancel::<UnsavedChangesDialog>(fixture.ctx()));
    fixture.idle(4);

    // 2. With reason: Role::ALERT_DIALOG, description contains both the closing question and the save-failure reason.
    let reason_text = "Disk full or permission denied";
    host.borrow_mut().open(UnsavedChangesDialog::new(CloseScope::Close, "poster.psd").reason(reason_text));
    fixture.idle(6);
    let dialogs = fixture.by_role(Role::ALERT_DIALOG);
    assert_eq!(dialogs.len(), 1, "reopened unsaved dialog with reason exposes Role::ALERT_DIALOG");
    let dialog = dialogs[0];
    assert_eq!(fixture.tree().name_of(dialog.node), Some("Unsaved changes"));
    let desc_with_reason = fixture.tree().description_of(dialog.node).expect("dialog has an accessible description");
    assert!(desc_with_reason.contains("poster.psd"), "description mentions document name: {desc_with_reason}");
    assert!(desc_with_reason.contains(reason_text), "description includes save failure reason: {desc_with_reason}");
    assert_eq!(desc_with_reason, format!("{desc_without_reason} {reason_text}"));
}

/// Apply, an auxiliary action, stands after Cancel on Windows; the Alternate role does not move it.
#[trial]
fn windows_puts_an_auxiliary_apply_after_cancel() {
    let mut fixture = Fixture::new(|ui| {
        let mut open = true;
        Dialog::new("Options")
            .opened(Bind::bound(&mut open))
            .buttons([
                FooterButton::new("OK", DialogRole::Accept),
                FooterButton::new("Apply", DialogRole::Other).stays_open(true),
                FooterButton::new("Cancel", DialogRole::Cancel),
            ])
            .show(ui, |_, _| {});
    });
    fixture.os(Os::Windows);
    fixture.idle(30);
    let left = |name: &str| fixture.by_label(name).expect("the button shows").rect.left();
    assert!(left("OK") < left("Cancel") && left("Cancel") < left("Apply"), "OK, Cancel, Apply from the start edge");
}

/// A destructive proceed is a committing button: Windows puts it first, before Cancel; macOS keeps Cancel nearer the
/// end edge than Delete.
#[trial]
fn a_delete_prompt_reads_delete_then_cancel_on_windows() {
    for os in [Os::Windows, Os::Mac] {
        let mut fixture = Fixture::new(|ui| {
            let mut open = true;
            Dialog::new("Delete Layer")
                .opened(Bind::bound(&mut open))
                .buttons([FooterButton::new("Cancel", DialogRole::Cancel), FooterButton::new("Delete", DialogRole::Destructive)])
                .show(ui, |_, _| {});
        });
        fixture.os(os);
        fixture.idle(30);
        let left = |name: &str| fixture.by_label(name).expect("the button shows").rect.left();
        assert!(left("Delete") < left("Cancel"), "{os:?}: Delete, then Cancel, from the start edge");
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

#[trial]
fn results_belong_to_the_opening_that_reported_them() {
    let frames = Frames::new();
    let mut host = DialogHost::new();
    let id = host.open(Probe::<1>::new("picker"));
    let first = host.open_serial(id).unwrap();
    assert_eq!(host.open(Probe::<1>::new("picker")), id);
    let second = host.open_serial(id).unwrap();
    assert_ne!(first, second);
    assert_eq!(host.take_by_open_serial::<Probe<1>>(second), None);
    assert_eq!(host.take_by_open_serial::<Probe<1>>(first), Some(DialogResult::Cancel));

    assert!(host.confirm_by_id(&frames.ctx, id));
    assert_eq!(host.take_by_open_serial::<Probe<1>>(first), None);
    assert_eq!(host.take_by_open_serial::<Probe<2>>(second), None);
    assert_eq!(host.take_by_open_serial::<Probe<1>>(second), Some(DialogResult::Ok(1)));
    assert_eq!(host.take::<Probe<1>>(), None);
}

#[trial]
fn a_new_opening_supersedes_an_unclaimed_result_at_the_same_id() {
    let frames = Frames::new();
    let mut host = DialogHost::new();
    let id = host.open(Probe::<1>::new("picker"));
    let first = host.open_serial(id).unwrap();
    assert!(host.confirm_by_id(&frames.ctx, id));
    assert_eq!(host.open(Probe::<1>::new("picker")), id);
    let second = host.open_serial(id).unwrap();
    assert_eq!(host.take_by_open_serial::<Probe<1>>(first), None);

    assert!(host.cancel_by_id(&frames.ctx, id));
    assert_eq!(host.take_by_id::<Probe<1>>(id), Some(DialogResult::Cancel));
    assert_eq!(host.take_by_open_serial::<Probe<1>>(second), None);
}

#[trial]
fn a_result_keeps_its_opening_serial_if_the_instance_changes() {
    let frames = Frames::new();
    let mut host = DialogHost::new();
    let id = host.open(Probe::<1>::new("picker"));
    let serial = host.open_serial(id).unwrap();
    host.get_mut_by_id::<Probe<1>>(id).unwrap().instance = Some(dialog_id("instance"));
    assert_eq!(host.open_serial(id), Some(serial));

    assert!(host.confirm_by_id(&frames.ctx, id));
    assert_eq!(host.take_by_open_serial::<Probe<1>>(serial), Some(DialogResult::Ok(1)));
}

#[trial]
fn open_serial_reports_the_latest_opening_when_dialog_types_share_an_id() {
    let frames = Frames::new();
    let mut host = DialogHost::new();
    let id = host.open(Probe::<1>::new("picker"));
    let first = host.open_serial(id).unwrap();
    assert_eq!(host.open(Probe::<2>::new("picker")), id);
    let second = host.open_serial(id).unwrap();
    assert_ne!(first, second);
    assert_eq!(host.open_serial_by_id::<Probe<1>>(id), Some(first));
    assert_eq!(host.open_serial_by_id::<Probe<2>>(id), Some(second));
    assert_eq!(host.open_serial_by_id::<Probe<3>>(id), None);
    assert!(host.get_by_id::<Probe<1>>(id).is_some());
    host.get_mut_by_id::<Probe<2>>(id).unwrap().shows = 5;
    assert_eq!(host.get_by_id::<Probe<2>>(id).unwrap().shows, 5);

    assert!(host.confirm::<Probe<2>>(&frames.ctx));
    assert_eq!(host.take_by_open_serial::<Probe<2>>(second), Some(DialogResult::Ok(2)));
}

#[trial]
fn a_live_opening_can_be_mutated_independently_of_the_first_dialog_with_its_id() {
    let mut host = DialogHost::new();
    let mut first = Probe::<1>::new("picker");
    first.instance = Some(dialog_id("first instance"));
    let id = host.open(first);
    let first = host.open_serial_by_id::<Probe<1>>(id).unwrap();
    let mut second = Probe::<1>::new("picker");
    second.instance = Some(dialog_id("second instance"));
    assert_eq!(host.open(second), id);
    let second = host.open_serial_by_id::<Probe<1>>(id).unwrap();
    assert_ne!(first, second);

    host.get_mut_by_open_serial::<Probe<1>>(second).unwrap().shows = 5;
    assert_eq!(host.get_by_id::<Probe<1>>(id).unwrap().shows, 0);
    assert_eq!(host.get_by_open_serial::<Probe<1>>(first).unwrap().shows, 0);
    assert_eq!(host.get_by_open_serial::<Probe<1>>(second).unwrap().shows, 5);
    assert!(host.get_mut_by_open_serial::<Probe<2>>(second).is_none());
}

/// The commands an Undo fired that a focused, unedited name field had nothing to do for, the field in a modal dialog
/// or in the window.
fn undo_with_nothing_to_undo(modal: bool) -> Vec<String> {
    let mut name = String::new();
    let mut fixture = Fixture::new(move |ui| {
        if modal {
            Dialog::new("Rename").show(ui, |ui, _| {
                TextField::new("Name", Bind::bound(&mut name)).show(ui);
            });
        } else {
            TextField::new("Name", Bind::bound(&mut name)).show(ui);
        }
    });
    fixture.settle();
    if !modal {
        let field = fixture.by_label("Name").expect("the field shows");
        fixture.click(&field).settle();
    }
    take_output(fixture.ctx());
    fixture.key(egui::Key::Z, Modifiers::COMMAND).settle();
    take_output(fixture.ctx()).fired.into_iter().map(|fired| fired.command.id).collect()
}

/// An Undo a focused text field has nothing to do for runs the window's Undo (11.3), but not past a modal dialog the
/// field is in: the modal rule holds for it as for the key itself, so the document under the dialog stays.
#[trial]
fn an_undo_a_dialogs_field_leaves_runs_no_command_under_the_dialog() {
    assert_eq!(undo_with_nothing_to_undo(false), [EditCommands::Undo.id()], "in the window, the field's leftover Undo is the app's");
    assert!(undo_with_nothing_to_undo(true).is_empty(), "under the dialog it reaches no command");
}

/// A prompt can carry an icon beside its message and a muted note under it: the message starts after the icon, the
/// note under it at the content's start.
#[trial]
fn a_prompt_shows_an_icon_beside_its_message_and_a_note_under_it() {
    let mut fixture = Fixture::new(|ui| {
        let _ = StandardPrompt::custom("Open this link?")
            .icon(craft_behavior::IconId::new("info"))
            .note("Links can lead to unsafe sites.")
            .button(craft_ui::dialog::PromptChoice::Proceed, "Open")
            .button(craft_ui::dialog::PromptChoice::Cancel, "Cancel")
            .show(ui);
    });
    fixture.settle();
    let at = |text: &str| fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect).unwrap_or(Rect::NOTHING);
    let (message, note) = (at("Open this link?"), at("Links can lead to unsafe sites."));
    assert!(message.is_positive() && note.is_positive(), "both texts are painted: {message:?} {note:?}");
    assert!(message.left() > note.left() + 8.0, "the message starts after the icon: {message:?} vs {note:?}");
    assert!(note.top() > message.bottom() - 1.0, "the note is under the message");
}

/// A document app's save question words and orders its buttons the same on every platform: Don't Save, Cancel and
/// Save last, with the app's title, question and note.
#[trial]
fn the_unsaved_question_can_ask_as_a_document_app_does() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = running(&host);
    fixture.os(Os::Linux);
    host.borrow_mut().open(
        UnsavedChangesDialog::new(CloseScope::Close, "poster.pdf")
            .document_wording(true)
            .title("Save changes to “poster.pdf” before closing?")
            .message("Your changes will be lost if you don't save them."),
    );
    fixture.idle(6);
    let at = |name: &str| fixture.by_label(name).unwrap_or_else(|| panic!("{name:?} is a button")).rect.min.x;
    assert!(at("Don't Save") < at("Cancel") && at("Cancel") < at("Save"), "Don't Save, Cancel, Save");
    assert!(fixture.texts().iter().any(|run| run.text.contains("before closing?")), "the app's title is shown");
}

/// A row that fills the dialog's width and spreads its children (a count at the start, a Remove button at the end)
/// puts them apart, the last at the row's end edge, not on top of each other.
#[trial]
fn a_space_between_row_in_a_dialog_keeps_its_children_apart_at_both_edges() {
    use craft_ui::buttons::Button;
    use craft_ui::display::Text;
    use craft_ui::layout::RowAlign;
    use craft_ui::layout_components::Row;
    let mut fixture = Fixture::new(|ui| {
        let mut open = true;
        Dialog::new("Hidden").opened(Bind::bound(&mut open)).show(ui, |ui, _| {
            Row::new().fill_width().align(RowAlign::SpaceBetween).show(ui, |ui, _| {
                Text::new("3 hidden items").show(ui);
                Button::new("Remove").show(ui);
            });
            Text::new("A longer paragraph of text that makes the dialog about as wide as production's does.").show(ui);
        });
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    let remove = fixture.by_label("Remove").expect("the button is shown").rect;
    let count = fixture.texts().iter().find(|run| run.text.contains("3 hidden")).map(|run| run.rect).expect("the count is painted");
    assert!(count.max.x < remove.min.x, "the children do not overlap: {count:?} {remove:?}");
    let dialog = fixture.by_role(craft_ui::contract::Role::DIALOG).first().map(|d| d.rect).expect("the dialog is shown");
    assert!(dialog.max.x - remove.max.x < 40.0, "Remove sits at the row's end edge: {remove:?} in {dialog:?}");
}

/// The variants of the buttons a prompt's footer shows, with the app's `DialogButtonLook` when `look` names one.
fn footer_variants(look: Option<craft_ui::sections::DialogButtonLook>) -> Vec<craft_theme::Variant> {
    let mut fixture = Fixture::new(|ui| {
        let _ = StandardPrompt::custom("Close?")
            .button(craft_ui::dialog::PromptChoice::Proceed, "Save")
            .button(craft_ui::dialog::PromptChoice::Alternate, "Don't Save")
            .button(craft_ui::dialog::PromptChoice::Cancel, "Cancel")
            .show(ui);
    });
    if let Some(look) = look {
        fixture.root_value(look);
    }
    fixture.settle();
    fixture.styles_of(std::any::type_name::<craft_ui::buttons::Button<'static>>()).iter().map(|style| style.variant).collect()
}

/// Upstream's footer by default: Cancel and the Alternate answer are outlined (secondary) buttons beside the accented
/// answer; no button is a ghost.
#[trial]
fn a_footer_cancel_is_an_outlined_button_by_default() {
    use craft_theme::Variant;
    let seen = footer_variants(None);
    assert!(seen.contains(&Variant::SECONDARY), "Cancel and Don't Save are outlined: {seen:?}");
    assert!(!seen.contains(&Variant::GHOST), "no ghost button in the footer: {seen:?}");
}

/// Production's dialogs (`DialogButtonLook::Quiet`) show Cancel and Don't Save as plain text beside the accented
/// answer: ghost buttons, not outlined ones.
#[trial]
fn a_footer_cancel_is_a_ghost_button_beside_the_accent_answer() {
    use craft_theme::Variant;
    let seen = footer_variants(Some(craft_ui::sections::DialogButtonLook::Quiet));
    assert!(seen.contains(&Variant::GHOST), "Cancel is a ghost button: {seen:?}");
    assert!(seen.contains(&Variant::PRIMARY), "Save is the accented button: {seen:?}");
    assert!(!seen.contains(&Variant::SECONDARY), "no outlined button in the footer: {seen:?}");
}

/// VectorCraft's dialogs (`DialogButtonLook::Filled`) show Cancel and Don't Save as filled neutral tiles beside the
/// accented answer, not as outlines.
#[trial]
fn a_footer_cancel_is_a_filled_button_beside_the_accent_answer() {
    use craft_theme::Variant;
    let seen = footer_variants(Some(craft_ui::sections::DialogButtonLook::Filled));
    assert!(seen.contains(&craft_ui::buttons::FILLED), "Cancel is a filled button: {seen:?}");
    assert!(seen.contains(&Variant::PRIMARY), "Save is the accented button: {seen:?}");
    assert!(!seen.contains(&Variant::SECONDARY), "no outlined button in the footer: {seen:?}");
}

/// The left edges of a Save anyway / Save as… / Cancel prompt's buttons on Linux, with the Alternate's role set to
/// `role` when it names one.
fn alternate_edges(role: Option<DialogRole>) -> (f32, f32, f32) {
    let mut fixture = Fixture::new(move |ui| {
        let prompt = StandardPrompt::custom("Changed on disk")
            .button(craft_ui::dialog::PromptChoice::Proceed, "Save anyway")
            .button(craft_ui::dialog::PromptChoice::Alternate, "Save as…")
            .button(craft_ui::dialog::PromptChoice::Cancel, "Cancel");
        let prompt = match role {
            Some(role) => prompt.alternate_role(role),
            None => prompt,
        };
        let _ = prompt.show(ui);
    });
    fixture.os(Os::Linux);
    fixture.settle();
    let left = |name: &str| fixture.by_label(name).unwrap_or_else(|| panic!("{name} button")).rect.left();
    (left("Save as…"), left("Cancel"), left("Save anyway"))
}

/// By default a prompt's Alternate answer is upstream's Alternate role: it closes the question, so it stands first off
/// Windows ("Save as…, Cancel, Save anyway").
#[trial]
fn a_prompts_alternate_answer_stands_first_by_default() {
    let (alternate, cancel, accept) = alternate_edges(None);
    assert!(alternate < cancel && cancel < accept, "Save as…, Cancel, Save anyway: {alternate} {cancel} {accept}");
}

/// With `alternate_role(DialogRole::Alternative)` (production's order) the Alternate answer is a second way to answer,
/// between Cancel and the accepting button ("Cancel, Save as…, Save anyway").
#[trial]
fn a_prompts_alternative_answer_stands_between_cancel_and_accept() {
    let (alternate, cancel, accept) = alternate_edges(Some(DialogRole::Alternative));
    assert!(cancel < alternate && alternate < accept, "Cancel, Save as…, Save anyway: {cancel} {alternate} {accept}");
}

/// A dialog's title row carries a close x; an app whose dialogs end with their buttons (production's) turns it off
/// once with `dialog::set_close_button`, and no dialog of the app shows it.
#[trial]
fn an_app_can_take_the_close_x_off_every_dialog() {
    let off = Rc::new(Cell::new(false));
    let flag = Rc::clone(&off);
    let mut fixture = Fixture::new(move |ui| {
        craft_ui::dialog::set_close_button(ui.ctx(), !flag.get());
        let _ = Dialog::new("Plain").buttons([FooterButton::new("OK", DialogRole::Accept)]).show(ui, |_, _| {});
    });
    fixture.idle(4);
    assert!(fixture.by_label("Close").is_some(), "the close x is there by default");
    off.set(true);
    fixture.idle(4);
    assert!(fixture.by_label("Close").is_none(), "the close x is gone once the app turned it off");
    assert!(fixture.by_label("OK").is_some(), "the dialog still has its buttons");
}

/// The title of the hosted modal the nudge and popup trials open.
const NUDGED: &str = "Nudge probe";

/// The body text of that modal.
const NUDGED_BODY: &str = "Press beside this dialog.";

/// A hosted modal with a line of text and OK; while `above` is on, a popover opened from its body shows above it.
struct Nudged {
    above: Arc<AtomicBool>,
}

impl Nudged {
    fn new() -> Self {
        Nudged { above: Arc::new(AtomicBool::new(false)) }
    }
}

impl StackDialog for Nudged {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("nudged")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let above = Arc::clone(&self.above);
        let shown = Dialog::new(NUDGED).buttons([FooterButton::new("OK", DialogRole::Accept)]).show(ui, |ui, _| {
            Text::new(NUDGED_BODY).show(ui);
            let mut open = above.load(Ordering::Relaxed);
            kept_popover(ui, "Above options", &mut open, ABOVE);
            above.store(open, Ordering::Relaxed);
        });
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }
}

/// The text inside the popover opened above the modal.
const ABOVE: &str = "Above the dialog";

/// The text inside the popover open beneath the modal.
const BENEATH: &str = "Beneath the dialog";

/// A popover bound to `open` that no press outside, focus move or Escape closes: only a close the runtime queues does.
fn kept_popover(ui: &mut Ui, label: &'static str, open: &mut bool, text: &'static str) {
    Popover::new(label, Bind::bound(open)).dismiss(Dismiss::new().outside(Outside::Keep)).escape(false).show(ui, |ui, _| {
        Text::new(text).show(ui);
    });
}

/// The rect of the open dialog in the latest frame; `None` while none shows.
fn dialog_rect(fixture: &Fixture) -> Option<Rect> {
    fixture.by_role(Role::DIALOG).first().map(|dialog| dialog.rect)
}

/// A pointer move to `at`, a press there and its release, each in a frame of its own; `during` runs after each frame.
fn press_at(fixture: &mut Fixture, at: Pos2, mut during: impl FnMut(&Fixture)) {
    for event in [pointer::moved(at), pointer::press(at), pointer::release(at)] {
        fixture.frame(vec![event]);
        during(fixture);
    }
}

/// The stroked rects the latest frame painted, with their strokes.
fn strokes(shapes: &[egui::Shape]) -> Vec<(Rect, egui::Stroke)> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Rect(rect) if rect.stroke.width > 0.0 => Some((rect.rect, rect.stroke)),
            _ => None,
        })
        .collect()
}

/// A hosted modal under full motion: a press inside does nothing; a press beside shakes it, also when the press frame
/// runs a second pass; a second press while it shakes does not start it over, so it is at rest by the end of the first
/// shake; and a dialog closed while it shakes opens again at rest. Under reduced motion the press flashes its edge and
/// the dialog never moves.
#[trial]
fn a_press_beside_a_modal_dialog_nudges_it_once_and_it_comes_to_rest() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let rerun = Rc::new(Cell::new(false));
    let (shown, again) = (Rc::clone(&host), Rc::clone(&rerun));
    let mut fixture = Fixture::new(move |ui| {
        shown.borrow_mut().run(ui);
        if again.get() && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the press frame runs twice");
        }
    });
    fixture.motion(MotionMode::Full);
    host.borrow_mut().open(Nudged::new());
    fixture.settle().idle(12);
    let rest = dialog_rect(&fixture).expect("the modal shows");
    let beside = Pos2::new(4.0, 4.0);
    assert!(!rest.contains(beside), "the press point lies beside the dialog");

    // On the dialog's own padding, beside its text and buttons.
    let inside = Pos2::new(rest.left() + 2.0, rest.center().y);
    let mut moved = false;
    press_at(&mut fixture, inside, |fixture| moved |= dialog_rect(fixture) != Some(rest));
    for _ in 0..12 {
        fixture.frame(Vec::new());
        moved |= dialog_rect(&fixture) != Some(rest);
    }
    assert!(!moved, "a press inside the dialog does not nudge it");
    assert!(host.borrow().is_open::<Nudged>(), "the press inside left the dialog open");

    // The nudge starts in the press frame, which runs two passes; the press reaches only the first.
    fixture.frame(vec![pointer::moved(beside)]);
    rerun.set(true);
    fixture.frame(vec![pointer::press(beside)]);
    rerun.set(false);
    assert!(fixture.passes() > 1, "the press frame ran a second pass");
    fixture.frame(vec![pointer::release(beside)]);
    moved |= dialog_rect(&fixture) != Some(rest);
    // Frames two to four after the first press: a second press while the shake runs.
    press_at(&mut fixture, beside, |fixture| moved |= dialog_rect(fixture) != Some(rest));
    // Frames five to ten: the open motion's duration is nine frames, so the first shake is over by the tenth.
    for _ in 0..6 {
        fixture.frame(Vec::new());
        moved |= dialog_rect(&fixture) != Some(rest);
    }
    assert!(moved, "a press beside the dialog moves it");
    assert_eq!(dialog_rect(&fixture), Some(rest), "the dialog is back at rest when the first nudge ends");
    assert!(!fixture.repaint_requested(), "the second press did not start the nudge over");
    assert!(host.borrow().is_open::<Nudged>(), "the presses beside left the dialog open");

    // A dialog closed in the middle of a nudge opens again as one that never was nudged.
    let reopened = |fixture: &mut Fixture, nudge: bool| -> Vec<Option<Rect>> {
        if nudge {
            press_at(fixture, beside, |_| {});
        } else {
            fixture.idle(3);
        }
        assert!(host.borrow_mut().cancel::<Nudged>(fixture.ctx()), "the open dialog cancels");
        fixture.frame(Vec::new());
        host.borrow_mut().open(Nudged::new());
        (0..10).map(|_| dialog_rect(fixture.frame(Vec::new()))).collect()
    };
    let calm = reopened(&mut fixture, false);
    fixture.settle().idle(12);
    let after_nudge = reopened(&mut fixture, true);
    assert_eq!(after_nudge, calm, "a reopened dialog does not carry the nudge it closed in");

    // Reduced motion: two edge flashes over the dialog, which stays where it is.
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.motion(MotionMode::Reduced);
    host.borrow_mut().open(Nudged::new());
    fixture.settle().idle(12);
    let rest = dialog_rect(&fixture).expect("the modal shows");
    let calm = strokes(&fixture.frame_shapes());
    let mut moved = false;
    press_at(&mut fixture, beside, |fixture| moved |= dialog_rect(fixture) != Some(rest));
    let mut flashed = false;
    for _ in 0..10 {
        let painted = strokes(&fixture.frame_shapes());
        flashed |= painted.iter().any(|stroke| !calm.contains(stroke) && stroke.0.contains(rest.center()));
        moved |= dialog_rect(&fixture) != Some(rest);
    }
    assert!(flashed, "under reduced motion a press beside flashes the dialog's edge");
    assert!(!moved, "under reduced motion the dialog does not move");
    fixture.idle(4);
    let after = strokes(&fixture.frame_shapes());
    assert!(after.iter().all(|stroke| calm.contains(stroke)), "the flash is over once the nudge ends");
}

/// A popup open beneath a modal closes in the pass the modal first shows; a popup opened above the modal afterwards
/// stays open.
#[trial]
fn a_new_modal_closes_the_popups_beneath_it() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let beneath = Rc::new(Cell::new(true));
    let (shown, below) = (Rc::clone(&host), Rc::clone(&beneath));
    let mut fixture = Fixture::new(move |ui| {
        let mut open = below.get();
        kept_popover(ui, "Beneath options", &mut open, BENEATH);
        below.set(open);
        shown.borrow_mut().run(ui);
    });
    fixture.settle();
    assert!(beneath.get() && fixture.by_label(BENEATH).is_some(), "the popup beneath is open before the modal");

    let dialog = Nudged::new();
    let above = Arc::clone(&dialog.above);
    host.borrow_mut().open(dialog);
    // The modal's first pass queues the close; the popup's owner takes it in the pass after.
    fixture.frame(Vec::new());
    assert!(fixture.by_label(NUDGED_BODY).is_some(), "the modal shows in its first pass");
    fixture.idle(2);
    assert!(!beneath.get(), "the popup beneath the new modal closed");
    fixture.settle();
    assert!(fixture.by_label(BENEATH).is_none(), "the closed popup is gone from the screen");

    above.store(true, Ordering::Relaxed);
    fixture.settle().idle(6);
    assert!(above.load(Ordering::Relaxed), "a popup opened above the shown modal stays open");
    assert!(fixture.by_label(ABOVE).is_some(), "the popup above the modal shows");
}

/// A dialog whose modality the test sets, as current state.
struct Tool {
    modeless: bool,
}

impl StackDialog for Tool {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("tool")
    }

    fn modeless(&self) -> bool {
        self.modeless
    }

    fn show(&mut self, _: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        None
    }
}

/// `shows_modal` reads the open dialogs' modality as it is now: an empty host shows none; a modeless dialog alone is
/// not modal; a modal opened in this pass counts at once, with no pass run; a startup modal still queued does not;
/// closing the modal returns it to false; a dialog that turns modal while open counts.
#[trial]
fn shows_modal_follows_the_open_dialogs_modality() {
    let frames = Frames::new();
    let mut host = DialogHost::new();
    assert!(!host.shows_modal(), "an empty host shows no modal");
    host.open(Tool { modeless: true });
    assert!(!host.shows_modal(), "a modeless dialog alone is not modal");
    host.enqueue(StartupStage::Warning, Probe::<3>::new("queued"));
    assert!(!host.shows_modal(), "a startup modal still in the queue is not shown");
    host.open(Probe::<1>::new("modal"));
    assert!(host.shows_modal(), "a modal opened this pass counts before any pass runs");
    assert!(host.cancel::<Probe<1>>(&frames.ctx));
    assert!(!host.shows_modal(), "with the modal closed only the modeless dialog is open");
    host.get_mut::<Tool>().expect("the tool is open").modeless = false;
    assert!(host.shows_modal(), "the modality is the dialog's current state");
}

/// A dialog with a text field and a number field before any other control, holding `name` and `radius`.
fn first_field_dialog(name: Rc<RefCell<String>>, radius: Rc<Cell<Option<f64>>>, number_first: bool) -> Fixture {
    use craft_ui::text::{NumberField, TextField};
    Fixture::new(move |ui| {
        let mut open = true;
        Dialog::new("New Layer")
            .opened(Bind::bound(&mut open))
            .buttons([FooterButton::new("OK", DialogRole::Accept), FooterButton::new("Cancel", DialogRole::Cancel)])
            .show(ui, |ui, _| {
                let mut text = name.borrow().clone();
                let mut value = radius.get();
                if number_first {
                    NumberField::new("Radius", Bind::bound(&mut value)).show(ui);
                    TextField::new("Name", Bind::bound(&mut text)).show(ui);
                } else {
                    TextField::new("Name", Bind::bound(&mut text)).show(ui);
                    NumberField::new("Radius", Bind::bound(&mut value)).show(ui);
                }
                *name.borrow_mut() = text;
                radius.set(value);
            });
    })
}

/// photocraft#15540: a dialog that opens with its first field focused selects that field's whole text, so typing
/// right away replaces the value, for a text field (New Layer's name) and a number field (Gaussian Blur's Radius).
#[trial]
fn typing_in_a_freshly_opened_dialog_replaces_its_first_field() {
    let mut typed_into = Vec::new();
    for number_first in [false, true] {
        let name = Rc::new(RefCell::new("Layer 1".to_owned()));
        let radius = Rc::new(Cell::new(Some(1.0)));
        let mut fixture = first_field_dialog(name.clone(), radius.clone(), number_first);
        fixture.settle();
        assert!(fixture.focused().is_some(), "the first field has the focus (number first: {number_first})");
        fixture.type_text("7");
        fixture.idle(2);
        typed_into.push(fixture.focused().and_then(|id| fixture.tree().value(id).map(str::to_owned)));
    }
    // Typing replaced the first field's text: the text field first, then the number field first.
    assert_eq!(typed_into, [Some("7".to_owned()), Some("7".to_owned())]);
}

/// A prompt's readers give the title, message, paragraphs and buttons it shows under the same host, without a `Ui`.
#[trial]
fn a_prompts_readers_give_what_it_shows() {
    let host = Arc::new(craft_host::Host::builder(craft_host::AppInfo { id: "trial", name: "Trial", version: "0" }).build());
    let dialogs = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&dialogs);
    let mut fixture = Fixture::new(move |ui| {
        shown.borrow_mut().run(ui);
    });
    fixture.host(Arc::clone(&host));
    let prompt = StandardPrompt::confirm_delete(["Layer 1"], 1).paragraph("It can't be undone.");
    let (title, message, paragraphs, buttons) =
        (prompt.shown_title(&host), prompt.shown_message(&host), prompt.shown_paragraphs(&host), prompt.shown_buttons(&host));
    dialogs.borrow_mut().open(prompt);
    fixture.idle(6);
    assert!(message.contains("Layer 1"), "the message names what it deletes: {message}");
    let painted: Vec<String> = fixture.texts().iter().map(|text| text.text.clone()).collect();
    for text in std::iter::once(&title).chain([&message]).chain(&paragraphs) {
        assert!(painted.contains(text), "{text:?} is shown: {painted:?}");
    }
    assert_eq!(buttons.iter().map(|(choice, _)| *choice).collect::<Vec<_>>(), [PromptChoice::Proceed, PromptChoice::Cancel]);
    for (_, label) in &buttons {
        assert!(fixture.by_label(label).is_some(), "the {label:?} button is shown");
    }
}

/// A question with an OK armed after half a second, shown under the same ids whatever it asks about: an install
/// approval asking about module `key`.
struct Approval {
    key: &'static str,
}

impl StackDialog for Approval {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id(self.key)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let shown = Dialog::new("Install")
            .id_salt("approval")
            .buttons([FooterButton::new("OK", DialogRole::Accept).armed_after(0.5), FooterButton::new("Cancel", DialogRole::Cancel)])
            .show(ui, |_, _| {});
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }
}

/// Tb-8 (6): a question the host puts in place of another between two frames waits its own delay before its armed OK
/// takes a click, though its footer has the same ids: the footer counted on from the question before.
#[trial]
fn a_new_question_in_place_of_another_waits_again_for_its_armed_answer() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = running(&host);
    let first = host.borrow_mut().open(Approval { key: "a" });
    fixture.frame(vec![egui::Event::WindowFocused(true)]);
    fixture.idle(60);
    let ctx = fixture.ctx().clone();
    assert!(host.borrow_mut().cancel_by_id(&ctx, first), "the first question goes");
    host.borrow_mut().open(Approval { key: "b" });
    let click_ok = |fixture: &mut Fixture| {
        let ok = fixture.by_label("OK").expect("the OK button is shown");
        fixture.click(&ok).idle(2);
    };
    click_ok(&mut fixture);
    assert!(host.borrow().is_open::<Approval>(), "a click on the new question's OK before its delay answers nothing");
    fixture.idle(60);
    click_ok(&mut fixture);
    assert!(host.borrow().is_empty(), "after its delay the OK answers it");
}

/// A dialog is padded `space.pad.dialog` and its footer buttons stand `size.dialog.button_height` high (VectorCraft's
/// 22-point margin and 28-point buttons); an app that sets neither keeps a panel's padding and a push button's height.
#[trial]
fn a_dialog_takes_its_padding_and_footer_button_height_from_the_dialog_sizes() {
    let shown = |overrides: &[(&str, f32)]| {
        let mut sizes = craft_theme::AppSizes::default();
        for (path, value) in overrides {
            sizes.all.insert((*path).to_owned(), craft_theme::SizeValue::Number(*value));
        }
        let mut root = craft_ui::runtime::env::RootEnv::default();
        root.sizes = Arc::new(sizes);
        let mut fixture = Fixture::new(|ui| {
            let mut open = true;
            Dialog::new("Options").opened(Bind::bound(&mut open)).buttons([FooterButton::new("OK", DialogRole::Accept)]).show(ui, |ui, _| {
                Text::new("Body text").show(ui);
            });
        });
        fixture.root(root).room(vec2(1200.0, 800.0)).settle();
        let dialog = fixture.by_role(Role::DIALOG).first().expect("the dialog shows").rect;
        let text = fixture.texts().iter().find(|run| run.text == "Body text").expect("the body is painted").rect;
        (text.left() - dialog.left(), fixture.by_label("OK").expect("the footer's OK").rect.height())
    };
    let standard = shown(&[]);
    let dialog = shown(&[("space.pad.dialog", 30.0), ("size.dialog.button_height", 40.0)]);
    assert!(dialog.0 > standard.0 + 10.0, "a larger dialog padding moves the body in from the edge: {standard:?} then {dialog:?}");
    assert!(dialog.1 > standard.1 + 10.0, "a taller dialog button raises OK: {standard:?} then {dialog:?}");
    let follows = shown(&[("space.pad.regular", 30.0), ("size.button", 40.0)]);
    assert!(
        (follows.0 - dialog.0).abs() < 0.5 && (follows.1 - dialog.1).abs() < 0.5,
        "both follow the panel padding and the push button: {follows:?} and {dialog:?}"
    );
}
