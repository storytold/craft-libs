//! Settled memoized views: a settled view adds its last run to the pass in place of running, and runs when one of the
//! settled rules breaks: what pointer sensing gives it, a pointer move it follows, a bound value, inspection record or command state it read, its room, its
//! environment (theme, density, language), an animation or a repaint asked inside it. Input elsewhere in the window
//! does not run it.

use crate::values::kit::{AT, INITIAL, Scrub, Seen};
use craft_behavior::MotionMode;
use craft_theme::{Contrast, Mode, Size};
use craft_ui::buttons::Button;
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{
    Bind, CommandSet, MemoProps, MemoView, Props as _, Show as _, ShowCollection as _, ShowContainer as _, Sizing, Tracked, bind_tracked,
};
use craft_ui::display::Spinner;
use craft_ui::layout_components::Stack;
use craft_ui::list::{List, ListRow};
use craft_ui::runtime::commands::{CommandInput, CommandScope, CommandState, CommandStates, Commands as _};
use craft_ui::runtime::env::RootEnv;
use craft_ui::runtime::{frame, inspect};
use craft_ui::testing::layout::in_region;
use craft_ui::testing::pointer::{moved, press, release, wheel};
use craft_ui::testing::{Fixture, Frames, MemoRun, last_frame_memo_runs, last_pass_measuring_runs, trial};
use egui::{Context, Id, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

const ORIGIN: Pos2 = Pos2::new(10.0, 60.0);
const ROOM: Vec2 = Vec2::new(300.0, 200.0);
/// Where the bound scrub of the reader view is shown, inside that view's room.
const READER: Rect = Rect { min: Pos2::new(150.0, 70.0), max: Pos2::new(250.0, 90.0) };
/// Settled frames a trial looks at.
const SETTLED: usize = 4;

thread_local! {
    /// What the reader view's scrub recorded when it last ran.
    static SEEN: Seen = Seen::default();
    /// The id and rect of the button each view of `Scene` showed when it last ran.
    static BUTTONS: [Cell<(Id, Rect)>; 2] = const { [Cell::new((Id::NULL, Rect::NOTHING)), Cell::new((Id::NULL, Rect::NOTHING))] };
    /// The rect of each row of the rows view, by key, as its last run built it.
    static ROW_RECTS: RefCell<Vec<Rect>> = const { RefCell::new(Vec::new()) };
    /// The buttons of `Shown`, `Beside`, `Published` and `Measured`, as the command scene's views last showed them.
    static COMMAND_BUTTONS: [Cell<Id>; 4] = const { [Cell::new(Id::NULL), Cell::new(Id::NULL), Cell::new(Id::NULL), Cell::new(Id::NULL)] };
}

/// A props type's name without its path and generics: `Reader` for `it::body::memo_settled::Reader<'_>`.
fn short_name(name: &'static str) -> &'static str {
    let name = name.split('<').next().unwrap_or(name);
    name.rsplit("::").next().unwrap_or(name)
}

/// The calls of every pass of the last frame in which the views whose props type is `name` ran, measuring runs
/// included.
fn runs_of(ctx: &Context, name: &str) -> usize {
    last_frame_memo_runs(ctx).iter().filter(|(of, _, run)| short_name(of) == name && *run == MemoRun::Ran).count()
}

/// The reader view: a button and a scrub bound to the value.
#[derive(MemoProps)]
struct Reader<'a> {
    value: Bind<'a, f32>,
}

fn reader(props: &mut Reader<'_>, ui: &mut Ui) {
    let response = Button::new("Alpha").show(ui).response;
    BUTTONS.with(|buttons| buttons[0].set((response.id, response.rect)));
    SEEN.with(|seen| Scrub::new(props.value.reborrowed()).id_salt("reader").seen(seen).show_at(ui, READER));
}

/// The other view: a button in a body, a spinner when `spinner` holds, and a repaint request in each run while
/// `repaints` holds.
#[derive(MemoProps)]
struct Other {
    repaints: bool,
    spinner: bool,
}

fn other(props: &mut Other, ui: &mut Ui) {
    if props.repaints {
        frame::request_repaint(ui.ctx());
    }
    // In a body, whose measure a run that is not settled takes from what its last run kept.
    Stack::new().show(ui, |ui, _| {
        let response = Button::new("Beta").show(ui).response;
        BUTTONS.with(|buttons| buttons[1].set((response.id, response.rect)));
    });
    if props.spinner {
        Spinner::new().show(ui);
    }
}

/// Two memoized views, `reader` and `other`, and a scrub outside them bound to the value the reader's own scrub shows:
/// state two parts show and edit, so it is `Tracked` and both bind it by its source.
/// The scrub that edits comes before the views in the pass when `editor_first`, else after them.
struct Scene {
    value: RefCell<Tracked<f32>>,
    room: Cell<Vec2>,
    editor_first: bool,
    spinner: bool,
    repaints: Cell<bool>,
}

impl Scene {
    fn new(editor_first: bool, spinner: bool) -> Scene {
        Scene { value: RefCell::new(Tracked::new(INITIAL)), room: Cell::new(ROOM), editor_first, spinner, repaints: Cell::new(false) }
    }

    fn show(&self, ui: &mut Ui) {
        if self.editor_first {
            self.editor(ui);
        }
        in_region(ui, Rect::from_min_size(ORIGIN, self.room.get()), |ui| {
            let mut value = self.value.borrow_mut();
            MemoView::new("reader", Reader { value: bind_tracked!(value) }, reader).show(ui);
            drop(value);
            MemoView::new("other", Other { repaints: self.repaints.get(), spinner: self.spinner }, other).show(ui);
        });
        if !self.editor_first {
            self.editor(ui);
        }
    }

    fn editor(&self, ui: &mut Ui) {
        let mut value = self.value.borrow_mut();
        Scrub::new(bind_tracked!(value)).id_salt("editor").show_at(ui, AT);
    }

    /// The runs of each view in one frame with `events`.
    fn frame(&self, frames: &mut Frames, events: Vec<egui::Event>) -> [usize; 2] {
        frames.frame(events, |ui| self.show(ui));
        [runs_of(&frames.ctx, "Reader"), runs_of(&frames.ctx, "Other")]
    }

    /// Frames with no input until both views are settled.
    fn settle(&self, frames: &mut Frames) {
        frames.idle(30, |ui| self.show(ui));
    }

    /// The id and rect of the button view `at` showed.
    fn button(at: usize) -> (Id, Rect) {
        BUTTONS.with(|buttons| buttons[at].get())
    }
}

fn painted(frames: &Frames, text: &str) -> bool {
    frames.texts().iter().any(|run| run.text == text)
}

/// An untouched settled view runs no component, yet its paint shows in each frame and its accessibility nodes stay
/// in the tree, under the same parents.
#[trial]
fn a_settled_view_does_not_run_and_still_paints() {
    let scene = Scene::new(false, false);
    let mut frames = Frames::new();
    frames.frame(vec![], |ui| scene.show(ui));
    let ids = [0, 1].map(|at| Scene::button(at).0);
    let tree = frames.tree();
    let parents = ids.map(|id| tree.parent_of(id));
    assert!(parents.iter().all(Option::is_some), "the buttons have nodes in a frame that runs them");
    scene.settle(&mut frames);
    for frame in 0..SETTLED {
        assert_eq!(scene.frame(&mut frames, vec![]), [0, 0], "settled frame {frame} ran a view");
        assert!(painted(&frames, "Alpha") && painted(&frames, "Beta"), "settled frame {frame} paints both views");
        let tree = frames.tree();
        assert_eq!(
            ids.map(|id| tree.name(id).map(str::to_owned)),
            [Some("Alpha".to_owned()), Some("Beta".to_owned())],
            "settled frame {frame} keeps the nodes"
        );
        assert_eq!(ids.map(|id| tree.parent_of(id)), parents, "settled frame {frame} keeps the nodes' parents");
    }
}

/// The pointer over a settled view runs it, and only it, in the frame it arrives, from the measures its last run
/// kept; leaving, the view runs until its hover has faded out, then settles again.
#[trial]
fn hovering_a_settled_view_runs_it_alone() {
    let scene = Scene::new(false, false);
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    let over = Scene::button(1).1.center();
    let [reader, other] = scene.frame(&mut frames, vec![moved(over)]);
    assert!(other > 0, "the hovered view runs in the frame the pointer arrives");
    let measured = last_pass_measuring_runs(&frames.ctx);
    assert!(measured.is_empty(), "the hovered view's body takes its kept measure: {measured:?}");
    assert_eq!(reader, 0, "the view beside it stays settled");
    let away = Pos2::new(ORIGIN.x + ROOM.x - 2.0, ORIGIN.y + ROOM.y - 2.0);
    let [_, other] = scene.frame(&mut frames, vec![moved(away)]);
    assert!(other > 0, "the view the pointer left runs to show its hover leaving");
    scene.settle(&mut frames);
    assert_eq!(scene.frame(&mut frames, vec![]), [0, 0], "once its hover faded, the view settles again");
}

/// A pointer that moves within one part of a settled view, once its hover has settled, leaves it settled: what
/// sensing gives the view is the same.
#[trial]
fn a_move_within_a_settled_views_part_replays_it() {
    let scene = Scene::new(false, false);
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    let over = Scene::button(1).1.center();
    scene.frame(&mut frames, vec![moved(over)]);
    scene.settle(&mut frames);
    for step in 1..=3 {
        let at = over + Vec2::new(step as f32, 0.0);
        assert_eq!(scene.frame(&mut frames, vec![moved(at)]), [0, 0], "move {step} within the hovered button replays both views");
    }
}

/// A pointer that moves onto another part of a settled view runs that view in the frame it arrives, and only it; the
/// view settles again once its hover has.
#[trial]
fn a_move_onto_another_part_runs_the_view_once() {
    let scene = Scene::new(false, false);
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    scene.frame(&mut frames, vec![moved(Scene::button(0).1.center())]);
    scene.settle(&mut frames);
    let [reader, other] = scene.frame(&mut frames, vec![moved(READER.center())]);
    assert!(reader > 0, "the move from the button onto the scrub runs the view that shows both");
    assert_eq!(other, 0, "the view beside it stays settled");
    scene.settle(&mut frames);
    assert_eq!(scene.frame(&mut frames, vec![]), [0, 0], "once its hover settled, the view replays again");
}

/// A view that marks a rect as following the pointer.
#[derive(MemoProps)]
struct Follower {
    follows: bool,
}

fn follower(props: &mut Follower, ui: &mut Ui) {
    let response = Button::new("Gamma").show(ui).response;
    BUTTONS.with(|buttons| buttons[0].set((response.id, response.rect)));
    if props.follows {
        craft_ui::runtime::pointer::follows_pointer(ui.ctx(), response.rect);
    }
}

/// A settled view whose run marked a rect as following the pointer runs on every move within it; one that marked
/// none replays on the same moves.
#[trial]
fn a_view_that_follows_the_pointer_runs_on_every_move() {
    for follows in [true, false] {
        let show = |ui: &mut Ui| {
            in_region(ui, Rect::from_min_size(ORIGIN, ROOM), |ui| {
                MemoView::new("follower", Follower { follows }, follower).show(ui);
            });
        };
        let mut frames = Frames::new();
        frames.idle(30, show);
        let over = Scene::button(0).1.center();
        frames.frame(vec![moved(over)], show);
        frames.idle(30, show);
        for step in 1..=3 {
            frames.frame(vec![moved(over + Vec2::new(step as f32, 0.0))], show);
            let runs = runs_of(&frames.ctx, "Follower");
            assert_eq!(runs > 0, follows, "move {step}: the view runs exactly when its run follows the pointer ({follows})");
        }
    }
}

/// A press on a part of a settled view runs that view, and only it, and so does its release.
#[trial]
fn a_press_and_its_release_run_the_view() {
    let scene = Scene::new(false, false);
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    let at = Scene::button(1).1.center();
    scene.frame(&mut frames, vec![moved(at)]);
    scene.settle(&mut frames);
    let [reader, other] = scene.frame(&mut frames, vec![press(at)]);
    assert!(other > 0 && reader == 0, "the press runs the view under it alone: {reader} {other}");
    let [reader, other] = scene.frame(&mut frames, vec![release(at)]);
    assert!(other > 0 && reader == 0, "the release runs it alone: {reader} {other}");
}

/// A modifier change runs the settled view under the pointer, whose parts sense with the held modifiers, and not the
/// view beside it; both settle again with the modifier held.
#[trial]
fn a_modifier_change_under_the_pointer_runs_the_view() {
    let scene = Scene::new(false, false);
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    scene.frame(&mut frames, vec![moved(Scene::button(1).1.center())]);
    scene.settle(&mut frames);
    frames.hold(egui::Modifiers::SHIFT);
    let [reader, other] = scene.frame(&mut frames, vec![]);
    assert!(other > 0, "Shift going down runs the view under the pointer");
    assert_eq!(reader, 0, "the view away from the pointer stays settled");
    scene.settle(&mut frames);
    assert_eq!(scene.frame(&mut frames, vec![]), [0, 0], "with Shift held, both views settle again");
}

/// Frames with motion off: a hover shows and fades in one frame, with no fade that runs its view.
fn still_frames() -> Frames {
    let mut frames = Frames::new();
    frames.root(RootEnv::default().motion(MotionMode::Off));
    frames
}

/// A view showing a list of `count` rows, which notes each row's rect as it builds it.
#[derive(MemoProps)]
struct Rows {
    count: usize,
}

fn rows(props: &mut Rows, ui: &mut Ui) {
    let keys: Vec<usize> = (0..props.count).collect();
    let source = craft_behavior::keyed(&keys, |key: &usize| *key);
    List::new("Rows").show_items(ui, &source, &mut |key, item| {
        ROW_RECTS.with(|rects| {
            let mut rects = rects.borrow_mut();
            if rects.len() <= *key {
                rects.resize(*key + 1, Rect::NOTHING);
            }
            rects[*key] = item.rect;
        });
        ListRow::new(format!("Row {key}"))
    });
}

/// The rect of the row `key` of the rows view, as its last run built it.
fn row_rect(key: usize) -> Rect {
    ROW_RECTS.with(|rects| rects.borrow()[key])
}

/// The rows view of `count` rows in the scene's room.
fn show_rows(count: usize) -> impl Fn(&mut Ui) + Copy {
    move |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, ROOM), |ui| {
            MemoView::new("rows", Rows { count }, rows).show(ui);
        });
    }
}

