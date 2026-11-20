//! Settling children (`Lazy::settles`): a settled child adds its last run to the pass in place of running, and runs
//! when something it shows may have changed: the pointer over it, a bound value, inspection record or command state it
//! read, its room, its content version, its environment (theme, density), an animation or a repaint asked inside it.

use crate::values::kit::{AT, INITIAL, Scrub, Seen};
use craft_theme::{Contrast, Mode, Size};
use craft_ui::buttons::Button;
use craft_ui::choice::{Radio, RadioGroup};
use craft_ui::contract::{Bind, CommandSet, Lazy, Props as _, Show as _, ShowContainer as _, Sizing, Tracked};
use craft_ui::display::Spinner;
use craft_ui::layout_components::Stack;
use craft_ui::runtime::commands::{CommandInput, CommandScope, CommandState, CommandStates, Commands as _};
use craft_ui::runtime::{frame, inspect};
use craft_ui::testing::layout::in_region;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, Frames, Settle, last_pass_measuring_runs, last_pass_runs, last_pass_settles, last_pass_text_layouts, trial};
use egui::{Id, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

const ORIGIN: Pos2 = Pos2::new(10.0, 60.0);
const ROOM: Vec2 = Vec2::new(300.0, 200.0);
/// Where the bound scrub of the reader child is shown, inside that child's room.
const READER: Rect = Rect { min: Pos2::new(150.0, 70.0), max: Pos2::new(250.0, 90.0) };
/// Settled frames a trial looks at.
const SETTLED: usize = 4;

/// Two settling children, `reader` and `other`, under a scrub bound to the value the reader's own scrub shows. The
/// scrub that edits comes before the children in the pass when `editor_first`, else after them. Each child shows the
/// content version in `versions`, if any; `other` asks for a repaint in each run while `repaints` holds, and the app
/// asks for one outside both children while `app_repaints` holds.
struct Scene {
    value: RefCell<f32>,
    room: Cell<Vec2>,
    editor_first: bool,
    spinner: bool,
    versions: [Cell<Option<u64>>; 2],
    repaints: Cell<bool>,
    app_repaints: Cell<bool>,
    /// Whether the scene shows only what is outside the settling children.
    bare: Cell<bool>,
    runs: [Cell<usize>; 2],
    buttons: [Cell<(Id, Rect)>; 2],
    seen: Seen,
}

impl Scene {
    fn new(editor_first: bool, spinner: bool) -> Scene {
        Scene {
            value: RefCell::new(INITIAL),
            room: Cell::new(ROOM),
            editor_first,
            spinner,
            versions: Default::default(),
            repaints: Cell::new(false),
            app_repaints: Cell::new(false),
            bare: Cell::new(false),
            runs: Default::default(),
            buttons: [Cell::new((Id::NULL, Rect::NOTHING)), Cell::new((Id::NULL, Rect::NOTHING))],
            seen: Seen::default(),
        }
    }

    /// The settling child `key`, with the content version of child `at`.
    fn child(&self, key: &str, at: usize) -> Lazy {
        let child = Lazy::new(key, 0.0).settles(true);
        match self.versions[at].get() {
            Some(version) => child.content(version),
            None => child,
        }
    }

    fn show(&self, ui: &mut Ui) {
        if self.app_repaints.get() {
            frame::request_repaint(ui.ctx());
        }
        if self.editor_first {
            self.editor(ui);
        }
        if !self.bare.get() {
            in_region(ui, Rect::from_min_size(ORIGIN, self.room.get()), |ui| {
                self.child("reader", 0).show(ui, |ui| {
                    self.runs[0].set(self.runs[0].get() + 1);
                    let response = Button::new("Alpha").show(ui).response;
                    self.buttons[0].set((response.id, response.rect));
                    Scrub::new(Bind::bound(&mut *self.value.borrow_mut())).id_salt("reader").seen(&self.seen).show_at(ui, READER);
                });
                self.child("other", 1).show(ui, |ui| {
                    self.runs[1].set(self.runs[1].get() + 1);
                    if self.repaints.get() {
                        frame::request_repaint(ui.ctx());
                    }
                    // In a body, whose measure a run that is not settled takes from what its last run kept.
                    Stack::new().show(ui, |ui, _| {
                        let response = Button::new("Beta").show(ui).response;
                        self.buttons[1].set((response.id, response.rect));
                    });
                    if self.spinner {
                        Spinner::new().show(ui);
                    }
                });
            });
        }
        if !self.editor_first {
            self.editor(ui);
        }
    }

    fn editor(&self, ui: &mut Ui) {
        Scrub::new(Bind::bound(&mut *self.value.borrow_mut())).id_salt("editor").show_at(ui, AT);
    }

    /// The runs of each child in one frame with `events`.
    fn frame(&self, frames: &mut Frames, events: Vec<egui::Event>) -> [usize; 2] {
        self.runs.iter().for_each(|runs| runs.set(0));
        frames.frame(events, |ui| self.show(ui));
        [self.runs[0].get(), self.runs[1].get()]
    }

    /// Frames with no input until both children are settled.
    fn settle(&self, frames: &mut Frames) {
        frames.idle(30, |ui| self.show(ui));
    }
}

fn painted(frames: &Frames, text: &str) -> bool {
    frames.texts().iter().any(|run| run.text == text)
}

/// An untouched settled child runs no component, yet its paint shows in each frame and its accessibility nodes stay
/// in the tree, under the same parents.
#[trial]
fn a_settled_child_does_not_run_and_still_paints() {
    let scene = Scene::new(false, false);
    let mut frames = Frames::new();
    frames.frame(vec![], |ui| scene.show(ui));
    let ids = scene.buttons.each_ref().map(|button| button.get().0);
    let tree = frames.tree();
    let parents = ids.map(|id| tree.parent_of(id));
    assert!(parents.iter().all(Option::is_some), "the buttons have nodes in a frame that runs them");
    scene.settle(&mut frames);
    for frame in 0..SETTLED {
        assert_eq!(scene.frame(&mut frames, vec![]), [0, 0], "settled frame {frame} ran a child");
        assert!(painted(&frames, "Alpha") && painted(&frames, "Beta"), "settled frame {frame} paints both children");
        let tree = frames.tree();
        assert_eq!(
            ids.map(|id| tree.name(id).map(str::to_owned)),
            [Some("Alpha".to_owned()), Some("Beta".to_owned())],
            "settled frame {frame} keeps the nodes"
        );
        assert_eq!(ids.map(|id| tree.parent_of(id)), parents, "settled frame {frame} keeps the nodes' parents");
    }
}

/// After two warm-up frames, an idle frame with no input runs nothing that settles: every keyed child replays, the
/// components that run are those outside every settling child, the frame takes one pass and lays out no text.
#[trial]
fn an_idle_frame_runs_nothing_that_settles() {
    let scene = Scene::new(false, false);
    scene.versions.iter().for_each(|version| version.set(Some(1)));
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    let reference = Scene::new(false, false);
    reference.bare.set(true);
    let mut bare = Frames::new();
    reference.settle(&mut bare);
    let outside = {
        reference.frame(&mut bare, vec![]);
        last_pass_runs(&bare.ctx).iter().map(|(_, runs)| runs).sum::<u32>()
    };
    let passes = frames.frame(vec![], |ui| scene.show(ui));
    let ran = last_pass_settles(&frames.ctx).into_iter().filter(|(_, what)| *what == Settle::Ran).count();
    let runs = last_pass_runs(&frames.ctx);
    assert_eq!(ran, 0, "an idle frame ran keyed children: {:?}", last_pass_settles(&frames.ctx));
    assert_eq!(runs.iter().map(|(_, runs)| runs).sum::<u32>(), outside, "components ran inside settling children: {runs:?}");
    assert_eq!(passes, 1, "an idle frame takes one pass");
    assert_eq!(last_pass_text_layouts(&frames.ctx), 0, "an idle frame laid out text");
}

/// The pointer over a settled child runs it, and only it, in the frame it arrives, from the measures its last run
/// kept; leaving, the child runs until its hover has faded out, then settles again.
#[trial]
fn hovering_a_settled_child_runs_it_alone() {
    let scene = Scene::new(false, false);
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    let over = scene.buttons[1].get().1.center();
    let [reader, other] = scene.frame(&mut frames, vec![moved(over)]);
    assert!(other > 0, "the hovered child runs in the frame the pointer arrives");
    let measured = last_pass_measuring_runs(&frames.ctx);
    assert!(measured.is_empty(), "the hovered child's body takes its kept measure: {measured:?}");
    assert_eq!(reader, 0, "the child beside it stays settled");
    let away = Pos2::new(ORIGIN.x + ROOM.x - 2.0, ORIGIN.y + ROOM.y - 2.0);
    let [_, other] = scene.frame(&mut frames, vec![moved(away)]);
    assert!(other > 0, "the child the pointer left runs to show its hover leaving");
    scene.settle(&mut frames);
    assert_eq!(scene.frame(&mut frames, vec![]), [0, 0], "once its hover faded, the child settles again");
}

/// Drags the editor right in steps; returns per drag frame the passes, the runs of each child, and whether the
/// reader showed the dragged value.
fn drag(scene: &Scene) -> Vec<(usize, [usize; 2], bool)> {
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    let from = Pos2::new(AT.left() + 10.0, AT.center().y);
    scene.frame(&mut frames, vec![moved(from)]);
    scene.frame(&mut frames, vec![press(from)]);
    let mut seen = Vec::new();
    for step in 1..=4 {
        scene.runs.iter().for_each(|runs| runs.set(0));
        let passes = frames.frame(vec![moved(from + Vec2::new(10.0 * step as f32 + 10.0, 0.0))], |ui| scene.show(ui));
        let shows = scene.seen.shown.get() == Some(*scene.value.borrow());
        seen.push((passes, [scene.runs[0].get(), scene.runs[1].get()], shows));
    }
    scene.frame(&mut frames, vec![release(from + Vec2::new(50.0, 0.0))]);
    seen
}

/// A drag of a control outside both children, bound to a value one of them reads, runs that child in each drag frame
/// and leaves the other settled, with the editor before the children in the pass and after them. The frame after the
/// press runs both: input unsettles every child for its frame and the next.
#[trial]
fn an_edit_runs_the_settled_child_that_read_the_value() {
    for editor_first in [true, false] {
        let frames = drag(&Scene::new(editor_first, false));
        for (passes, [reader, other], shows) in &frames[1..] {
            assert!(*reader > 0, "the reader runs in each drag frame (editor first: {editor_first}): {frames:?}");
            assert_eq!(*other, 0, "the other child stays settled (editor first: {editor_first}): {frames:?}");
            assert!(*shows, "the reader shows the dragged value (editor first: {editor_first}): {frames:?}");
            let expected = if editor_first { 1 } else { 2 };
            assert_eq!(*passes, expected, "passes per drag frame (editor first: {editor_first}): {frames:?}");
        }
    }
}

/// A settled child whose room changes runs in that frame.
#[trial]
fn a_settled_child_runs_when_its_room_changes() {
    let scene = Scene::new(false, false);
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    scene.room.set(ROOM + Vec2::new(40.0, 0.0));
    assert_eq!(scene.frame(&mut frames, vec![]), [1, 1], "both children run at the new room");
    assert_eq!(scene.frame(&mut frames, vec![]), [0, 0], "and settle at it");
}

/// A child with a running animation (a spinner) runs every frame; the child beside it stays settled.
#[trial]
fn a_child_with_a_running_animation_runs_every_frame() {
    let scene = Scene::new(false, true);
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    for frame in 0..SETTLED {
        assert_eq!(scene.frame(&mut frames, vec![]), [0, 1], "frame {frame}: the spinner's child runs, the other does not");
    }
}

/// A repaint asked inside a settling child runs that child again, and only it.
#[trial]
fn a_repaint_asked_inside_a_settled_child_runs_only_that_child() {
    let scene = Scene::new(false, false);
    scene.repaints.set(true);
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    for frame in 0..SETTLED {
        assert_eq!(scene.frame(&mut frames, vec![]), [0, 1], "frame {frame}: the child that asks runs, the other does not");
    }
}

/// A child with a content version runs in the frame its version changes, and only then; input elsewhere in the window
/// leaves it settled, and a repaint the app asks for outside every child runs it.
#[trial]
fn a_child_with_a_content_version_runs_when_it_changes() {
    let scene = Scene::new(false, false);
    scene.versions.iter().for_each(|version| version.set(Some(1)));
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    let away = Pos2::new(ORIGIN.x + ROOM.x + 90.0, ORIGIN.y + ROOM.y + 40.0);
    for events in [vec![moved(away)], vec![press(away)], vec![release(away)], vec![]] {
        assert_eq!(scene.frame(&mut frames, events.clone()), [0, 0], "input elsewhere ({events:?}) runs no versioned child");
    }
    scene.versions[0].set(Some(2));
    assert_eq!(scene.frame(&mut frames, vec![]), [1, 0], "the child whose version changed runs, the other does not");
    assert_eq!(scene.frame(&mut frames, vec![]), [0, 0], "both settle at the new version");
    scene.app_repaints.set(true);
    assert_eq!(scene.frame(&mut frames, vec![]), [1, 1], "an app repaint outside both children runs both");
}

/// The app commands the command scene's children show.
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

/// A command whose state an instance after the children publishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "settleScopedTrial", scope = "trial.settle")]
#[non_exhaustive]
enum Scoped {
    #[command(label = "Published", modal = "always")]
    Published,
}

const SCOPED: CommandScope = <Scoped as CommandSet>::SCOPE;

/// The inspection record whose pointed element the reader child reads.
const POINTED: &str = "settle-pointed";

/// Two settling children: `reader` shows the commands `Shown` and `Published`, and `Measured` in a content-sized body,
/// and reads the element `POINTED` points at; `beside` shows `Beside`. After them, an instance of `Published`'s scope,
/// a content-sized body, publishes its state, and the record is pointed at `pointed`.
struct CommandScene {
    published: Cell<bool>,
    pointed: Cell<Option<usize>>,
    runs: [Cell<usize>; 2],
    /// The buttons of `Shown`, `Beside`, `Published` and `Measured`.
    buttons: [Cell<Id>; 4],
}

impl CommandScene {
    fn new() -> CommandScene {
        CommandScene {
            published: Cell::new(true),
            pointed: Cell::new(None),
            runs: Default::default(),
            buttons: [Cell::new(Id::NULL), Cell::new(Id::NULL), Cell::new(Id::NULL), Cell::new(Id::NULL)],
        }
    }

    fn show(&self, ui: &mut Ui) {
        let pointed = Id::NULL.with(POINTED);
        in_region(ui, Rect::from_min_size(ORIGIN, ROOM), |ui| {
            Lazy::new("reader", 0.0).settles(true).show(ui, |ui| {
                self.runs[0].set(self.runs[0].get() + 1);
                self.buttons[0].set(Button::command(Shown::Visible).show(ui).id);
                self.buttons[2].set(Button::command(Scoped::Published).show(ui).id);
                Stack::new().sizing(Sizing::Content).id_salt("measured").show(ui, |ui, _| {
                    self.buttons[3].set(Button::command(Shown::Measured).show(ui).id);
                });
                inspect::pointed(ui.ctx(), pointed);
            });
            Lazy::new("beside", 0.0).settles(true).show(ui, |ui| {
                self.runs[1].set(self.runs[1].get() + 1);
                self.buttons[1].set(Button::command(Shown::Beside).show(ui).id);
            });
            Stack::new().command_scope(SCOPED).sizing(Sizing::Content).id_salt("publisher").show(ui, |_, cx| {
                cx.publish(Scoped::Published, CommandState::new().enabled(self.published.get()));
            });
        });
        inspect::point_at(ui.ctx(), pointed, self.pointed.get());
    }

    /// The passes of one frame with no input, and the runs of each child in it.
    fn frame(&self, frames: &mut Frames) -> (usize, [usize; 2]) {
        self.runs.iter().for_each(|runs| runs.set(0));
        let passes = frames.frame(vec![], |ui| self.show(ui));
        (passes, [self.runs[0].get(), self.runs[1].get()])
    }

    fn settle(&self, frames: &mut Frames) {
        frames.idle(30, |ui| self.show(ui));
    }

    /// Whether the button of `at` is disabled in the last frame's tree.
    fn disabled(&self, frames: &mut Frames, at: usize) -> bool {
        frames.tree().is_disabled(self.buttons[at].get())
    }
}

/// The app disabling a command a settled child shows runs that child, and only it, in the same frame, which then shows
/// the command disabled; settled again, it keeps showing it so.
#[trial]
fn an_app_command_state_runs_the_settled_child_that_shows_it() {
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
    assert!(reader > 0 && beside == 0, "only the child that shows Shown runs: {reader}, {beside}");
    assert!(scene.disabled(&mut frames, 0), "it shows Shown disabled in the frame the app disables it");
    assert!(!scene.disabled(&mut frames, 1), "Beside stays enabled");
    scene.settle(&mut frames);
    assert_eq!(scene.frame(&mut frames).1, [0, 0], "both children settle again");
    assert!(scene.disabled(&mut frames, 0), "a settled frame keeps Shown disabled");
    frames.commands(disable(true));
    let (_, [reader, _]) = scene.frame(&mut frames);
    assert!(reader > 0 && !scene.disabled(&mut frames, 0), "enabling it again runs the child, which shows it enabled");
}

/// A state published after a settled child that shows it, and a record pointed at after a settled child that read it,
/// run that child in the same frame: the change reruns the pass.
#[trial]
fn a_change_after_a_settled_reader_runs_it_in_the_same_frame() {
    let scene = CommandScene::new();
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    assert!(!scene.disabled(&mut frames, 2), "Published starts enabled");
    assert_eq!(scene.frame(&mut frames).0, 1, "a steady frame whose measured body publishes the same state takes one pass");
    scene.published.set(false);
    let (passes, [reader, beside]) = scene.frame(&mut frames);
    assert!(reader > 0 && beside == 0, "only the child that shows Published runs: {reader}, {beside}");
    assert!(passes > 1, "the publish reruns the pass");
    assert!(scene.disabled(&mut frames, 2), "it shows Published disabled in the frame it is published");
    scene.settle(&mut frames);
    assert_eq!(scene.frame(&mut frames).1, [0, 0], "both children settle again");
    assert!(scene.disabled(&mut frames, 2), "a settled frame keeps Published disabled");
    scene.pointed.set(Some(1));
    let (_, [reader, beside]) = scene.frame(&mut frames);
    assert!(reader > 0 && beside == 0, "pointing the record runs only the child that read it: {reader}, {beside}");
}

/// A command shown in a content-sized body, whose measuring run resolves it too, settles; the app disabling the command
/// runs the child that shows it in the same frame, which then shows it disabled.
#[trial]
fn a_command_read_in_a_measured_body_settles_and_runs_when_its_state_changes() {
    let scene = CommandScene::new();
    let mut frames = Frames::new();
    scene.settle(&mut frames);
    assert_eq!(scene.frame(&mut frames).1, [0, 0], "the child holding the measured body settles");
    let mut states = Tracked::new(CommandStates::new());
    states.set(Shown::Measured, CommandState::new().enabled(false));
    frames.commands(CommandInput::new().states(&states));
    let (_, [reader, beside]) = scene.frame(&mut frames);
    assert!(reader > 0 && beside == 0, "only the child that shows Measured runs: {reader}, {beside}");
    assert!(scene.disabled(&mut frames, 3), "it shows Measured disabled in the frame the app disables it");
    scene.settle(&mut frames);
    assert_eq!(scene.frame(&mut frames).1, [0, 0], "it settles again");
    assert!(scene.disabled(&mut frames, 3), "a settled frame keeps Measured disabled");
}

/// A settled child's replay keeps the arenas it shows: a press that arrives with the pointer, in the frame the pointer
/// first comes over the child, reaches the radio under it, though the group ran last several frames ago.
#[trial]
fn a_press_with_the_pointer_arriving_reaches_a_member_of_a_settled_child() {
    let mut frames = Frames::new();
    let picked = Cell::new(1u8);
    let mut show = |ui: &mut Ui| {
        let mut value = picked.get();
        in_region(ui, Rect::from_min_size(ORIGIN, ROOM), |ui| {
            Lazy::new("radios", 0.0).settles(true).show(ui, |ui| {
                RadioGroup::new("Size", Bind::bound(&mut value)).show(ui, |ui, _| {
                    Stack::new().show(ui, |ui, _| {
                        for (at, label) in [(1u8, "Small"), (2, "Medium"), (3, "Large")] {
                            Radio::new(at).label(label).show(ui);
                        }
                    });
                })
            });
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

/// A child with a content version runs when the theme changes, whose tokens it painted with, and when the density
/// does; it settles again after each.
#[trial]
fn a_settled_child_runs_when_the_theme_or_density_changes() {
    let runs = std::rc::Rc::new(Cell::new(0));
    let counted = std::rc::Rc::clone(&runs);
    let mut fixture = Fixture::new(move |ui| {
        Lazy::new("themed", 0.0).settles(true).content(1).show(ui, |ui| {
            counted.set(counted.get() + 1);
            Button::new("Themed").show(ui);
        });
    });
    fixture.settle();
    let frame = |fixture: &mut Fixture| {
        runs.set(0);
        fixture.frame(Vec::new());
        runs.get()
    };
    assert_eq!(frame(&mut fixture), 0, "the child settles");
    fixture.theme(Mode::Light, Contrast::High);
    assert!(frame(&mut fixture) > 0, "a theme change runs it");
    assert_eq!(frame(&mut fixture), 0, "and it settles in the new theme");
    fixture.density(Size::Comfortable);
    assert!(frame(&mut fixture) > 0, "a density change runs it");
}

/// A child with a content version runs when the host switches the language, and settles again in it.
#[trial]
fn a_settled_child_runs_when_the_language_changes() {
    let runs = std::rc::Rc::new(Cell::new(0));
    let counted = std::rc::Rc::clone(&runs);
    let mut fixture = Fixture::new(move |ui| {
        Lazy::new("worded", 0.0).settles(true).content(1).show(ui, |ui| {
            counted.set(counted.get() + 1);
            Button::new("Save").show(ui);
        });
    });
    fixture.settle();
    let frame = |fixture: &mut Fixture| {
        runs.set(0);
        fixture.frame(Vec::new());
        runs.get()
    };
    assert_eq!(frame(&mut fixture), 0, "the child settles");
    let (i18n, problems) = craft_i18n::I18n::from_owned([("de".to_owned(), String::new(), "@id\ttrial.save\tSpeichern\n".to_owned())], &["de"]);
    assert!(problems.is_empty(), "{problems:?}");
    let host = craft_host::Host::builder(craft_host::AppInfo { id: "trial", name: "Trial", version: "0" }).i18n(std::sync::Arc::new(i18n)).build();
    fixture.host(std::sync::Arc::new(host));
    assert!(frame(&mut fixture) > 0, "a new language runs it");
    assert_eq!(frame(&mut fixture), 0, "and it settles in that language, the host kept");
}
