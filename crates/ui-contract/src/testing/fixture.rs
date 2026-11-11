//! `Fixture`: one story or closure shown over [`Frames`], with the env a test varies and input by AccessKit query or part.

use super::Frames;
use super::story::{Story, StoryData, StoryOutput};
use crate::runtime::access::Role;
use crate::runtime::commands::{CommandInput, CommandRef, CommandState};
use crate::runtime::egui::access::{AccessTree, NodeKey};
use crate::runtime::egui::fixture::{self, AccessActionKind};
use crate::runtime::egui::injected;
use crate::runtime::egui::sizing::HELD_UPDATE_RERUN;
use crate::runtime::env::{Direction, Overrides, RootEnv, Scale, with_env};
use craft_behavior::{MotionMode, PartRef};
use craft_host::target::Os;
use craft_theme::{Contrast, Mode, Size, TextSize};
use egui::{Event, Id, Key, Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

/// The most frames [`Fixture::settle`] runs.
const SETTLE_FRAMES: usize = 4;

/// A node of the AccessKit tree, as input addresses it.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    /// The node.
    pub node: NodeKey,
    /// Its bounds in points.
    pub rect: Rect,
}

impl Target {
    /// The middle of the node's bounds.
    pub fn center(&self) -> Pos2 {
        self.rect.center()
    }
}

/// The room of a trial, named by the kind of host the component sits in. A trial that needs another size gives it to
/// [`Fixture::room`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Room {
    /// A docked side panel.
    Panel,
    /// A dialog's window.
    Dialog,
    /// An application window.
    Window,
}

impl Room {
    /// The room's size in points.
    pub fn size(self) -> Vec2 {
        match self {
            Room::Panel => Vec2::new(360.0, 600.0),
            Room::Dialog => Vec2::new(800.0, 600.0),
            Room::Window => Vec2::new(1120.0, 720.0),
        }
    }
}

/// One `show` of a story, inside the env scope when there is one.
fn shown(ui: &mut Ui, scope: Option<fn(&mut Overrides)>, show: fn(&mut Ui, &mut StoryData) -> StoryOutput, data: &RefCell<StoryData>) -> StoryOutput {
    match scope {
        Some(scope) => with_env(ui, scope, |ui| show(ui, &mut data.borrow_mut())),
        None => show(ui, &mut data.borrow_mut()),
    }
}

/// Pass `number` of a story under the purity probe: a measuring run, which must change nothing and send nothing, then
/// the real run, which must change nothing in a rerun pass unless that pass delivers an update an earlier pass held.
/// The measuring run's ids differ from the real run's, which draws in `ui` itself.
fn probed_pass(
    ui: &mut Ui,
    number: usize,
    scope: Option<fn(&mut Overrides)>,
    show: fn(&mut Ui, &mut StoryData) -> StoryOutput,
    data: &RefCell<StoryData>,
    delivers: &Cell<bool>,
    impure: &RefCell<Vec<Impure>>,
) -> StoryOutput {
    let ctx = ui.ctx().clone();
    let before = data.borrow().snapshot();
    crate::testing::take_effects(&ctx);
    crate::testing::measuring_run(&ctx, || {
        let mut child = crate::runtime::egui::memory::sizing_child(ui, "measuring run");
        shown(&mut child, scope, show, data);
    });
    let (changed, effects) = (data.borrow().changed_values_since(&before), crate::testing::take_effects(&ctx));
    if !changed.is_empty() || !effects.is_empty() {
        impure.borrow_mut().push(Impure { pass: number, measuring: true, changed, effects });
    }
    let before = data.borrow().snapshot();
    let output = shown(ui, scope, show, data);
    // A pass that held an update for the next pass asks for that rerun, which delivers the update.
    let delivering = delivers.replace(ctx.output(|out| out.request_discard_reasons.iter().any(|cause| cause.reason == HELD_UPDATE_RERUN)));
    if number >= 2 && !delivering {
        let changed = data.borrow().changed_values_since(&before);
        if !changed.is_empty() {
            impure.borrow_mut().push(Impure { pass: number, measuring: false, changed, effects: Vec::new() });
        }
    }
    output
}

