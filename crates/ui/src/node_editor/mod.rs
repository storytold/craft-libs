//! A reusable node-graph editor: nodes with input and output ports on a canvas, joined by links.
//!
//! The editor knows nothing about what the nodes mean. A host describes its graph through
//! [`GraphSource`] (the nodes, the links, which links are allowed, and a catalogue of node kinds
//! to add), and [`NodeEditor::show`] returns the [`Edit`]s the user asked for. The host applies
//! them as its own commands, so every edit is undoable and journaled the way the host's other
//! edits are; the editor itself keeps only the view (pan, zoom, focus, a link being made).
//!
//! **Accessibility.** The canvas, every node and every port are focusable nodes in the AccessKit
//! tree with a role, a name that starts with the visible label, and their state (a node's
//! selection, an input's source). Changes are announced through a live region.
//!
//! **Keyboard** (every mouse action has one):
//! - Tab and Shift+Tab walk the nodes and their ports in order.
//! - On the canvas: arrow keys pan, `+` and `-` zoom, `0` resets the view.
//! - On a node: arrow keys move it, Delete removes it, Enter selects it.
//! - On an output port: Enter starts a link from it; then on an input port, Enter ends the link
//!   there. Esc cancels a link being made.
//! - On an input port: Delete removes its link.
//! - Shift+A (on the canvas or a node) or the Add Node button opens the node browser: type to
//!   search, Up and Down to choose, Enter to add, Esc to close.
//!
//! **Mouse:** drag a node to move it, drag from an output port to an input port to link them,
//! drag the background (or scroll) to pan, Ctrl+scroll to zoom, right-click an input to unlink.
//!
//! **Limits.** The editor refuses an add past [`Limits::max_nodes`] or a new link past
//! [`Limits::max_links`] before it emits the edit, so a host never allocates past them.
//!
//! **Text.** Every word the editor shows or announces comes from a [`NodeEditorText`], so a host
//! passes its translations to [`NodeEditor::show_with_text`]; [`NodeEditor::show`] uses
//! [`English`].

use std::collections::HashMap;
use std::hash::Hash;

use egui::accesskit::{Live, Role};
use egui::{Align2, Color32, CornerRadius, EventFilter, FontId, Id, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

/// A node as the editor shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeInfo {
    /// The host's key for the node.
    pub id: u64,
    /// The node's name, shown in its header.
    pub title: String,
    /// What kind of node it is, shown under the title.
    pub kind: String,
    /// The names of its inputs, top to bottom.
    pub inputs: Vec<String>,
    /// The names of its outputs, top to bottom.
    pub outputs: Vec<String>,
    /// Where it sits on the canvas, in canvas units; `None` lets the editor place it.
    pub pos: Option<[f32; 2]>,
}

/// A link from an output port to an input port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Link {
    pub from: u64,
    pub output: usize,
    pub to: u64,
    pub input: usize,
}

/// A kind of node the browser can add.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The host's id for the kind.
    pub id: String,
    pub title: String,
    pub category: String,
}

/// What the editor asks of the graph it shows.
pub trait GraphSource {
    fn nodes(&self) -> Vec<NodeInfo>;
    fn links(&self) -> Vec<Link>;
    /// Whether `link` may be made (for example, it would not make a loop).
    fn can_link(&self, link: &Link) -> bool;
    /// The kinds of node the browser offers.
    fn catalog(&self) -> Vec<Entry>;
}

/// An edit the user asked for; the host applies it as one undoable step.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// Add a node of the catalogue entry `kind` at `pos` (canvas units).
    Add {
        kind: String,
        pos: [f32; 2],
    },
    Remove(u64),
    /// Make the link; it replaces any link into the same input.
    Link(Link),
    /// Take away the link into `input` of `to`.
    Unlink {
        to: u64,
        input: usize,
    },
    /// Put a node at `pos` (canvas units).
    Move {
        node: u64,
        pos: [f32; 2],
    },
}

/// The most a host lets the graph hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_nodes: usize,
    pub max_links: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { max_nodes: 256, max_links: 1024 }
    }
}