/// The fills painted as rects inside `row` across most of its height: a row's hover fill among them.
fn fills_in(shapes: &[egui::Shape], row: Rect) -> usize {
    shapes
        .iter()
        .filter(|shape| {
            matches!(shape, egui::Shape::Rect(rect)
                if rect.fill.a() > 0 && row.expand(0.5).contains_rect(rect.rect) && rect.rect.height() > row.height() / 2.0)
        })
        .count()
}

/// A pointer moving within one row of a list in a settled view replays the view; a move onto another row runs it
/// once, and the hover fill moves to that row. A list of 10 rows, and one of 40 that scrolls.
#[trial]
fn a_list_in_a_settled_view_runs_only_when_the_pointer_changes_rows() {
    for count in [10, 40] {
        let show = show_rows(count);
        let mut frames = still_frames();
        frames.idle(30, show);
        let (first, second) = (row_rect(1), row_rect(2));
        frames.frame(vec![moved(first.center())], show);
        frames.idle(30, show);
        for step in 1..=3 {
            frames.frame(vec![moved(first.center() + Vec2::new(10.0 * step as f32, 0.0))], show);
            assert_eq!(runs_of(&frames.ctx, "Rows"), 0, "{count} rows: move {step} within the row replays the view");
        }
        let shapes = frames.frame_shapes(show);
        assert!(fills_in(&shapes, first) > 0, "{count} rows: the row under the pointer shows the hover fill");
        assert_eq!(fills_in(&shapes, second), 0, "{count} rows: the next row shows none");
        frames.frame(vec![moved(second.center())], show);
        assert_eq!(runs_of(&frames.ctx, "Rows"), 1, "{count} rows: the move onto the next row runs the view once");
        frames.idle(30, show);
        assert_eq!(runs_of(&frames.ctx, "Rows"), 0, "{count} rows: the view settles again");
        let shapes = frames.frame_shapes(show);
        assert!(fills_in(&shapes, second) > 0, "{count} rows: the row the pointer reached shows the hover fill");
        assert_eq!(fills_in(&shapes, first), 0, "{count} rows: the row it left shows none");
    }
}

