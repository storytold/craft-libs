//! kittest tests of the node-graph editor on a small host graph: the tree (roles, names,
//! states), keyboard linking and unlinking, moving and removing nodes, the node browser, the
//! limits, and undo through the host.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use crate::node_editor::{Edit, English, Entry, GraphSource, Limits, Link, NodeEditor, NodeEditorText, NodeInfo, Part};

/// A host graph: nodes with one output and two inputs, and links that may not loop.
#[derive(Clone, Debug, Default, PartialEq)]
struct Toy {
    nodes: Vec<(u64, String, Option<[f32; 2]>)>,
    links: Vec<Link>,
    next: u64,
}

impl Toy {
    fn add(&mut self, title: &str) -> u64 {
        let id = self.next;
        self.next += 1;
        self.nodes.push((id, title.to_string(), None));
        id
    }

    fn reaches(&self, from: u64, to: u64) -> bool {
        let mut stack = vec![from];
        let mut seen = std::collections::HashSet::new();
        while let Some(n) = stack.pop() {
            if n == to {
                return true;
            }
            if seen.insert(n) {
                stack.extend(self.links.iter().filter(|l| l.to == n).map(|l| l.from));
            }
        }
        false
    }

    fn apply(&mut self, e: &Edit) {
        match e {
            Edit::Add { kind, pos } => {
                let id = self.add(kind);
                if let Some(n) = self.nodes.iter_mut().find(|n| n.0 == id) {
                    n.2 = Some(*pos);
                }
            }
            Edit::Remove(id) => {
                self.nodes.retain(|n| n.0 != *id);
                self.links.retain(|l| l.from != *id && l.to != *id);
            }
            Edit::Link(l) => {
                self.links.retain(|x| !(x.to == l.to && x.input == l.input));
                self.links.push(*l);
            }
            Edit::Unlink { to, input } => self.links.retain(|x| !(x.to == *to && x.input == *input)),
            Edit::Move { node, pos } => {
                if let Some(n) = self.nodes.iter_mut().find(|n| n.0 == *node) {
                    n.2 = Some(*pos);
                }
            }
        }
    }
}

impl GraphSource for Toy {
    fn nodes(&self) -> Vec<NodeInfo> {
        self.nodes
            .iter()
            .map(|(id, title, pos)| NodeInfo {
                id: *id,
                title: title.clone(),
                kind: "Toy".into(),
                inputs: vec!["Left".into(), "Right".into()],
                outputs: vec!["Out".into()],
                pos: *pos,
            })
            .collect()
    }

    fn links(&self) -> Vec<Link> {
        self.links.clone()
    }

    fn can_link(&self, l: &Link) -> bool {
        l.from != l.to && !self.reaches(l.from, l.to)
    }

    fn catalog(&self) -> Vec<Entry> {
        [("Wave", "Sources"), ("Noise", "Sources"), ("Clamp", "Maths"), ("Remap", "Maths")]
            .iter()
            .map(|(t, c)| Entry { id: t.to_string(), title: t.to_string(), category: c.to_string() })
            .collect()
    }
}

struct State {
    graph: Toy,
    editor: NodeEditor,
    limits: Limits,
    /// Whole graphs before each edit, as a host's undo keeps them.
    undo: Vec<Toy>,
}

fn harness(graph: Toy) -> Harness<'static, State> {
    let state = State { graph, editor: NodeEditor::default(), limits: Limits::default(), undo: Vec::new() };
    Harness::builder().with_size(egui::vec2(900.0, 600.0)).build_ui_state(
        |ui, s: &mut State| {
            if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z))
                && let Some(g) = s.undo.pop()
            {
                s.graph = g;
            }
            for e in s.editor.show(ui, "toy", &s.graph, s.limits) {
                s.undo.push(s.graph.clone());
                s.graph.apply(&e);
            }
        },
        state,
    )
}

fn two() -> Toy {
    let mut g = Toy::default();
    g.add("Alpha");
    g.add("Beta");
    g
}

fn focus(h: &mut Harness<'static, State>, part: Part) {
    h.state_mut().editor.focus(part);
    h.run();
    h.run();
}