/// The words the editor shows and announces. A host implements it with its translation system;
/// [`English`] is the source text. Names read by screen readers start with the visible label, so
/// keep that order in a translation.
pub trait NodeEditorText {
    /// The button that opens the node browser.
    fn add_node(&self) -> String;
    /// The button that resets pan and zoom.
    fn reset_view(&self) -> String;
    /// The status line under the buttons; `zoom_percent` is already rounded.
    fn status(&self, nodes: usize, links: usize, zoom_percent: f32) -> String;
    /// The canvas's accessible name.
    fn canvas(&self, nodes: usize, links: usize) -> String;
    /// A node's accessible name.
    fn node(&self, title: &str, kind: &str) -> String;
    /// Announced when a node is selected from the keyboard.
    fn selected(&self, title: &str) -> String;
    /// Announced when a node is removed.
    fn removed(&self, title: &str) -> String;
    /// An input port's accessible name while a link comes into it from the node `from`.
    fn input_linked(&self, input: &str, title: &str, from: &str) -> String;
    /// An input port's accessible name while nothing is linked into it.
    fn input_not_linked(&self, input: &str, title: &str) -> String;
    /// An output port's accessible name; `links` is how many links leave it.
    fn output(&self, output: &str, title: &str, links: usize) -> String;
    /// Announced when the link into an input is removed.
    fn unlinked(&self, input: &str, title: &str) -> String;
    /// Announced on Enter on a linked input when no link is being made.
    fn input_hint_linked(&self, input: &str, from: &str) -> String;
    /// Announced on Enter on an unlinked input when no link is being made.
    fn input_hint_not_linked(&self, input: &str) -> String;
    /// Announced when a link is started at an output.
    fn linking_from(&self, output: &str, title: &str) -> String;
    /// Announced when Esc cancels a link being made.
    fn link_cancelled(&self) -> String;
    /// Announced when a new link would pass [`Limits::max_links`].
    fn too_many_links(&self, max: usize) -> String;
    /// Announced when [`GraphSource::can_link`] refuses a link.
    fn cannot_link(&self, from: &str, input: &str, to: &str) -> String;
    /// Announced when a link is made.
    fn linked(&self, input: &str, to: &str, from: &str) -> String;
    /// The node browser's heading.
    fn browser_title(&self) -> String;
    /// The hint inside the browser's search field.
    fn search_hint(&self) -> String;
    /// The browser's search field's accessible name.
    fn search_name(&self) -> String;
    /// A browser entry's accessible name, also announced when it is chosen with the arrows.
    fn entry(&self, title: &str, category: &str) -> String;
    /// Shown when no browser entry matches the search.
    fn no_match(&self) -> String;
    /// Announced when an add would pass [`Limits::max_nodes`].
    fn too_many_nodes(&self, max: usize) -> String;
    /// Announced when a node is added.
    fn added(&self, title: &str) -> String;
}

/// The editor's English source text.
#[derive(Clone, Copy, Debug, Default)]
pub struct English;

impl NodeEditorText for English {
    fn add_node(&self) -> String {
        "Add Node…".into()
    }
    fn reset_view(&self) -> String {
        "Reset View".into()
    }
    fn status(&self, nodes: usize, links: usize, zoom_percent: f32) -> String {
        format!("{nodes} nodes, {links} links, zoom {zoom_percent} %")
    }
    fn canvas(&self, nodes: usize, links: usize) -> String {
        format!("Node graph, {nodes} nodes and {links} links")
    }
    fn node(&self, title: &str, kind: &str) -> String {
        format!("{title}, {kind} node")
    }
    fn selected(&self, title: &str) -> String {
        format!("{title} selected")
    }
    fn removed(&self, title: &str) -> String {
        format!("{title} removed")
    }
    fn input_linked(&self, input: &str, title: &str, from: &str) -> String {
        format!("{input} input of {title}, linked from {from}")
    }
    fn input_not_linked(&self, input: &str, title: &str) -> String {
        format!("{input} input of {title}, not linked")
    }
    fn output(&self, output: &str, title: &str, links: usize) -> String {
        format!("{output} output of {title}, {links} links")
    }
    fn unlinked(&self, input: &str, title: &str) -> String {
        format!("{input} of {title} unlinked")
    }
    fn input_hint_linked(&self, input: &str, from: &str) -> String {
        format!("{input} is linked from {from}; Delete removes the link")
    }
    fn input_hint_not_linked(&self, input: &str) -> String {
        format!("{input} is not linked: start a link at an output, then press Enter here")
    }
    fn linking_from(&self, output: &str, title: &str) -> String {
        format!("Linking from {output} of {title}: go to an input and press Enter, or Esc to cancel")
    }
    fn link_cancelled(&self) -> String {
        "Link cancelled".into()
    }
    fn too_many_links(&self, max: usize) -> String {
        format!("Not linked: a graph holds at most {max} links")
    }
    fn cannot_link(&self, from: &str, input: &str, to: &str) -> String {
        format!("Not linked: {from} cannot feed {input} of {to}")
    }
    fn linked(&self, input: &str, to: &str, from: &str) -> String {
        format!("{input} of {to} linked from {from}")
    }
    fn browser_title(&self) -> String {
        "Add a node".into()
    }
    fn search_hint(&self) -> String {
        "Search".into()
    }
    fn search_name(&self) -> String {
        "Search nodes".into()
    }
    fn entry(&self, title: &str, category: &str) -> String {
        format!("{title}, {category}")
    }
    fn no_match(&self) -> String {
        "No node matches.".into()
    }
    fn too_many_nodes(&self, max: usize) -> String {
        format!("Not added: a graph holds at most {max} nodes")
    }
    fn added(&self, title: &str) -> String {
        format!("{title} added")
    }
}