/// A closure or story shown frame by frame.
#[non_exhaustive]
pub struct Fixture {
    frames: Frames,
    show: Box<dyn FnMut(&mut Ui)>,
    root: RootEnv,
    commands: CommandInput,
    room: Vec2,
    tree: AccessTree,
    shown: Rect,
    data: Rc<RefCell<StoryData>>,
    outputs: Rc<RefCell<Vec<StoryOutput>>>,
    /// The id of the latest output, which stays when the outputs are taken.
    root_id: Rc<Cell<Option<Id>>>,
    passes: usize,
    /// The passes of the current frame a story's closure has run.
    pass: Rc<Cell<usize>>,
    /// Whether the story's passes run under the purity probe ([`Fixture::probe_purity`]).
    probed: Rc<Cell<bool>>,
    impure: Rc<RefCell<Vec<Impure>>>,
}

/// A pass in which a story's drawing code changed state it must leave alone, as a fixture under
/// [`Fixture::probe_purity`] sees it: in the measuring run the probe adds before each real pass, or in a rerun pass
/// (the second and later pass of a frame), where nothing may change.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Impure {
    /// The pass of its frame, from 1.
    pub pass: usize,
    /// Whether it happened in the measuring run (`true`) or in the real run of a rerun pass (`false`).
    pub measuring: bool,
    /// The story values that changed.
    pub changed: Vec<&'static str>,
    /// The effects (commands, announcements, toasts, requests) the measuring run skipped. Empty for a rerun pass.
    pub effects: Vec<&'static str>,
}

impl Fixture {
    /// A fixture that shows `show` in a room of 400 by 300 points.
    pub fn new(show: impl FnMut(&mut Ui) + 'static) -> Self {
        Fixture {
            frames: Frames::new(),
            show: Box::new(show),
            root: RootEnv::default(),
            commands: CommandInput::new(),
            room: Vec2::new(400.0, 300.0),
            tree: AccessTree::none(),
            shown: Rect::NOTHING,
            data: Rc::default(),
            outputs: Rc::default(),
            root_id: Rc::default(),
            passes: 0,
            pass: Rc::default(),
            probed: Rc::default(),
            impure: Rc::default(),
        }
    }

    /// A fixture that shows `story`; its data and outputs are read with [`Fixture::data`] and [`Fixture::take_outputs`].
    pub fn story(story: &Story) -> Self {
        Self::with_scope(story, None)
    }

    /// A fixture that shows `story` inside an env scope `scope` changes: disabled, read-only, or both.
    pub fn story_scoped(story: &Story, scope: fn(&mut Overrides)) -> Self {
        Self::with_scope(story, Some(scope))
    }

    fn with_scope(story: &Story, scope: Option<fn(&mut Overrides)>) -> Self {
        let data = Rc::new(RefCell::new(StoryData::of(story)));
        let outputs = Rc::new(RefCell::new(Vec::new()));
        let root_id = Rc::new(Cell::new(None));
        let (shared_data, shared_outputs, shared_root, show) = (data.clone(), outputs.clone(), root_id.clone(), story.show);
        let pass = Rc::new(Cell::new(0));
        let probed = Rc::new(Cell::new(false));
        let impure = Rc::new(RefCell::new(Vec::new()));
        let (shared_pass, shared_probed, shared_impure) = (pass.clone(), probed.clone(), impure.clone());
        let shared_delivers = Cell::new(false);
        let mut fixture = Fixture::new(move |ui| {
            let number = shared_pass.get() + 1;
            shared_pass.set(number);
            let output = if shared_probed.get() {
                probed_pass(ui, number, scope, show, &shared_data, &shared_delivers, &shared_impure)
            } else {
                shown(ui, scope, show, &shared_data)
            };
            shared_root.set(output.id);
            shared_outputs.borrow_mut().push(output);
        });
        fixture.pass = pass;
        fixture.probed = probed;
        fixture.impure = impure;
        fixture.data = data;
        fixture.outputs = outputs;
        fixture.root_id = root_id;
        fixture
    }

