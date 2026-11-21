//! The accessibility bridge: what a describer adds reaches the page view's tree.

use craft_pages::layout::LayoutOptions;
use craft_pages::ui::fixtures::{DemoPages, PAPER};
use craft_pages::ui::view::PlacedPage;
use craft_pages::ui::{AccessBuilder, AccessDescriber, AccessNode, AccessPosition, BlockRole, PageView, PageViewState};
use craft_ui::contract::Role;
use craft_ui::primitives::EditorPurpose;
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, vec2};
use std::cell::Cell;
use std::rc::Rc;

const ALT: &str = "The harbour at dusk";

/// Places one picture on the first page and records where.
struct OnePicture(Rc<Cell<Option<Rect>>>);

impl AccessDescriber for OnePicture {
    fn describe(&self, placed: &[PlacedPage], out: &mut AccessBuilder) {
        let Some(page) = placed.iter().find(|page| page.page == 0) else { return };
        let picture = Rect::from_min_size(page.rect.min + vec2(40.0, 60.0), vec2(120.0, 80.0));
        let paragraph = out.block(None, BlockRole::Paragraph);
        if out.image(paragraph, ALT, picture).is_some() {
            self.0.set(Some(picture));
        }
    }

    fn selection(&self) -> Option<(AccessPosition, AccessPosition)> {
        None
    }
}

/// A placed picture is an image node named by its alt text at its own screen rectangle, inside the paragraph that
/// holds it: a screen reader's cursor and a magnifier land on the picture, not on the page.
#[trial]
fn a_placed_picture_is_an_image_at_its_screen_rect_inside_its_paragraph() {
    let placed = Rc::new(Cell::new(None));
    let seen = Rc::clone(&placed);
    let pages = DemoPages::new(2);
    let mut state = PageViewState::default();
    let mut fixture = Fixture::new(move |ui| {
        let describer = OnePicture(Rc::clone(&seen));
        PageView::new("Pages", &pages, &mut state, PAPER).options(LayoutOptions::ACROBAT).access(&describer).show(ui, |_, _| ());
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    let picture = placed.get().expect("the describer placed the picture");
    let image = fixture.by_label(ALT).expect("a node is named by the alt text");
    assert_eq!(fixture.tree().role_of(image.node), Some(Role::IMAGE), "the alt text names an image");
    assert!(
        image.rect.min.distance(picture.min) < 1.0 && image.rect.max.distance(picture.max) < 1.0,
        "the image lies at the picture's rect: {:?} {picture:?}",
        image.rect
    );
    let paragraph = fixture.by_role(Role::PARAGRAPH);
    assert!(paragraph.iter().any(|paragraph| paragraph.rect.contains_rect(image.rect)), "the paragraph holds the picture: {paragraph:?}");
}

/// A Heading 1, a Heading 2 and a first-level list item.
struct Outline;

impl AccessDescriber for Outline {
    fn describe(&self, placed: &[PlacedPage], out: &mut AccessBuilder) {
        if !placed.iter().any(|page| page.page == 0) {
            return;
        }
        out.block(None, BlockRole::Heading(1));
        out.block(None, BlockRole::Heading(2));
        out.block(None, BlockRole::list_item(0, "•"));
    }

    fn selection(&self) -> Option<(AccessPosition, AccessPosition)> {
        None
    }
}

/// AccessKit's levels are zero-based (its `Node::set_level` doc; the Windows adapter adds one for UI Automation's
/// level), so a Heading 1 is level 0, a Heading 2 level 1 and a first-level list item level 0: a screen reader says
/// "heading level 1" for a Heading 1, not level 2 (WordCraft #3173).
#[trial]
fn headings_and_list_items_take_accesskit_zero_based_levels() {
    let pages = DemoPages::new(1);
    let mut state = PageViewState::default();
    let mut fixture = Fixture::new(move |ui| {
        PageView::new("Pages", &pages, &mut state, PAPER).options(LayoutOptions::ACROBAT).access(&Outline).show(ui, |_, _| ());
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    let levels = |role| fixture.by_role(role).iter().map(|target| fixture.tree().level_of(target.node)).collect::<Vec<_>>();
    assert_eq!(levels(Role::HEADING), [Some(0), Some(1)], "Heading 1 and Heading 2");
    assert_eq!(levels(Role::LIST_ITEM), [Some(0)], "a first-level list item");
}

/// One paragraph of one line on the first page, with the caret after its second character; it records its run.
struct OneLine(Rc<Cell<Option<AccessNode>>>);

impl AccessDescriber for OneLine {
    fn describe(&self, placed: &[PlacedPage], out: &mut AccessBuilder) {
        let Some(page) = placed.iter().find(|page| page.page == 0) else { return };
        let Some(paragraph) = out.block(None, BlockRole::Paragraph) else { return };
        let line = Rect::from_min_size(page.rect.min + vec2(40.0, 60.0), vec2(50.0, 14.0));
        let chars: Vec<(f32, f32)> = (0..5u8).map(|at| (line.min.x + 10.0 * f32::from(at), 10.0)).collect();
        self.0.set(out.line(paragraph, "Lines", &chars, line));
    }

    fn selection(&self) -> Option<(AccessPosition, AccessPosition)> {
        let caret = AccessPosition::new(self.0.get()?, 2);
        Some((caret, caret))
    }
}

/// The editing view's described pages are one document: the node that holds the key focus while the page has the
/// keyboard, with the text under it and the caret as its text selection, so a screen reader reads the line at the
/// caret. Before, the page was a canvas (an image to UI Automation) holding the focus, and the text and the caret sat
/// in a group beside it (storytold/wordcraft#7149, item 9).
#[trial]
fn the_editing_view_is_a_document_with_the_focus_and_the_caret() {
    let run = Rc::new(Cell::new(None));
    let seen = Rc::clone(&run);
    let pages = DemoPages::new(1);
    let mut state = PageViewState::default();
    let mut fixture = Fixture::new(move |ui| {
        let describer = OneLine(Rc::clone(&seen));
        PageView::new("Document", &pages, &mut state, PAPER)
            .options(LayoutOptions::ACROBAT)
            .editor(EditorPurpose::Text)
            .access(&describer)
            .show(ui, |_, _| ());
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    let document = fixture.by_label("Document").expect("the page view's node");
    fixture.click(&document);
    fixture.settle();
    let tree = fixture.tree();
    assert_eq!(tree.role_of(document.node), Some(Role::DOCUMENT), "the described pages are a document");
    assert_eq!(tree.focused_key(), document.node, "the document holds the key focus while the page has the keyboard");
    let runs = tree.with_role(Role::TEXT_RUN);
    let line = *runs.first().expect("the line's run");
    let paragraph = tree.parent_of_key(line).expect("the run's paragraph");
    assert_eq!(tree.parent_of_key(paragraph), Some(document.node), "the text hangs under the document");
    let ((anchor, _), (focus, at)) = tree.text_selection_of(document.node).expect("the document's text selection");
    assert_eq!((anchor, focus), (line, line), "the caret is in the line's run");
    assert!(at > 0, "the caret is inside the run: {at}");
}