/// What one frame of [`NodeEditor::show_with_text`] reads, handed to its helpers.
struct Frame<'a> {
    graph: &'a dyn GraphSource,
    links: &'a [Link],
    titles: &'a HashMap<u64, String>,
    node_count: usize,
    limits: Limits,
    text: &'a dyn NodeEditorText,
}

/// Smallest and largest zoom.
pub const ZOOM_RANGE: (f32, f32) = (0.25, 2.5);
/// How far a node moves for one arrow key, in canvas units.
pub const NUDGE: f32 = 20.0;
const PAN_STEP: f32 = 40.0;
const NODE_WIDTH: f32 = 170.0;
const HEADER: f32 = 34.0;
const ROW: f32 = 20.0;
const PORT_RADIUS: f32 = 5.0;
/// Most nodes the automatic layout walks (a host's own cap is lower in practice).
const LAYOUT_CAP: usize = 4096;

/// A part of the editor that can hold the keyboard focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Canvas,
    Node(u64),
    Input(u64, usize),
    Output(u64, usize),
    Search,
}

/// The node browser while it is open.
#[derive(Clone, Debug, Default)]
struct Browser {
    query: String,
    cursor: usize,
    /// Where an added node goes, in canvas units.
    at: [f32; 2],
}

/// The editor's view state; the graph itself stays with the host.
#[derive(Clone, Debug)]
pub struct NodeEditor {
    /// The canvas point at the top left of the view.
    pub pan: Vec2,
    pub zoom: f32,
    /// The node shown selected.
    pub selected: Option<u64>,
    /// An output port a link is being made from.
    pub pending: Option<(u64, usize)>,
    browser: Option<Browser>,
    /// A node being dragged, and how far.
    moving: Option<(u64, Vec2)>,
    /// What gets the focus next frame.
    focus: Option<Part>,
    /// The last announcement, read by screen readers from a live region.
    pub announcement: String,
}

impl Default for NodeEditor {
    fn default() -> NodeEditor {
        NodeEditor {
            pan: Vec2::ZERO,
            zoom: 1.0,
            selected: None,
            pending: None,
            browser: None,
            moving: None,
            focus: None,
            announcement: String::new(),
        }
    }
}

/// Where each node goes: its own position, or for one without, a column by the longest chain of
/// links into it.
pub fn layout(nodes: &[NodeInfo], links: &[Link]) -> HashMap<u64, [f32; 2]> {
    let nodes = &nodes[..nodes.len().min(LAYOUT_CAP)];
    let index: HashMap<u64, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
    // Relax the columns at most once per node, so a loop cannot run on.
    let mut column = vec![0usize; nodes.len()];
    for _ in 0..nodes.len() {
        let mut changed = false;
        for l in links {
            if let (Some(&f), Some(&t)) = (index.get(&l.from), index.get(&l.to))
                && column[t] < column[f] + 1
                && column[f] < nodes.len()
            {
                column[t] = column[f] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut rows: HashMap<usize, f32> = HashMap::new();
    let mut out = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        let pos = n.pos.unwrap_or_else(|| {
            let y = rows.entry(column[i]).or_insert(0.0);
            let p = [20.0 + column[i] as f32 * (NODE_WIDTH + 60.0), 20.0 + *y];
            *y += node_height(n) + 30.0;
            p
        });
        out.insert(n.id, pos);
    }
    out
}

fn node_height(n: &NodeInfo) -> f32 {
    HEADER + ROW * n.inputs.len().max(n.outputs.len()) as f32 + 6.0
}

/// Gives a widget's AccessKit node its name, keeping the role egui gave it.
fn set_name(ui: &egui::Ui, response: &egui::Response, label: String) {
    ui.ctx().accesskit_node_builder(response.id, |n| n.set_label(label));
}

fn name(ui: &egui::Ui, id: Id, role: Role, label: String, f: impl FnOnce(&mut egui::accesskit::Node)) {
    ui.ctx().accesskit_node_builder(id, |n| {
        n.set_role(role);
        n.set_label(label);
        f(n);
    });
}

/// What the editor draws with this frame.
struct Scene<'a> {
    painter: egui::Painter,
    visuals: egui::Visuals,
    view: View,
    wire: Stroke,
    font: FontId,
    small: FontId,
    pointer: Option<Pos2>,
    id: &'a dyn Fn(Part) -> Id,
}

impl Scene<'_> {
    /// A link's wire from an output at `a` to an input at `b`.
    fn curve(&self, a: Pos2, b: Pos2) {
        let dx = ((b.x - a.x).abs() * 0.5).max(30.0 * self.view.zoom);
        self.painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
            [a, a + Vec2::new(dx, 0.0), b - Vec2::new(dx, 0.0), b],
            false,
            Color32::TRANSPARENT,
            self.wire,
        ));
    }
}

