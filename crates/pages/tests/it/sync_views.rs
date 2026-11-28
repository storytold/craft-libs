//! The synced pair: the caption band over each half and the room left to its view.

use craft_i18n::Direction;
use craft_pages::ui::fixtures::{DemoPages, PAPER};
use craft_pages::ui::{Lead, PageView, PageViewState, SyncedViews, SyncedViewsState};
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// The view rects each half's closure got, left then right.
type Views = Rc<RefCell<[Option<Rect>; 2]>>;

/// A captioned pair over two demo documents in `direction`, settled.
fn captioned_pair(direction: Direction) -> (Fixture, Views) {
    let views: Views = Rc::default();
    let seen = Rc::clone(&views);
    let pages = DemoPages::new(3);
    let mut pair = SyncedViewsState::default();
    let (mut left, mut right) = (PageViewState::default(), PageViewState::default());
    let mut fixture = Fixture::new(move |ui| {
        let mut rects = [None; 2];
        SyncedViews::new("Compare", &mut pair).captions("Old", "New").halves(ui, (&mut left, &mut right), |ui, rect, state, half| {
            rects[usize::from(half == Lead::Right)] = Some(rect);
            let label = if half == Lead::Left { "Old pages" } else { "New pages" };
            Some(PageView::new(label, &pages, state, PAPER).show_at(ui, rect, |_, _| ()))
        });
        *seen.borrow_mut() = rects;
    });
    fixture.room(vec2(900.0, 600.0)).direction(direction).settle();
    (fixture, views)
}

#[trial]
fn captions_sit_over_each_half_and_the_view_takes_the_rest() {
    for direction in [Direction::Ltr, Direction::Rtl] {
        let rtl = matches!(direction, Direction::Rtl);
        let (fixture, views) = captioned_pair(direction);
        let room = fixture.shown();
        let shown = *views.borrow();
        let (Some(old_view), Some(new_view)) = (shown[0], shown[1]) else { panic!("both halves drew ({direction:?})") };
        let old = fixture.by_label("Old").expect("the left half's caption").rect;
        let new = fixture.by_label("New").expect("the right half's caption").rect;

        // The left half reads first: on the left, or on the right in a right-to-left interface.
        let ((first, first_view), (second, second_view)) = if rtl { ((new, new_view), (old, old_view)) } else { ((old, old_view), (new, new_view)) };
        assert!(
            (first_view.max.x - second_view.min.x).abs() < 0.5,
            "the divider sits at the boundary of the two contiguous views ({direction:?}): {first_view:?} {second_view:?}"
        );
        assert!(first.max.x <= second_view.min.x && second.min.x >= first_view.max.x, "the captions lie either side of the divider ({direction:?})");

        for (name, caption, view) in [("Old", old, old_view), ("New", new, new_view)] {
            assert!(caption.min.x >= view.min.x - 0.5 && caption.max.x <= view.max.x + 0.5, "{name}'s caption stays over its half ({direction:?})");
            assert!(caption.min.y >= room.min.y && caption.max.y <= view.min.y, "{name}'s caption sits above its view, clear of it ({direction:?})");
            let band = view.min.y - room.min.y;
            assert!(band > caption.height(), "{name}'s band pads its caption ({direction:?}): band {band}, caption {caption:?}");
            assert!(
                (band + view.height() - room.height()).abs() < 0.5,
                "{name}'s band and view share the room ({direction:?}): {view:?} in {room:?}"
            );
        }
    }
}
