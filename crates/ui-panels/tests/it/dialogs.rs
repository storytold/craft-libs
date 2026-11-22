//! The dialog host and stack.

use craft_behavior::{Metrics, PartKey, Sense};
use craft_host::target::{Os, WindowChrome};
use craft_quit::{CloseAnswer, CloseScope};
use craft_ui::contract::Bind;
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PartsCx, Props as _, Role, Show as _, ShowContainer as _};
use craft_ui::dialog::stories::hosted;
use craft_ui::dialog::{
    Alert, Dialog, DialogHost, DialogId, DialogResult, DialogStack, HostedAlert, PromptKind, StackDialog, StandardPrompt, StartupStage,
    UnsavedChangesDialog, dialog_id,
};
use craft_ui::runtime::commands::{CommandSet as _, EditCommands};
use craft_ui::runtime::frame::take_output;
use craft_ui::runtime::windows::{WindowHit, WindowHost};
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::testing::{Fixture, Frames, Story, StoryData, StoryOutput, trial};
use craft_ui::text::TextField;
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
    let dialog = fixture.by_role(Role::DIALOG).first().expect("the question").rect;
    assert!(dialog.left() >= 0.0 && dialog.right() <= 820.0, "the question stays on screen: {dialog:?}");
    let runs = fixture.texts();
    let message = runs.iter().find(|run| run.text.contains("WWWWWWWW")).expect("the document name is painted");
    assert!(message.rect.left() >= dialog.left() && message.rect.right() <= dialog.right(), "message {:?} exceeds dialog {dialog:?}", message.rect);
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
