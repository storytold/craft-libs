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