/// A wheel over the rows of a list in a settled view runs the view, whose list takes the wheel and scrolls.
#[trial]
fn a_wheel_over_a_scroll_area_in_a_settled_view_runs_it() {
    let show = show_rows(40);
    let mut frames = still_frames();
    frames.idle(30, show);
    let before = row_rect(1);
    frames.frame(vec![moved(before.center())], show);
    frames.idle(30, show);
    frames.frame(vec![wheel(-30.0)], show);
    assert!(runs_of(&frames.ctx, "Rows") > 0, "the wheel runs the view whose list takes it");
    frames.idle(30, show);
    assert_ne!(row_rect(1), before, "the rows scrolled");
}

/// A view showing a check box, whose hover asks for the pointing hand.
#[derive(MemoProps)]
struct Checked {}

fn checked(_: &mut Checked, ui: &mut Ui) {
    let response = Checkbox::new(Bind::given(true)).label("Grid").show(ui).response;
    BUTTONS.with(|buttons| buttons[0].set((response.id, response.rect)));
}

/// A settled view replayed under the pointer asks for the cursor its run asked for.
#[trial]
fn a_replayed_hovered_view_keeps_its_cursor() {
    let show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, ROOM), |ui| {
            MemoView::new("checked", Checked {}, checked).show(ui);
        });
    };
    let mut frames = still_frames();
    frames.idle(30, show);
    let at = Scene::button(0).1.center();
    frames.frame(vec![moved(at)], show);
    assert_eq!(frames.cursor_icon(), egui::CursorIcon::PointingHand, "the hovered check box asks for the pointing hand");
    frames.idle(30, show);
    frames.frame(vec![moved(at + Vec2::new(1.0, 0.0))], show);
    assert_eq!(runs_of(&frames.ctx, "Checked"), 0, "the move within the check box replays the view");
    assert_eq!(frames.cursor_icon(), egui::CursorIcon::PointingHand, "the replay asks for the cursor its run asked for");
}