fn focused_label(h: &Harness<'static, State>) -> String {
    h.query_all(egui_kittest::kittest::by()).find(|n| n.is_focused()).and_then(|n| n.accesskit_node().label()).unwrap_or_default()
}

fn press(h: &mut Harness<'static, State>, k: Key) {
    h.key_press(k);
    h.run();
    h.run();
}

#[test]
fn nodes_and_ports_are_in_the_tree_with_roles_names_and_state() {
    let mut h = harness(two());
    h.run();
    h.get_by_role_and_label(Role::Canvas, "Node graph, 2 nodes and 0 links");
    h.get_by_role_and_label(Role::Group, "Alpha, Toy node");
    h.get_by_role_and_label(Role::Button, "Left input of Beta, not linked");
    h.get_by_role_and_label(Role::Button, "Out output of Alpha, 0 links");
    // Tab walks from a node into its ports.
    focus(&mut h, Part::Node(0));
    assert_eq!(focused_label(&h), "Alpha, Toy node");
    press(&mut h, Key::Tab);
    assert_eq!(focused_label(&h), "Left input of Alpha, not linked");
    // Enter on a node selects it, and the tree says so.
    focus(&mut h, Part::Node(0));
    press(&mut h, Key::Enter);
    assert!(h.get_by_role_and_label(Role::Group, "Alpha, Toy node").accesskit_node().is_selected() == Some(true));
    assert_eq!(h.state().editor.announcement, "Alpha selected");
}

#[test]
fn a_link_is_made_and_removed_from_the_keyboard_and_undone() {
    let mut h = harness(two());
    h.run();
    // Enter on Alpha's output starts a link; Enter on Beta's Right input ends it.
    focus(&mut h, Part::Output(0, 0));
    press(&mut h, Key::Enter);
    assert_eq!(h.state().editor.pending, Some((0, 0)));
    assert!(
        h.get_by_role_and_label(Role::Button, "Out output of Alpha, 0 links").accesskit_node().is_selected() == Some(true),
        "the start of the link shows"
    );
    assert!(h.state().editor.announcement.starts_with("Linking from Out of Alpha"));
    focus(&mut h, Part::Input(1, 1));
    press(&mut h, Key::Enter);
    assert_eq!(h.state().graph.links, vec![Link { from: 0, output: 0, to: 1, input: 1 }]);
    assert_eq!(h.state().editor.pending, None);
    h.get_by_role_and_label(Role::Button, "Right input of Beta, linked from Alpha");
    assert_eq!(h.state().editor.announcement, "Right of Beta linked from Alpha");
    // A link back the other way would loop: refused, and said so.
    focus(&mut h, Part::Output(1, 0));
    press(&mut h, Key::Enter);
    focus(&mut h, Part::Input(0, 0));
    press(&mut h, Key::Enter);
    assert_eq!(h.state().graph.links.len(), 1);
    assert!(h.state().editor.announcement.starts_with("Not linked"), "{}", h.state().editor.announcement);
    // Delete on the linked input removes the link; undo brings it back.
    focus(&mut h, Part::Input(1, 1));
    press(&mut h, Key::Delete);
    assert!(h.state().graph.links.is_empty());
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(h.state().graph.links.len(), 1, "undo restores the link");
}

#[test]
fn esc_cancels_a_link_being_made_and_keeps_the_focus() {
    let mut h = harness(two());
    h.run();
    focus(&mut h, Part::Output(0, 0));
    press(&mut h, Key::Enter);
    press(&mut h, Key::Escape);
    assert_eq!(h.state().editor.pending, None);
    assert_eq!(h.state().editor.announcement, "Link cancelled");
    assert_eq!(focused_label(&h), "Out output of Alpha, 0 links");
    assert!(h.state().graph.links.is_empty());
}

