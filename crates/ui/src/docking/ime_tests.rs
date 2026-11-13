//! IME descriptors of editors in native floating windows (feature `native-windows`).

use egui::{Event, Id, ImeEvent, Rect, ViewportId, output::IMEOutput, pos2, vec2};

use super::testing::DockDriver;
use super::{DockContent, Floating, Layout, Node};
use crate::ime::{Composer, Edit};
use crate::layout::{SplitAxis, SplitSize};

/// Desktop position of the main window's content.
const ORIGIN: egui::Pos2 = pos2(100.0, 50.0);

struct Content {
    /// The panel whose canvas takes text through `composer`.
    editor: u32,
    composer: Composer,
    typed: String,
    /// A panel whose body clears the pass's IME descriptor, as an app's own editor that
    /// publishes after each body does when it has nothing to show.
    clears: Option<u32>,
    /// The bodies drawn in the last frame, with the viewport each was drawn in.
    drawn: Vec<(u32, ViewportId)>,
}

impl Content {
    fn new(editor: u32) -> Self {
        Self { editor, composer: Composer::new().with_owner(Id::unique("editor")), typed: String::new(), clears: None, drawn: Vec::new() }
    }
}

impl DockContent<u32> for Content {
    fn body(&mut self, ui: &mut egui::Ui, panel: &u32) {
        self.drawn.push((*panel, ui.ctx().viewport_id()));
        if *panel == self.editor {
            for edit in self.composer.update(ui.ctx(), true) {
                if let Edit::Insert(text) = edit {
                    self.typed.push_str(&text);
                }
            }
            self.composer.set_caret(ui.ctx(), Rect::from_min_size(ui.max_rect().min + vec2(8.0, 8.0), vec2(1.0, 16.0)));
        }
        if self.clears == Some(*panel) {
            ui.ctx().output_mut(|o| o.ime = None);
        }
    }
}

fn tabs(panels: &[u32]) -> Node<u32> {
    Node::Tabs { panels: panels.to_vec(), active: 0 }
}

/// Frame `[1, 2]`, and a native window with panel 5 above panel 6.
fn layout() -> Layout<u32> {
    let split = Node::Split { axis: SplitAxis::Vertical, size: SplitSize::Ratio(0.5), first: Box::new(tabs(&[5])), second: Box::new(tabs(&[6])) };
    Layout { root: Some(tabs(&[1, 2])), floating: vec![Floating::new(split, [300.0, 80.0, 240.0, 300.0])] }
}

fn driver(content: Content) -> DockDriver<u32, Content> {
    DockDriver::new(layout(), content).with_native_host(ORIGIN)
}

fn preedit(text: &str) -> Event {
    Event::Ime(ImeEvent::Preedit { text: text.into(), active_range_chars: None })
}

/// The window's id and the descriptor its OS window published last.
fn window_ime(d: &DockDriver<u32, Content>) -> (u64, Option<IMEOutput>) {
    let id = d.layout.floating[0].id;
    let viewport = d.viewport(id);
    (id, d.host().windows.get(&viewport).and_then(|w| w.ime.clone()))
}

#[test]
#[allow(clippy::expect_used)]
fn a_composition_in_a_native_window_publishes_on_that_window() {
    let mut d = driver(Content::new(5));
    let (id, _) = window_ime(&d);
    d.window_step(id, vec![preedit("に")]);
    assert_eq!(d.content.composer.preedit().map(|p| p.text), Some("に"));
    let (_, ime) = window_ime(&d);
    let ime = ime.expect("the window publishes the editor's descriptor");
    assert_eq!(ime.owner, Id::unique("editor"));
    // At the caret, in the window's own coordinates.
    let caret = ime.cursor_rect;
    assert!(caret.min.x < 240.0 && caret.min.y < 300.0, "{caret:?}");
    assert!(d.ime().is_none(), "the main window publishes nothing");

    // Docked, the editor publishes on the main window.
    let mut d = driver(Content::new(1));
    d.step(vec![preedit("に")]);
    assert_eq!(d.ime().map(|ime| ime.owner), Some(Id::unique("editor")));
    assert!(window_ime(&d).1.is_none());
}

#[test]
#[allow(clippy::expect_used)]
fn a_body_after_the_editors_keeps_the_window_composing() {
    let mut d = driver(Content { clears: Some(6), ..Content::new(5) });
    let (id, _) = window_ime(&d);
    d.content.drawn.clear();
    d.window_step(id, vec![preedit("に")]);
    let viewport = d.viewport(id);
    assert!(d.content.drawn.ends_with(&[(5, viewport), (6, viewport)]), "{:?}", d.content.drawn);
    let first = window_ime(&d).1.expect("the editor publishes");
    for _ in 0..2 {
        d.window_step(id, Vec::new());
        let ime = window_ime(&d).1.expect("the window keeps publishing");
        assert_eq!((ime.owner, ime.context_generation), (first.owner, first.context_generation), "the same IME context");
        assert!(!ime.should_interrupt_composition);
    }
    d.window_step(id, vec![Event::Ime(ImeEvent::Commit("日".into()))]);
    assert_eq!(d.content.typed, "日");
}