/// A view showing a button with a tooltip.
#[derive(MemoProps)]
struct Tipped {}

fn tipped(_: &mut Tipped, ui: &mut Ui) {
    let response = Button::new("Delta").tooltip("Hint").show(ui).response;
    BUTTONS.with(|buttons| buttons[0].set((response.id, response.rect)));
}

/// A tooltip shows once the pointer has rested on its button for the delay: the settled view replays while it waits,
/// runs once at the wake its run asked for, which opens the tooltip's layer, and once more in the next frame, when the
/// layer, sized in the first, paints the tooltip; then it replays with the tooltip shown.
#[trial]
fn a_tooltip_shows_after_its_delay_with_one_run_at_its_wake() {
    let show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, ROOM), |ui| {
            MemoView::new("tipped", Tipped {}, tipped).show(ui);
        });
    };
    let mut frames = still_frames();
    frames.idle(30, show);
    frames.frame(vec![moved(Scene::button(0).1.center())], show);
    frames.idle(2, show);
    // A second of frames, past the delay.
    let seen: Vec<(usize, bool)> = (0..60)
        .map(|_| {
            frames.frame(vec![], show);
            (runs_of(&frames.ctx, "Tipped"), painted(&frames, "Hint"))
        })
        .collect();
    let shown = seen.iter().position(|(_, shown)| *shown).expect("the tooltip shows within a second");
    let ran: Vec<usize> = (0..seen.len()).filter(|&frame| seen[frame].0 > 0).collect();
    assert!(shown > 0, "the tooltip waits for its delay: {seen:?}");
    assert_eq!(ran, [shown - 1, shown], "the view runs at the wake and in the frame the tooltip first paints: {seen:?}");
    assert!(seen[shown..].iter().all(|(_, shown)| *shown), "the tooltip stays shown: {seen:?}");
}