/// A node with where it is drawn (`at`, with a drag) and where it rests (`base`), in canvas units.
#[derive(Clone, Copy)]
struct Placed<'a> {
    node: &'a NodeInfo,
    at: [f32; 2],
    base: [f32; 2],
}

/// The view's mapping between canvas units and the screen.
#[derive(Clone, Copy)]
struct View {
    rect: Rect,
    pan: Vec2,
    zoom: f32,
}

impl View {
    fn screen(self, p: [f32; 2]) -> Pos2 {
        self.rect.min + (Vec2::new(p[0], p[1]) - self.pan) * self.zoom
    }

    fn canvas(self, p: Pos2) -> Vec2 {
        (p - self.rect.min) / self.zoom + self.pan
    }

    fn input(self, pos: [f32; 2], k: usize) -> Pos2 {
        self.screen([pos[0], pos[1] + HEADER + ROW * (k as f32 + 0.5)])
    }

    fn output(self, pos: [f32; 2], k: usize) -> Pos2 {
        self.screen([pos[0] + NODE_WIDTH, pos[1] + HEADER + ROW * (k as f32 + 0.5)])
    }
}

/// Whether the key was pressed with no modifier, taking it so nothing else acts on it.
fn key(ui: &egui::Ui, k: Key) -> bool {
    ui.input_mut(|i| i.consume_key(Modifiers::NONE, k))
}

impl NodeEditor {
    /// Move the keyboard focus to `part` next frame.
    pub fn focus(&mut self, part: Part) {
        self.focus = Some(part);
    }

    /// Whether Esc belongs to the editor (a link being made, or the browser open).
    pub fn wants_escape(&self) -> bool {
        self.pending.is_some() || self.browser.is_some()
    }

    /// The id egui gives `part` of an editor shown with `salt`.
    pub fn part_id(salt: impl Hash + std::fmt::Debug, part: Part) -> Id {
        Id::unique((salt, "node-editor", part))
    }

    fn view(&self, rect: Rect) -> View {
        View { rect, pan: self.pan, zoom: self.zoom }
    }

    fn open_browser(&mut self, rect: Rect) {
        let c = self.view(rect).canvas(rect.center());
        self.browser = Some(Browser { query: String::new(), cursor: 0, at: [c.x - NODE_WIDTH * 0.5, c.y - HEADER] });
        self.focus = Some(Part::Search);
    }

    fn say(&mut self, text: String) {
        self.announcement = text;
    }

    fn zoom_at(&mut self, rect: Rect, at: Pos2, factor: f32) {
        let before = self.view(rect).canvas(at);
        self.zoom = (self.zoom * factor).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        let after = self.view(rect).canvas(at);
        self.pan += before - after;
    }

    /// Draw the editor in the rest of `ui` with the [`English`] text and return what the user
    /// asked for.
    pub fn show(&mut self, ui: &mut egui::Ui, salt: impl Hash + std::fmt::Debug + Copy, graph: &dyn GraphSource, limits: Limits) -> Vec<Edit> {
        self.show_with_text(ui, salt, graph, limits, &English)
    }