#[test]
fn nodes_move_and_go_from_the_keyboard() {
    let mut h = harness(two());
    h.run();
    let before = crate::node_editor::layout(&h.state().graph.nodes(), &[])[&1];
    focus(&mut h, Part::Node(1));
    press(&mut h, Key::ArrowRight);
    press(&mut h, Key::ArrowDown);
    let pos = h.state().graph.nodes.iter().find(|n| n.0 == 1).and_then(|n| n.2).unwrap();
    assert_eq!(pos, [before[0] + crate::node_editor::NUDGE, before[1] + crate::node_editor::NUDGE]);
    assert_eq!(h.state().undo.len(), 2, "each move is a step");
    assert_eq!(focused_label(&h), "Beta, Toy node", "arrows move the node, not the focus");
    press(&mut h, Key::Delete);
    assert_eq!(h.state().graph.nodes.len(), 1);
    assert_eq!(h.state().editor.announcement, "Beta removed");
    assert!(focused_label(&h).starts_with("Node graph"), "the focus goes back to the canvas");
}

#[test]
fn the_canvas_pans_and_zooms_from_the_keyboard() {
    let mut h = harness(two());
    h.run();
    focus(&mut h, Part::Canvas);
    press(&mut h, Key::ArrowRight);
    assert!(h.state().editor.pan.x > 0.0);
    press(&mut h, Key::Plus);
    assert!(h.state().editor.zoom > 1.0);
    press(&mut h, Key::Num0);
    assert_eq!((h.state().editor.pan, h.state().editor.zoom), (egui::Vec2::ZERO, 1.0));
    assert!(h.state().undo.is_empty(), "the view is not an edit");
    assert!(focused_label(&h).starts_with("Node graph"));
}

#[test]
fn the_node_browser_searches_chooses_with_arrows_and_adds_with_enter() {
    let mut h = harness(two());
    h.run();
    focus(&mut h, Part::Canvas);
    h.key_press_modifiers(Modifiers::SHIFT, Key::A);
    h.run();
    h.run();
    assert!(h.state().editor.wants_escape());
    assert_eq!(focused_label(&h), "Search nodes");
    h.get_by_role_and_label(Role::ListBoxOption, "Wave, Sources");
    // Searching by category leaves Clamp and Remap; Down picks the second.
    h.event(egui::Event::Text("math".into()));
    h.run();
    assert!(h.query_by_role_and_label(Role::ListBoxOption, "Wave, Sources").is_none());
    press(&mut h, Key::ArrowDown);
    assert!(h.get_by_role_and_label(Role::ListBoxOption, "Remap, Maths").accesskit_node().is_selected() == Some(true));
    assert_eq!(h.state().editor.announcement, "Remap, Maths");
    press(&mut h, Key::Enter);
    assert_eq!(h.state().graph.nodes.last().map(|n| n.1.clone()), Some("Remap".into()));
    assert!(!h.state().editor.wants_escape());
    assert!(focused_label(&h).starts_with("Node graph"));
    // Esc closes the browser without adding anything.
    h.key_press_modifiers(Modifiers::SHIFT, Key::A);
    h.run();
    h.run();
    press(&mut h, Key::Escape);
    assert!(!h.state().editor.wants_escape());
    assert_eq!(h.state().graph.nodes.len(), 3);
}

#[test]
fn limits_are_checked_before_an_edit_is_made() {
    let mut h = harness(two());
    h.state_mut().limits = Limits { max_nodes: 2, max_links: 1 };
    h.run();
    focus(&mut h, Part::Canvas);
    h.key_press_modifiers(Modifiers::SHIFT, Key::A);
    h.run();
    h.run();
    press(&mut h, Key::Enter);
    assert_eq!(h.state().graph.nodes.len(), 2, "no third node");
    assert_eq!(h.state().editor.announcement, "Not added: a graph holds at most 2 nodes");
    // One link is allowed; a second new link is not, but replacing the first one is.
    for (from, to, input) in [(0, 1, 0), (0, 1, 1), (0, 1, 0)] {
        focus(&mut h, Part::Output(from, 0));
        press(&mut h, Key::Enter);
        focus(&mut h, Part::Input(to, input));
        press(&mut h, Key::Enter);
    }
    assert_eq!(h.state().graph.links, vec![Link { from: 0, output: 0, to: 1, input: 0 }]);
    assert_eq!(h.state().undo.len(), 2, "the first link, then the replacement; the second new link was refused");
}