/// Drags the editor right in steps; returns per frame from the press on the passes, the runs of each view, and whether
/// the reader showed the dragged value.
fn drag(scene: &Scene) -> Vec<(usize, [usize; 2], bool)> {
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    let from = Pos2::new(AT.left() + 10.0, AT.center().y);
    scene.frame(&mut frames, vec![moved(from)]);
    let mut seen = Vec::new();
    for step in 0..=4 {
        let event = if step == 0 { press(from) } else { moved(from + Vec2::new(10.0 * step as f32 + 10.0, 0.0)) };
        let passes = frames.frame(vec![event], |ui| scene.show(ui));
        let shows = SEEN.with(|seen| seen.shown.get()) == Some(**scene.value.borrow());
        seen.push((passes, [runs_of(&frames.ctx, "Reader"), runs_of(&frames.ctx, "Other")], shows));
    }
    scene.frame(&mut frames, vec![release(from + Vec2::new(50.0, 0.0))]);
    seen
}

/// A drag of a control outside both views, bound to a value one of them reads, runs that view in each drag frame and
/// leaves the other settled, with the editor before the views in the pass and after them. The press, input outside
/// both views, runs neither.
#[trial]
fn an_edit_runs_the_settled_view_that_read_the_value() {
    for editor_first in [true, false] {
        let frames = drag(&Scene::new(editor_first, false));
        assert_eq!(frames[0].1, [0, 0], "the press runs no view (editor first: {editor_first}): {frames:?}");
        for (passes, [reader, other], shows) in &frames[2..] {
            assert!(*reader > 0, "the reader runs in each drag frame (editor first: {editor_first}): {frames:?}");
            assert_eq!(*other, 0, "the other view stays settled (editor first: {editor_first}): {frames:?}");
            assert!(*shows, "the reader shows the dragged value (editor first: {editor_first}): {frames:?}");
            let expected = if editor_first { 1 } else { 2 };
            assert_eq!(*passes, expected, "passes per drag frame (editor first: {editor_first}): {frames:?}");
        }
    }
}