    /// Runs every following pass of the story under the purity probe: a measuring run of the story before each real
    /// pass, and a comparison of the story's values around each rerun pass. [`Fixture::impure`] reports what it sees.
    /// Set before the first frame; a fixture that shows a closure has no story to probe.
    pub fn probe_purity(&mut self) -> &mut Self {
        self.probed.set(true);
        self
    }

    /// The impure passes the fixture has seen so far under [`Fixture::probe_purity`]; see [`Impure`].
    pub fn impure(&self) -> Vec<Impure> {
        self.impure.borrow().clone()
    }

    fn set_root(&mut self, change: impl FnOnce(&mut RootEnv)) -> &mut Self {
        change(&mut self.root);
        self.frames.root(self.root.clone());
        self
    }

    /// Gives app-scope `command` this state in every following pass, as an app's `FrameInput.commands` does; a later
    /// call for the same command replaces the earlier one.
    pub fn command_state(&mut self, command: impl Into<CommandRef>, state: CommandState) -> &mut Self {
        let command = command.into();
        self.commands.states.retain(|(held, _)| *held != command);
        self.commands.states.push((command, state));
        self.frames.commands(self.commands.clone());
        self
    }

    /// The room the story is shown in.
    pub fn room(&mut self, room: Vec2) -> &mut Self {
        self.room = room;
        self.frames.screen(room);
        self
    }

    /// The room of a host kind: the same as [`Fixture::room`] with that kind's size.
    pub fn in_room(&mut self, room: Room) -> &mut Self {
        self.room(room.size())
    }

    /// The layout direction.
    pub fn direction(&mut self, direction: Direction) -> &mut Self {
        self.set_root(|root| root.direction = direction)
    }

    /// Light or dark, and standard or high contrast.
    pub fn theme(&mut self, mode: Mode, contrast: Contrast) -> &mut Self {
        self.set_root(|root| {
            root.mode = mode.into();
            root.contrast = contrast.into();
        })
    }

    /// The motion mode.
    pub fn motion(&mut self, motion: MotionMode) -> &mut Self {
        self.set_root(|root| *root = std::mem::take(root).motion(motion))
    }

    /// The text-size step.
    pub fn text_size(&mut self, text_size: TextSize) -> &mut Self {
        self.set_root(|root| root.text_size = text_size)
    }

    /// The density preset.
    pub fn density(&mut self, density: Size) -> &mut Self {
        self.set_root(|root| root.density = density)
    }

    /// The interface scale.
    pub fn scale(&mut self, scale: Scale) -> &mut Self {
        self.set_root(|root| root.scale = scale)
    }

    /// The host every following frame is given: its language, catalogs and services.
    pub fn host(&mut self, host: std::sync::Arc<craft_host::Host>) -> &mut Self {
        self.frames.host(host);
        self
    }

    /// The operating system the keys and chrome follow: the shell's OS and capabilities, and egui's own OS for its
    /// text editing.
    pub fn os(&mut self, os: Os) -> &mut Self {
        crate::runtime::windows::set_os(&self.frames.ctx, os);
        fixture::set_os(&self.frames.ctx, os);
        self
    }

    /// Who draws the root window's frame. A native shell puts its menu bar in the window when the app draws the frame
    /// ([`WindowChrome::Integrated`]), as the app's shell does.
    pub fn chrome(&mut self, chrome: craft_host::target::WindowChrome) -> &mut Self {
        crate::runtime::windows::set_chrome(&self.frames.ctx, chrome);
        self
    }

    /// The manual clock, moved on by `seconds` before the next frame.
    pub fn advance(&mut self, seconds: f64) -> &mut Self {
        self.frames.advance(seconds);
        self
    }

    /// The frames' context.
    pub fn ctx(&self) -> &egui::Context {
        &self.frames.ctx
    }