#[test]
fn the_layout_places_nodes_in_columns_and_survives_a_loop() {
    use crate::node_editor::layout;
    let mut g = Toy::default();
    for t in ["A", "B", "C"] {
        g.add(t);
    }
    g.links = vec![Link { from: 0, output: 0, to: 1, input: 0 }, Link { from: 1, output: 0, to: 2, input: 0 }];
    let p = layout(&g.nodes(), &g.links());
    assert!(p[&0][0] < p[&1][0] && p[&1][0] < p[&2][0], "{p:?}");
    // A loop from a host that let one through still finishes.
    g.links.push(Link { from: 2, output: 0, to: 0, input: 1 });
    assert_eq!(layout(&g.nodes(), &g.links()).len(), 3);
}

/// A host's text: English with the canvas and node names in another wording.
struct Host;

impl NodeEditorText for Host {
    fn add_node(&self) -> String {
        English.add_node()
    }
    fn reset_view(&self) -> String {
        English.reset_view()
    }
    fn status(&self, nodes: usize, links: usize, zoom_percent: f32) -> String {
        English.status(nodes, links, zoom_percent)
    }
    fn canvas(&self, nodes: usize, links: usize) -> String {
        format!("Graphe, {nodes} nœuds, {links} liens")
    }
    fn node(&self, title: &str, kind: &str) -> String {
        format!("{title}, nœud {kind}")
    }
    fn selected(&self, title: &str) -> String {
        format!("{title} choisi")
    }
    fn removed(&self, title: &str) -> String {
        English.removed(title)
    }
    fn input_linked(&self, input: &str, title: &str, from: &str) -> String {
        English.input_linked(input, title, from)
    }
    fn input_not_linked(&self, input: &str, title: &str) -> String {
        English.input_not_linked(input, title)
    }
    fn output(&self, output: &str, title: &str, links: usize) -> String {
        English.output(output, title, links)
    }
    fn unlinked(&self, input: &str, title: &str) -> String {
        English.unlinked(input, title)
    }
    fn input_hint_linked(&self, input: &str, from: &str) -> String {
        English.input_hint_linked(input, from)
    }
    fn input_hint_not_linked(&self, input: &str) -> String {
        English.input_hint_not_linked(input)
    }
    fn linking_from(&self, output: &str, title: &str) -> String {
        English.linking_from(output, title)
    }
    fn link_cancelled(&self) -> String {
        English.link_cancelled()
    }
    fn too_many_links(&self, max: usize) -> String {
        English.too_many_links(max)
    }
    fn cannot_link(&self, from: &str, input: &str, to: &str) -> String {
        English.cannot_link(from, input, to)
    }
    fn linked(&self, input: &str, to: &str, from: &str) -> String {
        English.linked(input, to, from)
    }
    fn browser_title(&self) -> String {
        English.browser_title()
    }
    fn search_hint(&self) -> String {
        English.search_hint()
    }
    fn search_name(&self) -> String {
        English.search_name()
    }
    fn entry(&self, title: &str, category: &str) -> String {
        English.entry(title, category)
    }
    fn no_match(&self) -> String {
        English.no_match()
    }
    fn too_many_nodes(&self, max: usize) -> String {
        English.too_many_nodes(max)
    }
    fn added(&self, title: &str) -> String {
        English.added(title)
    }
}

#[test]
fn the_host_text_names_the_tree_and_the_announcements() {
    let state = State { graph: two(), editor: NodeEditor::default(), limits: Limits::default(), undo: Vec::new() };
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 600.0)).build_ui_state(
        |ui, s: &mut State| {
            for e in s.editor.show_with_text(ui, "toy", &s.graph, s.limits, &Host) {
                s.graph.apply(&e);
            }
        },
        state,
    );
    h.run();
    h.get_by_role_and_label(Role::Canvas, "Graphe, 2 nœuds, 0 liens");
    assert!(h.query_by_role_and_label(Role::Group, "Alpha, Toy node").is_none(), "no English name is left");
    focus(&mut h, Part::Node(0));
    assert_eq!(focused_label(&h), "Alpha, nœud Toy");
    press(&mut h, Key::Enter);
    assert_eq!(h.state().editor.announcement, "Alpha choisi");
}