/// A settled view whose room changes runs in that frame.
#[trial]
fn a_settled_view_runs_when_its_room_changes() {
    let scene = Scene::new(false, false);
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    scene.room.set(ROOM + Vec2::new(40.0, 0.0));
    assert_eq!(scene.frame(&mut frames, vec![]), [1, 1], "both views run at the new room");
    assert_eq!(scene.frame(&mut frames, vec![]), [0, 0], "and settle at it");
}

/// A view with a running animation (a spinner) runs every frame; the view beside it stays settled.
#[trial]
fn a_view_with_a_running_animation_runs_every_frame() {
    let scene = Scene::new(false, true);
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    for frame in 0..SETTLED {
        assert_eq!(scene.frame(&mut frames, vec![]), [0, 1], "frame {frame}: the spinner's view runs, the other does not");
    }
}

/// The app commands the command scene's views show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "settleTrial")]
#[non_exhaustive]
enum Shown {
    #[command(label = "Shown", modal = "always")]
    Visible,
    #[command(label = "Beside", modal = "always")]
    Beside,
    #[command(label = "Measured", modal = "always")]
    Measured,
}

/// A command whose state an instance after the views publishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "settleScopedTrial", scope = "trial.settle")]
#[non_exhaustive]
enum Scoped {
    #[command(label = "Published", modal = "always")]
    Published,
}

const SCOPED: CommandScope = <Scoped as CommandSet>::SCOPE;

/// The inspection record whose pointed element the command reader view reads.
const POINTED: &str = "settle-pointed";

/// The command reader view: the commands `Shown` and `Published`, and `Measured` in a content-sized body; it reads the
/// element `POINTED` points at.
#[derive(MemoProps)]
struct CommandReader {}

fn command_reader(_: &mut CommandReader, ui: &mut Ui) {
    let shown = Button::command(Shown::Visible).show(ui).id;
    let published = Button::command(Scoped::Published).show(ui).id;
    COMMAND_BUTTONS.with(|buttons| {
        buttons[0].set(shown);
        buttons[2].set(published);
    });
    Stack::new().sizing(Sizing::Content).id_salt("measured").show(ui, |ui, _| {
        let id = Button::command(Shown::Measured).show(ui).id;
        COMMAND_BUTTONS.with(|buttons| buttons[3].set(id));
    });
    inspect::pointed(ui.ctx(), Id::NULL.with(POINTED));
}

/// The view beside the reader: the command `Beside`.
#[derive(MemoProps)]
struct Beside {}

fn beside(_: &mut Beside, ui: &mut Ui) {
    let id = Button::command(Shown::Beside).show(ui).id;
    COMMAND_BUTTONS.with(|buttons| buttons[1].set(id));
}

/// Two memoized views, the command reader and the view beside it. After them, an instance of `Published`'s scope, a
/// content-sized body, publishes its state, and the record is pointed at `pointed`.
struct CommandScene {
    published: Cell<bool>,
    pointed: Cell<Option<usize>>,
}

impl CommandScene {
    fn new() -> CommandScene {
        CommandScene { published: Cell::new(true), pointed: Cell::new(None) }
    }

