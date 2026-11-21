//! A toolbar, an options bar and a status bar are memoized by the host: the host writes a view over the members it
//! passes into a `Bar` and shows it with `MemoView`. These trials use views written that way (the code a host copies):
//! idle frames replay them, a change to what one holds runs only that one, and an event leaves through an `Outbox`.

use craft_behavior::IconId;
use craft_ui::bars::{Bar, BarHome, BarItem, BarItems, BarKind, StatusMessage, StatusProgress};
use craft_ui::buttons::IconButton;
use craft_ui::contract::{Bind, CommandSet, MemoProps, MemoView, Outbox, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout_components::Stack;
use craft_ui::runtime::commands::CommandScope;
use craft_ui::runtime::env::Label;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Fixture, Frames, MemoRun, last_frame_memo_runs, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The tool commands of the toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "tool", scope = "trial.tools")]
#[non_exhaustive]
pub enum ToolCommand {
    #[command(label = "Pick")]
    Pick,
}

const TOOLS: CommandScope = <ToolCommand as CommandSet>::SCOPE;

#[derive(Debug, PartialEq)]
enum ToolEvent {
    Picked,
}

/// The toolbar's props: what its view reads. The events go out through the outbox.
#[derive(MemoProps)]
struct ToolbarProps<'a> {
    items: &'a [BarItem],
    current: u8,
    /// The text of the strip's caption: a change in its length resizes the toolbar.
    width: &'a str,
    sent: Outbox<'a, ToolEvent>,
}

/// The toolbar: a `Bar` of tools whose members are the strip and the current tool's name.
fn toolbar(props: &mut ToolbarProps<'_>, ui: &mut Ui) {
    Stack::new().command_scope(TOOLS).show(ui, |ui, cx| {
        use craft_ui::runtime::commands::Commands as _;
        while cx.take_command::<ToolCommand>().is_some() {
            props.sent.send(ToolEvent::Picked);
        }
        Bar::new("Tools", BarKind::Tool).show(ui, |ui, _| {
            BarItems::new(props.items.to_vec()).show(ui);
            // The current tool: a face whose selected look changes, not its size.
            IconButton::new(IconId::new("more")).label("Current").selected(props.current == 1).show(ui);
            Text::new(format!("Width {}", props.width)).show(ui);
        });
    });
}

#[derive(Debug, PartialEq)]
enum StatusEvent {
    Cancel,
}

#[derive(MemoProps)]
struct StatusProps<'a> {
    message: &'a StatusMessage,
    task: Label,
    fraction: Option<f32>,
    sent: Outbox<'a, StatusEvent>,
}

/// The status bar: the message, then the task's progress with a cancel button.
fn status(props: &mut StatusProps<'_>, ui: &mut Ui) {
    Bar::new("Status", BarKind::Status).show(ui, |ui, _| {
        props.message.show(ui);
        let progress = StatusProgress::new(props.task.clone(), props.fraction).cancel("Cancel").show(ui);
        if progress.cancelled {
            props.sent.send(StatusEvent::Cancel);
        }
    });
}

struct Scene {
    items: Vec<BarItem>,
    current: u8,
    width: String,
    message: StatusMessage,
    fraction: Option<f32>,
    picked: usize,
    cancelled: usize,
}

const TOOLBAR_AT: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(410.0, 50.0) };
const STATUS_AT: Rect = Rect { min: Pos2::new(10.0, 70.0), max: Pos2::new(410.0, 110.0) };

fn scene() -> (Fixture, Rc<RefCell<Scene>>) {
    let state = Rc::new(RefCell::new(Scene {
        items: vec![BarItem::new(ToolCommand::Pick).display(craft_ui::bars::ItemDisplay::Text)],
        current: 0,
        width: "short".to_owned(),
        message: StatusMessage::new("Ready"),
        fraction: Some(0.25),
        picked: 0,
        cancelled: 0,
    }));
    let shared = Rc::clone(&state);
    let mut fixture = Fixture::new(move |ui| {
        let mut scene = shared.borrow_mut();
        let (mut tool_events, mut status_events) = (Vec::new(), Vec::new());
        {
            let Scene { items, current, width, message, fraction, .. } = &*scene;
            in_region(ui, TOOLBAR_AT, |ui| {
                MemoView::new("toolbar", ToolbarProps { items, current: *current, width, sent: Outbox::new(&mut tool_events) }, toolbar).show(ui)
            });
            in_region(ui, STATUS_AT, |ui| {
                MemoView::new(
                    "status",
                    StatusProps { message, task: Label::from("Export"), fraction: *fraction, sent: Outbox::new(&mut status_events) },
                    status,
                )
                .show(ui)
            });
        }
        scene.picked += tool_events.len();
        scene.cancelled += status_events.len();
    });
    fixture.room(Vec2::new(440.0, 140.0)).settle();
    fixture.settle();
    (fixture, state)
}

/// How many times the views whose props type is named `name` ran, in the last frame.
fn ran(fixture: &Fixture, name: &str) -> usize {
    last_frame_memo_runs(fixture.ctx()).iter().filter(|(of, _, run)| of.contains(name) && *run == MemoRun::Ran).count()
}

fn replayed(fixture: &Fixture, name: &str) -> usize {
    last_frame_memo_runs(fixture.ctx()).iter().filter(|(of, _, run)| of.contains(name) && *run == MemoRun::Replayed).count()
}

