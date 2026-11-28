//! The accessibility bridge: what a describer adds reaches the page view's tree.

use craft_pages::layout::LayoutOptions;
use craft_pages::ui::fixtures::{DemoPages, PAPER};
use craft_pages::ui::view::PlacedPage;
use craft_pages::ui::{AccessBuilder, AccessDescriber, AccessPosition, BlockRole, PageView, PageViewState};
use craft_ui::contract::Role;
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

/// One line of 600 characters, ten-letter words with a space after each: [`MAX_RUN`] splits it inside a word at 255
/// and after a space at 510.
struct LongLine;

const LONG_LINE_WORDS: usize = 60;

impl AccessDescriber for LongLine {
    fn describe(&self, placed: &[PlacedPage], out: &mut AccessBuilder) {
        let Some(page) = placed.iter().find(|page| page.page == 0) else { return };
        let Some(paragraph) = out.block(None, BlockRole::Paragraph) else { return };
        let text = "abcdefghi ".repeat(LONG_LINE_WORDS);
        let chars: Vec<(f32, f32)> = (0..text.chars().count()).map(|at| (page.rect.left() + at as f32, 1.0)).collect();
        let line = Rect::from_min_size(page.rect.min + vec2(0.0, 40.0), vec2(chars.len() as f32, 12.0));
        out.line(paragraph, &text, &chars, line);
    }

    fn selection(&self) -> Option<(AccessPosition, AccessPosition)> {
        None
    }
}

/// The runs a line too long for one run is split into are one line to a screen reader: each links the next and the
/// one before on the line, so a line move doesn't stop at a split, and a run that starts inside a word starts no
/// word there, so a word move doesn't either (WordCraft #7826). A run that starts after a space starts a word.
#[trial]
fn the_runs_of_a_long_line_read_as_one_line_and_split_no_word() {
    use egui::accesskit::Role as Ak;
    const { assert!(craft_pages::ui::access::MAX_RUN * 2 < LONG_LINE_WORDS * 10, "the line needs three runs") };
    let pages = DemoPages::new(1);
    let mut state = PageViewState::default();
    let mut fixture = Fixture::new(move |ui| {
        PageView::new("Pages", &pages, &mut state, PAPER).options(LayoutOptions::ACROBAT).access(&LongLine).show(ui, |_, _| ());
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    let tree = fixture.tree();
    // In reading order: the update lists nodes in no particular order.
    let mut runs: Vec<_> = tree.update().nodes.iter().filter(|(_, node)| node.role() == Ak::TextRun).collect();
    runs.sort_by(|(_, a), (_, b)| a.bounds().map(|r| r.x0).partial_cmp(&b.bounds().map(|r| r.x0)).unwrap_or(std::cmp::Ordering::Equal));
    assert_eq!(runs.len(), 3, "the line is split in three runs");
    let ((first, a), (second, b), (third, c)) = (runs[0], runs[1], runs[2]);
    assert_eq!(a.previous_on_line(), None, "the first run starts the line");
    assert_eq!((a.next_on_line(), b.previous_on_line()), (Some(*second), Some(*first)), "the first two runs are linked");
    assert_eq!((b.next_on_line(), c.previous_on_line()), (Some(*third), Some(*second)), "the last two runs are linked");
    assert_eq!(c.next_on_line(), None, "the last run ends the line");
    assert!(b.word_starts().first().is_some_and(|start| *start > 0), "a run split inside a word starts none there: {:?}", b.word_starts());
    assert_eq!(c.word_starts().first(), Some(&0), "a run after a space starts a word");
}