    fn show(&self, ui: &mut Ui) {
        in_region(ui, Rect::from_min_size(ORIGIN, ROOM), |ui| {
            MemoView::new("reader", CommandReader {}, command_reader).show(ui);
            MemoView::new("beside", Beside {}, beside).show(ui);
            Stack::new().command_scope(SCOPED).sizing(Sizing::Content).id_salt("publisher").show(ui, |_, cx| {
                cx.publish(Scoped::Published, CommandState::new().enabled(self.published.get()));
            });
        });
        inspect::point_at(ui.ctx(), Id::NULL.with(POINTED), self.pointed.get());
    }

    /// The passes of one frame with no input, and the runs of each view in it.
    fn frame(&self, frames: &mut Frames) -> (usize, [usize; 2]) {
        let passes = frames.frame(vec![], |ui| self.show(ui));
        (passes, [runs_of(&frames.ctx, "CommandReader"), runs_of(&frames.ctx, "Beside")])
    }

    fn settle(&self, frames: &mut Frames) {
        frames.idle(30, |ui| self.show(ui));
    }

    /// Whether the button of `at` is disabled in the last frame's tree.
    fn disabled(&self, frames: &mut Frames, at: usize) -> bool {
        frames.tree().is_disabled(COMMAND_BUTTONS.with(|buttons| buttons[at].get()))
    }
}

/// The app disabling a command a settled view shows runs that view, and only it, in the same frame, which then shows
/// the command disabled; settled again, it keeps showing it so.
#[trial]
fn an_app_command_state_runs_the_settled_view_that_shows_it() {
    let scene = CommandScene::new();
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    assert!(!scene.disabled(&mut frames, 0), "Shown starts enabled");
    let mut states = Tracked::new(CommandStates::new());
    let mut disable = |enabled: bool| {
        states.set(Shown::Visible, CommandState::new().enabled(enabled));
        CommandInput::new().states(&states)
    };
    frames.commands(disable(false));
    let (_, [reader, beside]) = scene.frame(&mut frames);
    assert!(reader > 0 && beside == 0, "only the view that shows Shown runs: {reader}, {beside}");
    assert!(scene.disabled(&mut frames, 0), "it shows Shown disabled in the frame the app disables it");
    assert!(!scene.disabled(&mut frames, 1), "Beside stays enabled");
    scene.settle(&mut frames);
    assert_eq!(scene.frame(&mut frames).1, [0, 0], "both views settle again");
    assert!(scene.disabled(&mut frames, 0), "a settled frame keeps Shown disabled");
    frames.commands(disable(true));
    let (_, [reader, _]) = scene.frame(&mut frames);
    assert!(reader > 0 && !scene.disabled(&mut frames, 0), "enabling it again runs the view, which shows it enabled");
}

/// A state published after a settled view that shows it, and a record pointed at after a settled view that read it,
/// run that view in the same frame: the change reruns the pass.
#[trial]
fn a_change_after_a_settled_reader_runs_it_in_the_same_frame() {
    let scene = CommandScene::new();
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    assert!(!scene.disabled(&mut frames, 2), "Published starts enabled");
    assert_eq!(scene.frame(&mut frames).0, 1, "a steady frame whose measured body publishes the same state takes one pass");
    scene.published.set(false);
    let (passes, [reader, beside]) = scene.frame(&mut frames);
    assert!(reader > 0 && beside == 0, "only the view that shows Published runs: {reader}, {beside}");
    assert!(passes > 1, "the publish reruns the pass");
    assert!(scene.disabled(&mut frames, 2), "it shows Published disabled in the frame it is published");
    scene.settle(&mut frames);
    assert_eq!(scene.frame(&mut frames).1, [0, 0], "both views settle again");
    assert!(scene.disabled(&mut frames, 2), "a settled frame keeps Published disabled");
    scene.pointed.set(Some(1));
    let (_, [reader, beside]) = scene.frame(&mut frames);
    assert!(reader > 0 && beside == 0, "pointing the record runs only the view that read it: {reader}, {beside}");
}

/// A command shown in a content-sized body, whose measuring run resolves it too, settles; the app disabling the command
/// runs the view that shows it in the same frame, which then shows it disabled.
#[trial]
fn a_command_read_in_a_measured_body_settles_and_runs_when_its_state_changes() {
    let scene = CommandScene::new();
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    assert_eq!(scene.frame(&mut frames).1, [0, 0], "the view holding the measured body settles");
    let mut states = Tracked::new(CommandStates::new());
    states.set(Shown::Measured, CommandState::new().enabled(false));
    frames.commands(CommandInput::new().states(&states));
    let (_, [reader, beside]) = scene.frame(&mut frames);
    assert!(reader > 0 && beside == 0, "only the view that shows Measured runs: {reader}, {beside}");
    assert!(scene.disabled(&mut frames, 3), "it shows Measured disabled in the frame the app disables it");
    scene.settle(&mut frames);
    assert_eq!(scene.frame(&mut frames).1, [0, 0], "it settles again");
    assert!(scene.disabled(&mut frames, 3), "a settled frame keeps Measured disabled");
}