#[trial]
fn an_idle_toolbar_and_status_bar_run_nothing() {
    let (mut fixture, _) = scene();
    for _ in 0..3 {
        fixture.frame(vec![]);
        assert_eq!((ran(&fixture, "ToolbarProps"), ran(&fixture, "StatusProps")), (0, 0), "an idle frame runs neither bar");
        assert_eq!((replayed(&fixture, "ToolbarProps"), replayed(&fixture, "StatusProps")), (1, 1));
    }
}

#[trial]
fn a_tool_change_runs_only_the_toolbar() {
    let (mut fixture, state) = scene();
    state.borrow_mut().current = 1;
    fixture.frame(vec![]);
    assert_eq!(
        (ran(&fixture, "ToolbarProps"), ran(&fixture, "StatusProps")),
        (1, 0),
        "the tool change runs the toolbar once: {:?}",
        last_frame_memo_runs(fixture.ctx())
    );
    // The selected look fades in, which keeps the view running until it has settled.
    fixture.settle();
    for _ in 0..3 {
        fixture.frame(vec![]);
    }
    assert_eq!(ran(&fixture, "ToolbarProps"), 0, "then it replays");
}

/// A change that resizes the toolbar asks for a discard pass, and the view that asked runs again in it: once per pass.
#[trial]
fn a_change_that_resizes_the_toolbar_runs_it_in_the_discard_pass_too() {
    let (mut fixture, state) = scene();
    state.borrow_mut().width = "a much longer caption".to_owned();
    fixture.frame(vec![]);
    assert_eq!((ran(&fixture, "ToolbarProps"), ran(&fixture, "StatusProps")), (2, 0), "{:?}", last_frame_memo_runs(fixture.ctx()));
    fixture.frame(vec![]);
    assert_eq!(ran(&fixture, "ToolbarProps"), 0, "then it replays");
}

#[trial]
fn a_status_progress_change_runs_only_the_status_bar() {
    let (mut fixture, state) = scene();
    state.borrow_mut().fraction = Some(0.5);
    fixture.frame(vec![]);
    assert_eq!((ran(&fixture, "ToolbarProps"), ran(&fixture, "StatusProps")), (0, 1), "the progress change runs only the status bar");
    state.borrow_mut().message = StatusMessage::new("Saved");
    fixture.frame(vec![]);
    assert_eq!((ran(&fixture, "ToolbarProps"), ran(&fixture, "StatusProps")), (0, 1), "so does a new message");
}

/// A click on an item of a replayed bar delivers its command once: the pointer engages the view, which runs and sends.
#[trial]
fn a_click_on_a_toolbar_item_delivers_its_event_once() {
    let (mut fixture, state) = scene();
    let item = fixture.by_label("Pick").expect("the item is shown");
    fixture.click(&item);
    fixture.settle();
    assert_eq!(state.borrow().picked, 1, "the click delivered the command once");
    fixture.settle();
    assert_eq!(state.borrow().picked, 1, "and the replays after it send nothing");
}

#[trial]
fn a_click_on_the_cancel_button_delivers_its_event_once() {
    let (mut fixture, state) = scene();
    let cancel = fixture.by_label("Cancel").expect("the cancel button is shown");
    fixture.click(&cancel);
    fixture.settle();
    assert_eq!(state.borrow().cancelled, 1, "the click delivered the cancel once");
    fixture.settle();
    assert_eq!(state.borrow().cancelled, 1);
}

/// The lead of an options bar as a member: Home and the vertical rule after it, which fills the bar's line.
#[derive(MemoProps)]
struct LeadProps {
    on: bool,
}

fn lead(props: &mut LeadProps, ui: &mut Ui) -> Rect {
    BarHome::new("Home", Bind::given(props.on)).show(ui);
    ui.min_rect()
}

/// A bar offered far more height than its members need.
const TALL_ROOM: Vec2 = Vec2::new(400.0, 300.0);

/// A memoized member takes what its content used: its rule that fills the line is as long as without the view, so the
/// bar with the member as a memoized view is as high as the bar with the same content shown plainly, both far below
/// the room they are offered.
#[trial]
fn a_memoized_member_takes_the_height_its_content_used_not_the_bars_room() {
    let (plain, memo, member) = (Cell::new(Rect::NOTHING), Cell::new(Rect::NOTHING), Cell::new(Rect::NOTHING));
    Frames::new().idle(4, |ui| {
        let at = |x: f32| Rect::from_min_size(Pos2::new(x, 10.0), TALL_ROOM);
        plain.set(in_region(ui, at(10.0), |ui| {
            Bar::new("Plain", BarKind::Options).show(ui, |ui, _| BarHome::new("Home", Bind::given(false)).show(ui)).output.response.rect
        }));
        memo.set(in_region(ui, at(420.0), |ui| {
            let shown = Bar::new("Memoized", BarKind::Options).show(ui, |ui, _| MemoView::new("lead", LeadProps { on: false }, lead).show(ui));
            if let Some(Some(rect)) = shown.inner {
                member.set(rect);
            }
            shown.output.response.rect
        }));
    });
    let (plain, memo, member) = (plain.get(), memo.get(), member.get());
    assert!(plain.height() < TALL_ROOM.y / 2.0, "the plain bar sizes from its members: {plain:?}");
    assert!(member.height() < TALL_ROOM.y / 2.0, "the member takes what its content used, not the bar's room: {member:?}");
    assert!((memo.height() - plain.height()).abs() <= 0.5, "the bar is as high with the member memoized: {memo:?} against {plain:?}");
}
