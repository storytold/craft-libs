//! The accessibility bridge: what a describer adds reaches the page view's tree.

use craft_pages::layout::LayoutOptions;
use craft_pages::ui::fixtures::{DemoPages, PAPER};
use craft_pages::ui::view::PlacedPage;
use craft_pages::ui::{AccessBuilder, AccessDescriber, AccessPosition, BlockRole, PageView, PageViewState};
use craft_ui::contract::{Role, RunDirection};
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

/// One right-to-left and one top-to-bottom line, on the first page.
struct TwoLines;

impl AccessDescriber for TwoLines {
    fn describe(&self, placed: &[PlacedPage], out: &mut AccessBuilder) {
        if !placed.iter().any(|page| page.page == 0) {
            return;
        }
        let Some(paragraph) = out.block(None, BlockRole::Paragraph) else { return };
        // Three characters, the first (logical) one rightmost: each is 10 wide, from x 150 down to 130.
        let rtl = Rect::from_min_max(egui::pos2(130.0, 20.0), egui::pos2(160.0, 34.0));
        out.line(paragraph, "אבג", &[(150.0, 10.0), (140.0, 10.0), (130.0, 10.0)], rtl, RunDirection::RightToLeft);
        // Three characters stacked, the first at the top: each is 20 tall, from y 50 down.
        let tall = Rect::from_min_max(egui::pos2(300.0, 50.0), egui::pos2(316.0, 110.0));
        out.line(paragraph, "日本語", &[(50.0, 20.0), (70.0, 20.0), (90.0, 20.0)], tall, RunDirection::TopToBottom);
    }

    fn selection(&self) -> Option<(AccessPosition, AccessPosition)> {
        None
    }
}

/// A screen reader's character positions follow the run's reading direction inside the run's own rectangle: a
/// right-to-left run says so and counts from its right edge, a vertical run says so, has a box with size and counts
/// from its top (WordCraft #7823). Before, both read as left-to-right with positions that ran backwards or zero
/// widths.
#[trial]
fn right_to_left_and_vertical_runs_keep_their_direction_and_bounds() {
    let pages = DemoPages::new(1);
    let mut state = PageViewState::default();
    let mut fixture = Fixture::new(move |ui| {
        PageView::new("Pages", &pages, &mut state, PAPER).options(LayoutOptions::ACROBAT).access(&TwoLines).show(ui, |_, _| ());
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    let tree = fixture.tree();
    let runs: Vec<_> = tree.with_role(Role::TEXT_RUN).into_iter().filter_map(|key| Some((tree.run_of(key)?, tree.bounds_of(key)?))).collect();
    let find = |direction| runs.iter().find(|((d, ..), _)| *d == direction).unwrap_or_else(|| panic!("no run reads {direction:?}: {runs:?}"));
    let ((_, positions, widths), bounds) = find(RunDirection::RightToLeft);
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "positions grow from the right edge: {positions:?}");
    assert!(widths.iter().all(|width| *width > 0.0), "every character has a width: {widths:?}");
    assert!(bounds.width() > 0.0, "the run has width: {bounds:?}");
    let reach = positions[2] + widths[2];
    assert!(reach <= bounds.width() + 0.5, "the characters stay inside the run: {reach} in {bounds:?}");
    let ((_, positions, widths), bounds) = find(RunDirection::TopToBottom);
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "positions go down the run: {positions:?}");
    assert!(widths.iter().all(|width| *width > 0.0), "every character has a height: {widths:?}");
    assert!(bounds.width() > 0.0 && bounds.height() > 0.0, "the run has size: {bounds:?}");
    assert!(positions[2] + widths[2] <= bounds.height() + 0.5, "the characters stay inside the run: {bounds:?}");
}