/// The radio view: a group of three radios bound to the picked size.
#[derive(MemoProps)]
struct Radios<'a> {
    value: Bind<'a, u8>,
}

fn radios(props: &mut Radios<'_>, ui: &mut Ui) {
    RadioGroup::new("Size", props.value.reborrowed()).show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            for (at, label) in [(1u8, "Small"), (2, "Medium"), (3, "Large")] {
                Radio::new(at).label(label).show(ui);
            }
        });
    });
}

/// A settled view's replay keeps the arenas it shows: a press that arrives with the pointer, in the frame the pointer
/// first comes over the view, reaches the radio under it, though the group ran last several frames ago.
#[trial]
fn a_press_with_the_pointer_arriving_reaches_a_member_of_a_settled_view() {
    let mut frames = Frames::new();
    let picked = Cell::new(1u8);
    let mut show = |ui: &mut Ui| {
        let mut value = picked.get();
        in_region(ui, Rect::from_min_size(ORIGIN, ROOM), |ui| {
            MemoView::new("radios", Radios { value: Bind::bound(&mut value) }, radios).show(ui);
        });
        picked.set(value);
    };
    frames.frame(vec![], &mut show);
    let medium = frames.texts().iter().find(|run| run.text == "Medium").map(|run| run.rect.center()).expect("Medium is painted");
    frames.idle(30, &mut show);
    frames.frame(vec![moved(medium), press(medium)], &mut show);
    frames.frame(vec![release(medium)], &mut show);
    assert_eq!(picked.get(), 2, "the press that came with the pointer picks Medium");
}

/// A view showing one button.
#[derive(MemoProps)]
struct Labelled {
    label: &'static str,
}

fn labelled(props: &mut Labelled, ui: &mut Ui) {
    Button::new(props.label).show(ui);
}

/// A fixture showing a view of the button `label`, settled.
fn labelled_fixture(label: &'static str) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        MemoView::new("labelled", Labelled { label }, labelled).show(ui);
    });
    fixture.settle();
    fixture
}

/// The runs of the button view in one frame with no input.
fn labelled_frame(fixture: &mut Fixture) -> usize {
    fixture.frame(Vec::new());
    runs_of(fixture.ctx(), "Labelled")
}

/// A view runs when the theme changes, whose tokens it painted with, and when the density does; it settles again after
/// each.
#[trial]
fn a_settled_view_runs_when_the_theme_or_density_changes() {
    let mut fixture = labelled_fixture("Themed");
    assert_eq!(labelled_frame(&mut fixture), 0, "the view settles");
    fixture.theme(Mode::Light, Contrast::High);
    assert!(labelled_frame(&mut fixture) > 0, "a theme change runs it");
    assert_eq!(labelled_frame(&mut fixture), 0, "and it settles in the new theme");
    fixture.density(Size::Comfortable);
    assert!(labelled_frame(&mut fixture) > 0, "a density change runs it");
}

/// A view runs when the host switches the language, and settles again in it.
#[trial]
fn a_settled_view_runs_when_the_language_changes() {
    let mut fixture = labelled_fixture("Save");
    assert_eq!(labelled_frame(&mut fixture), 0, "the view settles");
    let (i18n, problems) = craft_i18n::I18n::from_owned([("de".to_owned(), String::new(), "@id\ttrial.save\tSpeichern\n".to_owned())], &["de"]);
    assert!(problems.is_empty(), "{problems:?}");
    let host = craft_host::Host::builder(craft_host::AppInfo { id: "trial", name: "Trial", version: "0" }).i18n(std::sync::Arc::new(i18n)).build();
    fixture.host(std::sync::Arc::new(host));
    assert!(labelled_frame(&mut fixture) > 0, "a new language runs it");
    assert_eq!(labelled_frame(&mut fixture), 0, "and it settles in that language, the host kept");
}