    /// The story's data.
    pub fn data(&self) -> std::cell::RefMut<'_, StoryData> {
        self.data.borrow_mut()
    }

    /// The outputs the story gave since the last call, one per pass.
    pub fn take_outputs(&mut self) -> Vec<StoryOutput> {
        std::mem::take(&mut *self.outputs.borrow_mut())
    }

    /// The component id of the latest shown output, kept when the outputs are taken.
    pub fn root_id(&self) -> Option<Id> {
        self.root_id.get()
    }

    /// The first part of the shown component that senses a drag.
    pub fn drag_part(&self) -> Option<PartRef> {
        crate::runtime::pointer::sensing_parts(self.ctx(), self.root_id()?, craft_behavior::Sense::DRAG).into_iter().next()
    }

    /// The first part of the shown component that senses a click: what a click activates when the component has
    /// parts of its own.
    pub fn click_part(&self) -> Option<PartRef> {
        crate::runtime::pointer::sensing_parts(self.ctx(), self.root_id()?, craft_behavior::Sense::CLICK).into_iter().next()
    }

    /// The styles the pipeline resolved for the component type `type_name` in this fixture.
    pub fn styles_of(&self, type_name: &str) -> Vec<std::sync::Arc<super::style::StyleSeen>> {
        super::style::styles_of(self.ctx(), type_name)
    }

    /// The text runs the latest frame painted.
    pub fn texts(&self) -> &[fixture::PaintedText] {
        self.frames.texts()
    }

    /// The visual bounds of the shapes the latest frame painted.
    pub fn shape_bounds(&self) -> &[Rect] {
        self.frames.shape_bounds()
    }

    /// Half the stroke width of the shape behind each of `shape_bounds`.
    pub fn shape_overshoot(&self) -> &[f32] {
        self.frames.shape_overshoot()
    }

    /// Whether anything asked for another pass since the latest frame ended.
    pub fn repaint_requested(&self) -> bool {
        self.frames.repaint_requested()
    }

    /// The IME output of the latest frame.
    pub fn ime(&self) -> Option<&fixture::ImeSeen> {
        self.frames.ime()
    }

    /// The area the latest pass allocated.
    pub fn shown(&self) -> Rect {
        self.shown
    }

    /// The passes the latest frame ran.
    pub fn passes(&self) -> usize {
        self.passes
    }

    /// One frame with `events`.
    pub fn frame(&mut self, events: Vec<Event>) -> &mut Self {
        let room = Rect::from_min_size(Pos2::ZERO, self.room);
        self.pass.set(0);
        let (show, shown) = (&mut self.show, &mut self.shown);
        self.passes = self.frames.frame(events, |ui| {
            let used = super::layout::in_region(ui, room, |ui| {
                show(ui);
                fixture::used_rect(ui)
            });
            *shown = used;
        });
        self.tree = self.frames.tree();
        self
    }

    /// `n` frames with no events.
    pub fn idle(&mut self, n: usize) -> &mut Self {
        for _ in 0..n {
            self.frame(Vec::new());
        }
        self
    }

    /// Frames with no events until the latest one is stable, where an app stops painting: it ran one pass, asked for
    /// no repaint, and has the shown rect, root id and access node count of the frame before it. The frame before the
    /// first one counts when the fixture has run one, so a settle after input runs at least one frame and a settle of
    /// a fresh fixture at least two. A story that never comes to rest (a running animation) gets four frames.
    pub fn settle(&mut self) -> &mut Self {
        let mut before = (self.passes > 0).then(|| self.signature());
        for _ in 0..SETTLE_FRAMES {
            self.frame(Vec::new());
            let now = self.signature();
            if self.passes == 1 && !self.repaint_requested() && before == Some(now) {
                break;
            }
            before = Some(now);
        }
        self
    }

    /// What [`Fixture::settle`] compares between two frames.
    fn signature(&self) -> (Rect, Option<Id>, usize) {
        (self.shown, self.root_id(), self.tree.node_count())
    }

    /// The AccessKit tree of the latest frame.
    pub fn tree(&self) -> &AccessTree {
        &self.tree
    }

    /// The node by key, with its bounds, when the latest tree has it.
    pub fn target(&self, node: NodeKey) -> Option<Target> {
        Some(Target { node, rect: self.tree.bounds_of(node)? })
    }

    /// The node named `label`: the interactive node whose name resolves to it (its own, or the node that labels it
    /// through `labelled_by`), else the first node that carries the text.
    pub fn by_label(&self, label: &str) -> Option<Target> {
        let tree = &self.tree;
        let reported: HashSet<NodeKey> = tree.keys().into_iter().collect();
        let ordered = self.in_tree_order(&reported);
        let node = ordered
            .iter()
            .copied()
            .find(|key| {
                super::access::name(tree, *key) == Some(label) && tree.role_of(*key) != Some(Role::LABEL) && super::access::is_interactive(tree, *key)
            })
            .or_else(|| ordered.iter().copied().find(|key| tree.name_of(*key) == Some(label)))?;
        self.target(node)
    }

    /// The nodes of `wanted` in tree order: depth first from the root, children in their declared order. `by_label`
    /// takes the first match in this order, so repeated labels resolve to the first on screen.
    fn in_tree_order(&self, wanted: &HashSet<NodeKey>) -> Vec<NodeKey> {
        let mut ordered = Vec::with_capacity(wanted.len());
        let mut pending: Vec<NodeKey> = self.tree.root_key().into_iter().collect();
        while let Some(key) = pending.pop() {
            if wanted.contains(&key) {
                ordered.push(key);
            }
            pending.extend(self.tree.children_of(key).into_iter().rev());
        }
        ordered
    }

    /// The nodes of `role` in tree order.
    pub fn by_role(&self, role: Role) -> Vec<Target> {
        let wanted: HashSet<NodeKey> = self.tree.with_role(role).into_iter().collect();
        self.in_tree_order(&wanted).into_iter().filter_map(|node| self.target(node)).collect()
    }

    /// The sub-node of `part` below the component `component`.
    pub fn part(&self, component: Id, part: impl Into<PartRef>) -> Option<Target> {
        let id = crate::runtime::access::sub_node_id(component, part.into());
        self.target(NodeKey::of(id))
    }

    /// The rect of the part `part` of the component `owner` as the sensing step recorded it, for parts that have no
    /// access node (pointer-only ones). Callers assert how rects relate (inside, below), not their size.
    pub fn part_rect(&self, owner: Id, part: impl Into<PartRef>) -> Option<Rect> {
        crate::runtime::pointer::part_bounds(self.ctx(), owner, part.into())
    }

    /// The node of the component `component`; for a widget whose node is hidden because it folds into its owner's
    /// (a text part's widget), the owner's node.
    pub fn node_of(&self, component: Id) -> Option<Target> {
        let owner = crate::runtime::access::fold::owner_of(&self.frames.ctx, component);
        self.target(NodeKey::of(owner.unwrap_or(component)))
    }

    /// Hover, press and release on `target`, in separate frames, then one idle frame.
    pub fn click(&mut self, target: &Target) -> &mut Self {
        self.click_at(target.center())
    }

    /// Hover, press and release at `pos`, in separate frames, then one idle frame.
    pub fn click_at(&mut self, pos: Pos2) -> &mut Self {
        self.play(injected::click_frames(pos, egui::PointerButton::Primary, 1, Modifiers::NONE));
        self.frame(Vec::new())
    }

    /// The frames of `frames`, one pass each.
    fn play(&mut self, frames: Vec<injected::Frame>) -> &mut Self {
        for events in frames {
            self.frame(events);
        }
        self
    }

    /// The modifiers held in every following frame, until the next call. A key sent while they are held carries
    /// them ([`Fixture::key`]).
    pub fn hold(&mut self, modifiers: Modifiers) -> &mut Self {
        self.frames.hold(modifiers);
        self
    }

    /// Press on `target`.
    pub fn press(&mut self, target: &Target) -> &mut Self {
        let at = target.center();
        self.frame(vec![super::pointer::moved(at)]);
        self.frame(vec![super::pointer::press(at)])
    }

    /// Release at the middle of `target`.
    pub fn release(&mut self, target: &Target) -> &mut Self {
        self.frame(vec![super::pointer::release(target.center())])
    }

    /// Press on `target`, move by `by` in four steps, release.
    pub fn drag(&mut self, target: &Target, by: Vec2) -> &mut Self {
        self.drag_at(target.center(), by)
    }

    /// A drag from the point `from` by `by`: the same frames as [`Fixture::drag`], for a point that is no target.
    pub fn drag_at(&mut self, from: Pos2, by: Vec2) -> &mut Self {
        self.frame(vec![super::pointer::moved(from)]);
        self.frame(vec![super::pointer::press(from)]);
        for step in 1..=4 {
            self.frame(vec![super::pointer::moved(from + by * (step as f32 / 4.0))]);
        }
        self.frame(vec![super::pointer::release(from + by)]);
        self.frame(Vec::new())
    }

    /// The first half of a drag that pauses: press at `from`, move by `by` in four steps, and one frame with the
    /// pointer still, button held. A trial reads the running gesture, presses Escape or checks a snap line, then ends
    /// it with [`Fixture::release_at`].
    pub fn drag_hold_at(&mut self, from: Pos2, by: Vec2) -> &mut Self {
        self.frame(vec![super::pointer::moved(from)]);
        self.frame(vec![super::pointer::press(from)]);
        for step in 1..=4 {
            self.frame(vec![super::pointer::moved(from + by * (step as f32 / 4.0))]);
        }
        self.frame(Vec::new())
    }

    /// Releases the button at `at`, then a frame with no input.
    pub fn release_at(&mut self, at: Pos2) -> &mut Self {
        self.frame(vec![super::pointer::release(at)]);
        self.frame(Vec::new())
    }

    /// A drag from `from` by `by` that pauses before the release: [`Fixture::drag_hold_at`], `halt` with the gesture
    /// running, then [`Fixture::release_at`].
    pub fn drag_at_pausing(&mut self, from: Pos2, by: Vec2, halt: impl FnOnce(&mut Fixture)) -> &mut Self {
        self.drag_hold_at(from, by);
        halt(self);
        self.release_at(from + by)
    }

    /// A key press and release with `modifiers` and the ones [`Fixture::hold`] keeps, as a keyboard reports them:
    /// the modifiers `modifiers` adds go down in the press's frame, the key comes up in the next, and a last frame
    /// returns to the held ones when `modifiers` added any.
    pub fn key(&mut self, key: Key, modifiers: Modifiers) -> &mut Self {
        let held = self.frames.held();
        let pressed = held | modifiers;
        let added = pressed != held;
        let down = added.then(|| injected::modifiers_changed(pressed)).into_iter().chain([injected::key_press(key, pressed)]).collect();
        let mut frames = vec![down, vec![injected::key_release(key, pressed)]];
        if added {
            frames.push(vec![injected::modifiers_changed(held)]);
        }
        self.play(frames)
    }

    /// Typed `text`.
    pub fn type_text(&mut self, text: &str) -> &mut Self {
        self.play(injected::text_frames(text))
    }

    /// An AccessKit action on `target`.
    pub fn access(&mut self, kind: AccessActionKind, target: &Target) -> &mut Self {
        self.frame(vec![fixture::access_request_node(kind, target.node)])
    }

    /// The stop that holds focus: the id of its component and the part when focus is on a part.
    pub fn focused_stop(&self) -> Option<(Id, Option<PartRef>)> {
        crate::runtime::focus::focused_stop(&self.frames.ctx)
    }

    /// The id of the widget egui has focused.
    pub fn focused(&self) -> Option<Id> {
        let id = crate::runtime::egui::focus::focused(&self.frames.ctx)?;
        // A folded text widget is the focus itself, though its node folds into the owner's.
        if crate::runtime::access::fold::is_text_widget(&self.frames.ctx, id) {
            return Some(id);
        }
        // Otherwise the id whose node the access tree reports as focused: the widget itself, or the owner it folds into.
        let focused = self.tree.focused_key();
        let owner = crate::runtime::access::fold::owner_of(&self.frames.ctx, id);
        Some(owner.filter(|owner| NodeKey::of(*owner) == focused).unwrap_or(id))
    }
}

impl Fixture {
    /// `events` as one OS frame, followed by the frames an app runs for it: one for each step of input the one-press
    /// rule held back, then the frame egui repaints for after the last frame that carried input (a release the press
    /// machine reports a pass after its drag started lands there).
    pub fn frame_through(&mut self, events: Vec<Event>) -> &mut Self {
        self.frame(events);
        while crate::runtime::frame::holds_input(self.ctx(), egui::ViewportId::ROOT) {
            self.frame(Vec::new());
        }
        self.frame(Vec::new())
    }
}
