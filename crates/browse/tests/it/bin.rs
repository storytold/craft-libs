//! `MediaBin` drops from another list.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::keyed;
use craft_browse::ui::bin::{BinEntry, BinItem, BinModel, BinView};
use craft_browse::ui::{BinEvent, BinState, MediaBin};
use craft_behavior::ColumnKey;
use craft_ui::contract::{Bind, ListDrag, ShowCollection as _};
use craft_ui::list::{List, ListRow};
use craft_ui::testing::{Fixture, trial};
use egui::vec2;

/// Bin 1 holds the bin 3 and the item 10; bin 2 holds the item 20.
struct Project;

impl BinModel<u32> for Project {
    fn children(&self, parent: Option<&u32>, out: &mut Vec<BinEntry<u32>>) {
        match parent {
            None => out.extend([BinEntry::Bin(1), BinEntry::Bin(2)]),
            Some(1) => out.extend([BinEntry::Bin(3), BinEntry::Item(10)]),
            Some(2) => out.push(BinEntry::Item(20)),
            _ => {}
        }
    }

    fn item(&self, key: &u32, item: &mut BinItem) {
        item.name = match key {
            1 => "Footage",
            2 => "Archive",
            3 => "Interviews",
            10 => "Beach.mov",
            _ => "Old beach.mov",
        }
        .to_owned();
    }

    fn cell(&self, _: &u32, _: &ColumnKey) -> String {
        String::new()
    }

    fn is_bin(&self, key: &u32) -> bool {
        *key < 10
    }

    fn parent(&self, key: &u32) -> Option<u32> {
        match key {
            3 | 10 => Some(1),
            20 => Some(2),
            _ => None,
        }
    }

    fn count(&self, bin: &u32) -> usize {
        usize::from(*bin != 3)
    }
}

/// The bin `into` the drop of item 20 reported, dropped from another list on the bin showing bin 1 in `view` at
/// `down` points under the Beach.mov row; `None` when it reported no drop.
fn dropped(view: BinView, down: f32) -> Option<Option<u32>> {
    let seen: Rc<RefCell<Vec<BinEvent<u32>>>> = Rc::default();
    let events = seen.clone();
    let mut state = BinState::<u32>::default();
    state.shown = Some(1);
    state.settings.view = view;
    let mut fixture = Fixture::new(move |ui| {
        let other = [20u32];
        let _ = List::<u32>::new("Other").natural_height(true).drag_payload(|drag: &ListDrag<u32>| Some(drag.clone())).show_items(
            ui,
            &keyed(&other, |n: &u32| *n),
            &mut |_, _| ListRow::new("Old beach.mov"),
        );
        events.borrow_mut().extend(MediaBin::new("Project", Bind::bound(&mut state), &Project).show(ui).events);
    });
    fixture.room(vec2(600.0, 700.0)).settle();
    let from = fixture.by_label("Old beach.mov").expect("the other list's row").center();
    let to = fixture.by_label("Beach.mov").expect("the bin's item").center() + vec2(0.0, down);
    fixture.drag_at(from, to - from).settle();
    let found = seen.borrow().iter().find_map(|event| match event {
        BinEvent::Drop { items, into } if items == &[20] => Some(*into),
        _ => None,
    });
    found
}

/// A drag from another list dropped on an item or on the empty background of the List and Icon views goes into the
/// bin shown.
#[trial]
fn a_foreign_drop_off_the_bins_goes_into_the_shown_bin() {
    for view in [BinView::List, BinView::Icon] {
        for (down, place) in [(0.0, "an item"), (160.0, "the empty background")] {
            let into = dropped(view, down);
            assert!(matches!(into, Some(None | Some(1))), "{view:?}, a drop on {place}: {into:?}");
        }
    }
}
