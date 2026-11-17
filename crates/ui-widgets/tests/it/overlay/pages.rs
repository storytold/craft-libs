//! A component popover sized from its content, whose pages switch in place.

use super::kit::{Leaf, PlainStyle, Seen};
use craft_behavior::Sense;
use craft_ui::contract::{Common, Component, Cx, MeasureCx, Overlay, OverlayUi, Overlays as _, Pages, Props as _, Role, Show as _, Sizing};
use craft_ui::style::StyleCx;
use craft_ui::testing::{Fixture, Room, trial};
use egui::{Response, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// What the trials read from the popover.
#[derive(Default)]
struct Paged {
    /// The page shown.
    page: Cell<u8>,
    /// Presses on the rows of page 0 and page 1.
    row_a: RefCell<Seen>,
    row_b: RefCell<Seen>,
    tab_a: RefCell<Seen>,
    tab_b: RefCell<Seen>,
    fill: RefCell<Seen>,
}

/// An opener whose popover shows two tabs and, under them, the page of the tab picked: page 0 a narrow row, page 1 a
/// wide row and a leaf that fills the width.
#[derive(Component)]
struct PagedOpener<'a> {
    #[common]
    common: Common,
    paged: &'a Paged,
}

impl Component for PagedOpener<'_> {
    const ROLE: Role = Role::BUTTON;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PagedOpenerStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(60.0, 20.0)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.popup("pages").open(cx);
    }

    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("pages");
        let paged = self.paged;
        ui.show(popup, Overlay::popover(), |ui, _| {
            ui.horizontal(|ui| {
                if craft_ui::contract::clicked(&Leaf::new(&paged.tab_a, Vec2::new(40.0, 20.0)).label("Tab A").show(ui)) {
                    paged.page.set(0);
                }
                if craft_ui::contract::clicked(&Leaf::new(&paged.tab_b, Vec2::new(40.0, 20.0)).label("Tab B").show(ui)) {
                    paged.page.set(1);
                }
            });
            let pages = [0_u8, 1];
            Pages::new("pages", &pages, &paged.page.get()).show(ui, |ui, page| {
                if *page == 0 {
                    Leaf::new(&paged.row_a, Vec2::new(40.0, 20.0)).label("A row").show(ui);
                } else {
                    Leaf::new(&paged.row_b, Vec2::new(160.0, 20.0)).label("B row").show(ui);
                    Leaf::new(&paged.fill, Vec2::new(40.0, 20.0)).label("Fill").sizing(Sizing::Fill).show(ui);
                }
            });
        });
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

fn fixture(paged: &Rc<Paged>) -> Fixture {
    let shown = paged.clone();
    let mut fixture = Fixture::new(move |ui| {
        PagedOpener { common: Common::new(), paged: &shown }.label("Paint").show(ui);
    });
    fixture.in_room(Room::Window);
    fixture.settle();
    fixture
}

/// The popover takes its content's size: a leaf that fills the width fills the popover, which is narrower than the
/// window, not the window.
#[trial]
fn a_content_sized_popover_is_narrower_than_the_room() {
    let paged = Rc::new(Paged::default());
    paged.page.set(1);
    let fixture = fixture(&paged);
    let fill = fixture.by_label("Fill").expect("the filling leaf shows");
    let row = fixture.by_label("B row").expect("the wide row shows");
    assert!(fill.rect.width() < Room::Window.size().x / 2.0, "the filling leaf takes the popover's width, not the window's");
    assert!(fill.rect.width() + 0.5 >= row.rect.width(), "the filling leaf fills the width the popover settled on");
}

/// A page switch moves and resizes nothing: the tabs stay where they were, and a click on the new page's row right
/// after the switch lands on it.
#[trial]
fn a_page_switch_keeps_the_popover_and_the_next_click_lands_on_the_new_page() {
    let paged = Rc::new(Paged::default());
    let mut fixture = fixture(&paged);
    let tab_b = fixture.by_label("Tab B").expect("the tabs show");
    let row_a = fixture.by_label("A row").expect("page 0 shows");
    fixture.click(&tab_b);
    assert_eq!(paged.page.get(), 1, "the tab switched the page");
    let tab_after = fixture.by_label("Tab B").expect("the tabs still show");
    assert!((tab_after.rect.min - tab_b.rect.min).length() < 0.5, "the switch did not move the tabs");
    let row_b = fixture.by_label("B row").expect("page 1 shows");
    assert!((row_b.rect.min - row_a.rect.min).length() < 0.5, "the new page starts where the old one did");
    fixture.click(&row_b);
    assert_eq!(paged.row_b.borrow().activations, 1, "the click right after the switch landed on the new page's row");
}