    /// Draw the editor in the rest of `ui` with the host's `text` and return what the user asked
    /// for.
    pub fn show_with_text(
        &mut self,
        ui: &mut egui::Ui,
        salt: impl Hash + std::fmt::Debug + Copy,
        graph: &dyn GraphSource,
        limits: Limits,
        text: &dyn NodeEditorText,
    ) -> Vec<Edit> {
        let mut edits = Vec::new();
        let nodes = graph.nodes();
        let links = graph.links();
        let positions = layout(&nodes, &links);
        let titles: HashMap<u64, String> = nodes.iter().map(|n| (n.id, n.title.clone())).collect();
        let id = |part: Part| NodeEditor::part_id(salt, part);
        let frame = Frame { graph, links: &links, titles: &titles, node_count: nodes.len(), limits, text };

        let mut open_browser = false;
        ui.horizontal(|ui| {
            let r = ui.button(text.add_node());
            set_name(ui, &r, text.add_node());
            open_browser |= r.clicked();
            if ui.button(text.reset_view()).clicked() {
                (self.pan, self.zoom) = (Vec2::ZERO, 1.0);
            }
            ui.label(egui::RichText::new(text.status(nodes.len(), links.len(), (self.zoom * 100.0).round())).small());
        });

        let size = ui.available_size();
        let (rect, canvas) = ui.allocate_exact_size(Vec2::new(size.x.max(200.0), (size.y - 24.0).max(140.0)), Sense::click_and_drag());
        if open_browser {
            self.open_browser(rect);
        }
        let canvas_id = canvas.id;
        name(ui, canvas_id, Role::Canvas, text.canvas(nodes.len(), links.len()), |_| {});
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals().clone();
        painter.rect_filled(rect, CornerRadius::ZERO, visuals.extreme_bg_color);
        self.grid(&painter, rect, visuals.weak_text_color().gamma_multiply(0.4));
        self.canvas_input(ui, rect, &canvas);

        let view = self.view(rect);
        let moving = self.moving;
        let place = |n: u64| -> [f32; 2] {
            let p = positions.get(&n).copied().unwrap_or([0.0, 0.0]);
            match moving {
                Some((m, d)) if m == n => [p[0] + d.x, p[1] + d.y],
                _ => p,
            }
        };
        let by_id: HashMap<u64, &NodeInfo> = nodes.iter().map(|n| (n.id, n)).collect();
        let scene = Scene {
            painter,
            wire: Stroke::new(2.0 * view.zoom.max(0.5), visuals.widgets.active.fg_stroke.color.gamma_multiply(0.8)),
            font: FontId::proportional(13.0 * view.zoom.clamp(0.6, 1.6)),
            small: FontId::proportional(11.0 * view.zoom.clamp(0.6, 1.6)),
            pointer: ui.input(|i| i.pointer.interact_pos()),
            visuals,
            view,
            id: &id,
        };
        for l in &links {
            if by_id.contains_key(&l.from) && by_id.contains_key(&l.to) {
                scene.curve(view.output(place(l.from), l.output), view.input(place(l.to), l.input));
            }
        }

        let mut hovered_input: Option<(u64, usize)> = None;
        let mut released_drag: Option<(u64, usize)> = None;
        for n in &nodes {
            let placed = Placed { node: n, at: place(n.id), base: positions.get(&n.id).copied().unwrap_or([0.0, 0.0]) };
            self.node_ui(ui, &scene, &frame, &placed, &mut edits);
            // Inputs on the left edge, outputs on the right.
            for port in n.inputs.iter().enumerate() {
                if self.input_port(ui, &scene, &frame, &placed, port, &mut edits) {
                    hovered_input = Some((n.id, port.0));
                }
            }
            for port in n.outputs.iter().enumerate() {
                if self.output_port(ui, &scene, &frame, &placed, port) {
                    released_drag = Some((n.id, port.0));
                }
            }
        }
        // A drag from an output let go over an input links them.
        if let Some((f, o)) = released_drag {
            match hovered_input {
                Some((t, k)) => {
                    let input = by_id.get(&t).and_then(|n| n.inputs.get(k)).cloned().unwrap_or_default();
                    self.finish_link(Link { from: f, output: o, to: t, input: k }, &frame, &input, &mut edits);
                }
                None => self.pending = None,
            }
        }
        if self.pending.is_some() && self.browser.is_none() && key(ui, Key::Escape) {
            self.pending = None;
            self.say(text.link_cancelled());
        }
        let ours = ui.memory(|m| m.focused()).is_some_and(|f| f == canvas_id || nodes.iter().any(|n| f == id(Part::Node(n.id))));
        if ours && ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::A)) {
            self.open_browser(rect);
        }
        self.browser_ui(ui, salt, rect, &frame, &mut edits);

        if let Some(part) = self.focus.take() {
            let fid = match part {
                Part::Canvas => canvas_id,
                Part::Node(n) | Part::Input(n, _) | Part::Output(n, _) if !by_id.contains_key(&n) => canvas_id,
                p => id(p),
            };
            ui.memory_mut(|m| m.request_focus(fid));
        }
        let r = ui.label(egui::RichText::new(self.announcement.clone()).small());
        ui.ctx().accesskit_node_builder(r.id, |n| n.set_live(Live::Polite));
        edits
    }

    /// The canvas: drag or scroll to pan, Ctrl+scroll to zoom, keys while it has the focus.
    fn canvas_input(&mut self, ui: &mut egui::Ui, rect: Rect, canvas: &egui::Response) {
        if canvas.dragged() && self.moving.is_none() {
            self.pan -= canvas.drag_delta() / self.zoom;
        }
        if canvas.clicked() {
            self.selected = None;
            canvas.request_focus();
        }
        if ui.rect_contains_pointer(rect) {
            let (scroll, zoom, pointer) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
            if zoom != 1.0 {
                self.zoom_at(rect, pointer.unwrap_or(rect.center()), zoom);
            } else if scroll != Vec2::ZERO {
                self.pan -= scroll / self.zoom;
            }
        }
        if canvas.has_focus() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(canvas.id, EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() })
            });
            let d = arrows(ui) * PAN_STEP;
            self.pan += d / self.zoom;
            if key(ui, Key::Plus) || key(ui, Key::Equals) {
                self.zoom_at(rect, rect.center(), 1.25);
            }
            if key(ui, Key::Minus) {
                self.zoom_at(rect, rect.center(), 0.8);
            }
            if key(ui, Key::Num0) {
                (self.pan, self.zoom) = (Vec2::ZERO, 1.0);
            }
        }
    }

    /// A node's box: drawn, dragged, and its keys while it has the focus.
    fn node_ui(&mut self, ui: &mut egui::Ui, scene: &Scene<'_>, frame: &Frame<'_>, placed: &Placed<'_>, edits: &mut Vec<Edit>) {
        let Placed { node: n, at: p, base } = *placed;
        let (view, visuals, painter, text) = (scene.view, &scene.visuals, &scene.painter, frame.text);
        let node_rect = Rect::from_min_size(view.screen(p), Vec2::new(NODE_WIDTH, node_height(n)) * view.zoom);
        let nid = (scene.id)(Part::Node(n.id));
        let r = ui.interact(node_rect.intersect(view.rect), nid, Sense::click_and_drag());
        let selected = self.selected == Some(n.id);
        name(ui, nid, Role::Group, text.node(&n.title, &n.kind), |a| a.set_selected(selected));
        let fill = if selected { visuals.selection.bg_fill.gamma_multiply(0.35) } else { visuals.widgets.noninteractive.bg_fill };
        let edge = if r.has_focus() || selected { visuals.selection.stroke.color } else { visuals.widgets.noninteractive.bg_stroke.color };
        painter.rect_filled(node_rect, CornerRadius::same(4), fill);
        painter.rect_stroke(node_rect, CornerRadius::same(4), Stroke::new(if r.has_focus() { 2.5 } else { 1.0 }, edge), StrokeKind::Inside);
        painter.text(node_rect.min + Vec2::new(8.0, 4.0) * view.zoom, Align2::LEFT_TOP, &n.title, scene.font.clone(), visuals.strong_text_color());
        painter.text(node_rect.min + Vec2::new(8.0, 19.0) * view.zoom, Align2::LEFT_TOP, &n.kind, scene.small.clone(), visuals.weak_text_color());
        if r.clicked() {
            self.selected = Some(n.id);
            r.request_focus();
        }
        if r.drag_started() {
            self.moving = Some((n.id, Vec2::ZERO));
        }
        if r.dragged()
            && let Some((m, d)) = &mut self.moving
            && *m == n.id
        {
            *d += r.drag_delta() / view.zoom;
        }
        if r.drag_stopped()
            && let Some((m, d)) = self.moving.take()
            && m == n.id
            && d != Vec2::ZERO
        {
            edits.push(Edit::Move { node: n.id, pos: [base[0] + d.x, base[1] + d.y] });
        }
        if r.has_focus() {
            ui.memory_mut(|m| m.set_focus_lock_filter(nid, EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() }));
            let d = arrows(ui) * NUDGE;
            if d != Vec2::ZERO {
                edits.push(Edit::Move { node: n.id, pos: [base[0] + d.x, base[1] + d.y] });
            }
            if key(ui, Key::Enter) {
                self.selected = Some(n.id);
                self.say(text.selected(&n.title));
            }
            if key(ui, Key::Delete) || key(ui, Key::Backspace) {
                edits.push(Edit::Remove(n.id));
                if self.selected == Some(n.id) {
                    self.selected = None;
                }
                self.focus = Some(Part::Canvas);
                self.say(text.removed(&n.title));
            }
        }
    }

    /// Input port `k` of a node; returns whether the pointer is over it.
    fn input_port(
        &mut self,
        ui: &mut egui::Ui,
        scene: &Scene<'_>,
        frame: &Frame<'_>,
        placed: &Placed<'_>,
        (k, input): (usize, &String),
        edits: &mut Vec<Edit>,
    ) -> bool {
        let (n, view, visuals, text) = (placed.node, scene.view, &scene.visuals, frame.text);
        let c = view.input(placed.at, k);
        let pid = (scene.id)(Part::Input(n.id, k));
        let hit = Rect::from_min_size(c - Vec2::new(PORT_RADIUS * 2.0, ROW * 0.5) * view.zoom, Vec2::new(NODE_WIDTH * 0.5, ROW) * view.zoom);
        let pr = ui.interact(hit.intersect(view.rect), pid, Sense::click());
        let source = frame.links.iter().find(|l| l.to == n.id && l.input == k).copied();
        let from = source.and_then(|l| frame.titles.get(&l.from).cloned());
        let label = match &from {
            Some(f) => text.input_linked(input, &n.title, f),
            None => text.input_not_linked(input, &n.title),
        };
        name(ui, pid, Role::Button, label, |_| {});
        let fill = if source.is_some() { visuals.selection.bg_fill } else { visuals.widgets.inactive.bg_fill };
        scene.painter.circle(c, PORT_RADIUS * view.zoom, fill, Stroke::new(if pr.has_focus() { 2.5 } else { 1.0 }, visuals.text_color()));
        scene.painter.text(c + Vec2::new(10.0, 0.0) * view.zoom, Align2::LEFT_CENTER, input, scene.small.clone(), visuals.text_color());
        let unlink = (pr.secondary_clicked() || (pr.has_focus() && (key(ui, Key::Delete) || key(ui, Key::Backspace)))) && source.is_some();
        if unlink {
            edits.push(Edit::Unlink { to: n.id, input: k });
            self.say(text.unlinked(input, &n.title));
        }
        if pr.clicked() || (pr.has_focus() && (key(ui, Key::Enter) || key(ui, Key::Space))) {
            pr.request_focus();
            match self.pending {
                Some((f, o)) => self.finish_link(Link { from: f, output: o, to: n.id, input: k }, frame, input, edits),
                None => self.say(match &from {
                    Some(f) => text.input_hint_linked(input, f),
                    None => text.input_hint_not_linked(input),
                }),
            }
        }
        if pr.has_focus() && self.pending.is_some() {
            ui.memory_mut(|m| m.set_focus_lock_filter(pid, EventFilter { escape: true, ..Default::default() }));
        }
        scene.pointer.is_some_and(|h| hit.contains(h))
    }

    /// Output port `k` of a node; returns whether a drag from it was let go this frame.
    fn output_port(&mut self, ui: &mut egui::Ui, scene: &Scene<'_>, frame: &Frame<'_>, placed: &Placed<'_>, (k, output): (usize, &String)) -> bool {
        let (n, view, visuals) = (placed.node, scene.view, &scene.visuals);
        let c = view.output(placed.at, k);
        let pid = (scene.id)(Part::Output(n.id, k));
        let hit = Rect::from_min_size(
            c - Vec2::new(NODE_WIDTH * 0.5 - PORT_RADIUS * 2.0, ROW * 0.5) * view.zoom,
            Vec2::new(NODE_WIDTH * 0.5, ROW) * view.zoom,
        );
        let pr = ui.interact(hit.intersect(view.rect), pid, Sense::click_and_drag());
        let pending = self.pending == Some((n.id, k));
        let uses = frame.links.iter().filter(|l| l.from == n.id && l.output == k).count();
        name(ui, pid, Role::Button, frame.text.output(output, &n.title, uses), |a| a.set_selected(pending));
        let fill = if pending { visuals.selection.stroke.color } else { visuals.widgets.inactive.bg_fill };
        scene.painter.circle(c, PORT_RADIUS * view.zoom, fill, Stroke::new(if pr.has_focus() { 2.5 } else { 1.0 }, visuals.text_color()));
        scene.painter.text(c - Vec2::new(10.0, 0.0) * view.zoom, Align2::RIGHT_CENTER, output, scene.small.clone(), visuals.text_color());
        if pr.clicked() || (pr.has_focus() && (key(ui, Key::Enter) || key(ui, Key::Space))) {
            pr.request_focus();
            self.pending = Some((n.id, k));
            self.say(frame.text.linking_from(output, &n.title));
        }
        if pr.drag_started() {
            self.pending = Some((n.id, k));
        }
        if pr.dragged()
            && let Some(h) = scene.pointer
        {
            scene.curve(c, h);
        }
        if pr.has_focus() && self.pending.is_some() {
            ui.memory_mut(|m| m.set_focus_lock_filter(pid, EventFilter { escape: true, ..Default::default() }));
        }
        pr.drag_stopped()
    }

    fn grid(&self, painter: &egui::Painter, rect: Rect, dot: Color32) {
        let step = 40.0 * self.zoom;
        if step < 8.0 {
            return;
        }
        let start = rect.min - Vec2::new((self.pan.x * self.zoom).rem_euclid(step), (self.pan.y * self.zoom).rem_euclid(step));
        let cols = ((rect.width() / step) as usize + 2).min(400);
        let rows = ((rect.height() / step) as usize + 2).min(400);
        for i in 0..cols {
            for j in 0..rows {
                painter.circle_filled(start + Vec2::new(i as f32, j as f32) * step, 1.0, dot);
            }
        }
    }

    fn finish_link(&mut self, link: Link, frame: &Frame<'_>, input: &str, edits: &mut Vec<Edit>) {
        let Frame { graph, links, titles, limits, text, .. } = *frame;
        self.pending = None;
        let replaces = links.iter().any(|l| l.to == link.to && l.input == link.input);
        let to = titles.get(&link.to).cloned().unwrap_or_default();
        let from = titles.get(&link.from).cloned().unwrap_or_default();
        if !replaces && links.len() >= limits.max_links {
            self.say(text.too_many_links(limits.max_links));
        } else if !graph.can_link(&link) {
            self.say(text.cannot_link(&from, input, &to));
        } else {
            edits.push(Edit::Link(link));
            self.say(text.linked(input, &to, &from));
        }
    }

    fn browser_ui(&mut self, ui: &mut egui::Ui, salt: impl Hash + std::fmt::Debug + Copy, rect: Rect, frame: &Frame<'_>, edits: &mut Vec<Edit>) {
        let Frame { graph, node_count: count, limits, text, .. } = *frame;
        let Some(mut b) = self.browser.take() else { return };
        let catalog = graph.catalog();
        let q = b.query.trim().to_lowercase();
        let matches: Vec<&Entry> =
            catalog.iter().filter(|e| q.is_empty() || e.title.to_lowercase().contains(&q) || e.category.to_lowercase().contains(&q)).collect();
        b.cursor = b.cursor.min(matches.len().saturating_sub(1));
        let mut close = false;
        let mut add: Option<String> = None;
        let mut said: Option<String> = None;
        let search_id = NodeEditor::part_id(salt, Part::Search);
        egui::Area::new(Id::unique((salt, "node-browser"))).order(egui::Order::Foreground).fixed_pos(rect.min + Vec2::new(12.0, 12.0)).show(
            ui.ctx(),
            |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(250.0);
                    ui.label(egui::RichText::new(text.browser_title()).strong());
                    let r = ui.add(egui::TextEdit::singleline(&mut b.query).id(search_id).hint_text(text.search_hint()).desired_width(230.0));
                    set_name(ui, &r, text.search_name());
                    if r.has_focus() {
                        ui.memory_mut(|m| {
                            m.set_focus_lock_filter(search_id, EventFilter { vertical_arrows: true, escape: true, ..Default::default() })
                        });
                        let (down, up, esc) = (key(ui, Key::ArrowDown), key(ui, Key::ArrowUp), key(ui, Key::Escape));
                        if down {
                            b.cursor = (b.cursor + 1).min(matches.len().saturating_sub(1));
                        }
                        if up {
                            b.cursor = b.cursor.saturating_sub(1);
                        }
                        close |= esc;
                        if (down || up)
                            && let Some(e) = matches.get(b.cursor)
                        {
                            said = Some(text.entry(&e.title, &e.category));
                        }
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        add = matches.get(b.cursor).map(|e| e.id.clone());
                    }
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        for (i, e) in matches.iter().enumerate() {
                            let r = ui.selectable_label(i == b.cursor, format!("{}  ·  {}", e.title, e.category));
                            ui.ctx().accesskit_node_builder(r.id, |n| {
                                n.set_role(Role::ListBoxOption);
                                n.set_label(text.entry(&e.title, &e.category));
                                n.set_selected(i == b.cursor);
                            });
                            if r.clicked() {
                                add = Some(e.id.clone());
                            }
                        }
                        if matches.is_empty() {
                            ui.label(text.no_match());
                        }
                    });
                });
            },
        );
        if let Some(s) = said {
            self.say(s);
        }
        if let Some(kind) = add {
            if count >= limits.max_nodes {
                self.say(text.too_many_nodes(limits.max_nodes));
            } else {
                let title = catalog.iter().find(|e| e.id == kind).map(|e| e.title.clone()).unwrap_or_default();
                edits.push(Edit::Add { kind, pos: b.at });
                self.say(text.added(&title));
            }
            close = true;
        }
        if close {
            self.focus = Some(Part::Canvas);
        } else {
            self.browser = Some(b);
        }
    }
}

/// The arrow keys pressed this frame, with no modifier, as a direction.
fn arrows(ui: &egui::Ui) -> Vec2 {
    let mut d = Vec2::ZERO;
    if key(ui, Key::ArrowLeft) {
        d.x -= 1.0;
    }
    if key(ui, Key::ArrowRight) {
        d.x += 1.0;
    }
    if key(ui, Key::ArrowUp) {
        d.y -= 1.0;
    }
    if key(ui, Key::ArrowDown) {
        d.y += 1.0;
    }
    d
}

#[cfg(test)]
mod tests;
