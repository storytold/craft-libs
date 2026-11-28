//! Shift+Cmd on a long range looks the selected keys up in a set: the comparisons it makes grow with the rows, not
//! with the rows times the selected keys.

use super::kit::{Dismissed, ROW_H, Row};
use craft_behavior::{DisplayOrder, ItemSource, Selection, SpanLayout};
use craft_ui::contract::{
    Bind, BindMut, Collection, CollectionProps, CollectionResponse, Common, Component, ContainerProps, Cx, Role, SelectMode, ShowCollection as _,
};
use craft_ui::style::StyleCx;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, pointer, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};

/// How many times two keys were compared.
static COMPARED: AtomicUsize = AtomicUsize::new(0);

/// A row key that counts its comparisons.
#[derive(Clone, Copy, Debug, Eq)]
struct Counted(usize);

impl PartialEq for Counted {
    fn eq(&self, other: &Self) -> bool {
        COMPARED.fetch_add(1, Ordering::Relaxed);
        self.0 == other.0
    }
}

impl Hash for Counted {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

/// `len` rows keyed `Counted(0)` up.
struct Rows(usize);

impl DisplayOrder<Counted> for Rows {
    fn len(&self) -> usize {
        self.0
    }

    fn key(&self, at: usize) -> Counted {
        Counted(at)
    }

    fn index_of(&self, key: &Counted) -> Option<usize> {
        (key.0 < self.0).then_some(key.0)
    }
}

impl ItemSource<Counted> for Rows {}

/// A multi-select list of `Counted` rows at `ROW_H`.
#[derive(craft_ui::contract::Component)]
#[component(collection)]
struct Keyed<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
    selection: Bind<'a, Selection<Counted>>,
}

impl Component for Keyed<'_> {
    const ROLE: Role = Role::LIST;
    type Style = PlainStyle;
    type Output = CollectionResponse<Counted, Dismissed>;

    fn style(_: &StyleCx<'_>, _: KeyedStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl Collection for Keyed<'_> {
    type Key = Counted;
    type ItemLayout = SpanLayout;
    type Selection = Selection<Counted>;
    type Item<'r> = Row;
    type ItemEvent = Dismissed;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, Selection<Counted>>> {
        Some(self.selection.reborrow())
    }

    fn item_layout(&self, source: &dyn ItemSource<Counted>, width: f32, _: &StyleCx<'_>) -> SpanLayout {
        SpanLayout::new(source, width, ROW_H)
    }
}

const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);
const VIEW: Vec2 = Vec2::new(300.0, 200.0);
const ROWS: usize = 5_000;

/// 2,000 rows are selected with the anchor on row 4,000, and Shift+Cmd on row 0 adds the 4,001 rows from the anchor.
/// Each row searching the selected keys makes about 6 million comparisons; a set makes few, and the frames' own reads
/// of the selection (the ten rows in view against the selected keys) stay well under the bound.
#[trial]
fn shift_cmd_on_a_long_range_looks_the_selected_keys_up_in_a_set() {
    let rows = Rows(ROWS);
    let selection = RefCell::new(Selection::new((2_000..=4_000).map(Counted).collect(), Some(Counted(4_000)), Some(Counted(4_000))));
    let first = RefCell::new(Rect::NOTHING);
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let mut held = selection.borrow().clone();
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            let list = Keyed {
                common: Common::new(),
                props: ContainerProps::new().sizing(craft_ui::contract::Sizing::Fill),
                items: CollectionProps::new().select(SelectMode::Multi),
                selection: Bind::bound(&mut held),
            };
            list.show_items(ui, &rows, &mut |key, cx| {
                if key.0 == 0 {
                    *first.borrow_mut() = cx.rect;
                }
                Row::new()
            });
        });
        *selection.borrow_mut() = held;
    };
    frames.idle(3, &mut show);
    let at = first.borrow().center();
    COMPARED.store(0, Ordering::Relaxed);
    frames.hold(Modifiers::COMMAND | Modifiers::SHIFT);
    pointer::click(&mut frames, at, &mut show);
    frames.hold(Modifiers::NONE);
    let compared = COMPARED.load(Ordering::Relaxed);
    let mut items: Vec<usize> = selection.borrow().items.iter().map(|key| key.0).collect();
    items.sort_unstable();
    assert_eq!(items, (0..=4_000).collect::<Vec<_>>(), "Shift+Cmd adds the rows from the anchor to row 0");
    assert!(compared < 1_000_000, "the range looks the selected keys up in a set: {compared} comparisons");
}
