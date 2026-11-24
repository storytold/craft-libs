//! The row trials: rows with focusable children, cell selection in a sheet, rows of measured heights and a lead that
//! leaves the source.

use super::kit::{Count, Dismissed, List, ROW_H, RowOut, Words};
use craft_behavior::{
    CancelReason, CellAt, CellRange, CellSelection, ColumnKey, EditEvent, EditEvents, GestureId, ItemSource, Mixed, PartKey, ScrollPos, Selection,
    SelectionModel as _, Sense, SheetLayout, SpanLayout, keyed,
};
use craft_keymap::Mods;
use craft_ui::contract::{
    Bind, BindMut, Collection, CollectionEvent, CollectionProps, CollectionResponse, Common, Component, ContainerProps, Cx, EditResponse, Focus as _,
    MeasureCx, PartsCx, Props as _, Role, SelectMode, ShowCollection as _, request_rename,
};
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::style::StyleCx;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, input, pointer, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Id, Key, Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

/// Places that differ by less are the same place: the rounding of the layout.
const SAME: f32 = 0.5;

const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);
const VIEW: Vec2 = Vec2::new(300.0, 200.0);
const CELL_W: f32 = 50.0;
const TALL: f32 = 3.0;
const CHILD_A: PartKey = PartKey::new("a");
const CHILD_B: PartKey = PartKey::new("b");

/// What the trials' rows and collections saw.
#[derive(Default)]
struct Log {
    /// The row and part that held focus in the last pass that had one.
    focus: RefCell<Vec<(usize, &'static str)>>,
    rects: RefCell<Vec<(usize, Rect)>>,
    events: RefCell<Vec<CollectionEvent<usize, Dismissed>>>,
    id: Cell<Option<Id>>,
}

impl Log {
    fn new() -> &'static Log {
        Box::leak(Box::default())
    }

    fn rect(&self, key: usize) -> Rect {
        self.rects.borrow().iter().rev().find(|(row, _)| *row == key).map(|(_, rect)| *rect).expect("the row ran")
    }

    fn leads(&self) -> Vec<usize> {
        self.events.borrow().iter().filter_map(|event| if let CollectionEvent::LeadMoved { key, .. } = event { Some(*key) } else { None }).collect()
    }

    fn take(&self, out: CollectionResponse<usize, Dismissed>) {
        self.id.set(Some(out.response.id));
        self.events.borrow_mut().extend(out.events);
    }

    fn id(&self) -> Id {
        self.id.get().expect("the list ran")
    }
}

/// A row that records its rect and focus, with two focusable parts when it has children and a tall measure for
/// the key 1 of an auto-height list.
#[derive(craft_ui::contract::Component)]
struct ProbeRow {
    #[common]
    common: Common,
    key: usize,
    children: bool,
    tall: bool,
    log: &'static Log,
}

impl ProbeRow {
    fn new(key: usize, log: &'static Log) -> Self {
        ProbeRow { common: Common::new(), key, children: false, tall: false, log }
    }

    fn children(mut self) -> Self {
        self.children = true;
        self
    }

    fn tall_when(mut self, tall: bool) -> Self {
        self.tall = tall;
        self
    }
}

impl Component for ProbeRow {
    const ROLE: Role = Role::LIST_ITEM;
    type Style = PlainStyle;
    type Output = RowOut;

    fn style(_: &StyleCx<'_>, _: ProbeRowStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(10.0, if self.tall { ROW_H * TALL } else { ROW_H })
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        if self.children {
            let half = Vec2::new(rect.width() / 2.0, rect.height());
            cx.part(CHILD_A, Rect::from_min_size(rect.min, half)).sense(Sense::CLICK).focusable();
            cx.part(CHILD_B, Rect::from_min_size(rect.min + Vec2::new(half.x, 0.0), half)).sense(Sense::CLICK).focusable();
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.log.rects.borrow_mut().push((self.key, cx.rect));
        if cx.is_focused() {
            self.log.focus.borrow_mut().push((self.key, cx.active_part().map_or("row", |part| part.key.name())));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> RowOut {
        RowOut { response: cx.response.clone(), event: None }
    }
}

/// What the stub rename editor reports for a typed Enter: the key, the text typed, the modifiers held with it, and how
/// often the editor was shown.
struct Typing {
    enter: Cell<bool>,
    text: Cell<Option<String>>,
    mods: Cell<Mods>,
    shown: Cell<usize>,
}

impl Typing {
    fn new() -> Self {
        Typing { enter: Cell::new(false), text: Cell::new(None), mods: Cell::new(Mods::NONE), shown: Cell::new(0) }
    }

    /// The next pass sees one Enter with `text` typed (none: the proposed draft), with Shift held when `shift`.
    fn press(&self, text: Option<&str>, shift: bool) {
        self.text.set(text.map(str::to_owned));
        self.mods.set(if shift { Mods::SHIFT } else { Mods::NONE });
        self.enter.set(true);
    }
}

/// A list of `ProbeRow`s over `SpanLayout`, rows of the control height or of the heights they measure.
#[derive(craft_ui::contract::Component)]
#[component(collection)]
struct Probe<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
    selection: Option<Bind<'a, CellSelection<usize>>>,
    scroll: Option<Bind<'a, ScrollPos>>,
    auto: bool,
    /// The rename draft every row proposes, and whether the rename field reports Enter.
    rename: Option<(&'a str, &'a Cell<bool>)>,
    /// The typed Enter the rename field reports, one per `Typing::press`.
    typing: Option<&'a Typing>,
}

impl<'a> Probe<'a> {
    fn new(select: SelectMode) -> Self {
        Probe {
            common: Common::new(),
            props: ContainerProps::new().sizing(craft_ui::contract::Sizing::Fill),
            items: CollectionProps::new().select(select),
            selection: None,
            scroll: None,
            auto: false,
            rename: None,
            typing: None,
        }
    }

    /// Rows rename with `draft` proposed; the field reports Enter while `enter` is set.
    fn renaming(mut self, draft: &'a str, enter: &'a Cell<bool>) -> Self {
        self.items = self.items.rename(true);
        self.rename = Some((draft, enter));
        self
    }

    /// Rows rename with `draft` proposed; the field reports the Enters `typing` is given, each once.
    fn typing(mut self, draft: &'a str, typing: &'a Typing) -> Self {
        self = self.renaming(draft, &typing.enter);
        self.typing = Some(typing);
        self
    }

    fn auto(mut self) -> Self {
        self.auto = true;
        self
    }

    fn scrolled(mut self, scroll: Bind<'a, ScrollPos>) -> Self {
        self.scroll = Some(scroll);
        self
    }
}

impl Component for Probe<'_> {
    const ROLE: Role = Role::LIST;
    type Style = PlainStyle;
    type Output = CollectionResponse<usize, Dismissed>;

    fn style(_: &StyleCx<'_>, _: ProbeStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn scroll(&mut self) -> Option<BindMut<'_, ScrollPos>> {
        self.scroll.as_mut().map(Bind::reborrow)
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl Collection for Probe<'_> {
    type Key = usize;
    type ItemLayout = SpanLayout;
    type Selection = CellSelection<usize>;
    type Item<'r> = ProbeRow;
    type ItemEvent = Dismissed;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, CellSelection<usize>>> {
        self.selection.as_mut().map(Bind::reborrow)
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, width: f32, _: &StyleCx<'_>) -> SpanLayout {
        if self.auto { SpanLayout::sized(source, width, ROW_H) } else { SpanLayout::new(source, width, ROW_H) }
    }

    fn rename_draft(&self, _: &usize) -> Option<String> {
        self.rename.map(|(draft, _)| draft.to_owned())
    }

    fn rename_editor(&mut self, ui: &mut Ui, _: Bind<'_, String>, label: Rect, _: &str) -> Option<EditResponse<String>> {
        let (draft, enter) = self.rename?;
        let mut events = EditEvents::new();
        let (submitted, mods) = match self.typing {
            Some(typing) => {
                typing.shown.set(typing.shown.get() + 1);
                let pressed = enter.replace(false);
                if pressed && let Some(text) = typing.text.take() {
                    events.push(EditEvent::commit(GestureId::new(0), Mixed::Same(draft.to_owned()), text));
                }
                (pressed, typing.mods.get())
            }
            None => (enter.get(), Mods::NONE),
        };
        let mut editor = EditResponse::lifted(ui.allocate_rect(label, egui::Sense::hover()), events, None);
        // The probe field is no focus stop, so it asks for the focus a real text field keeps: a field that holds the focus
        // and then loses it ends the rename.
        request(ui.ctx(), FocusTarget::Id(editor.response.id));
        editor.submitted = submitted;
        editor.submit_mods = mods;
        Some(editor)
    }
}

/// A sheet of `CELL_W` columns over `SheetLayout`, with the first column frozen.
#[derive(craft_ui::contract::Component)]
#[component(collection)]
struct Sheet<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
    selection: Option<Bind<'a, CellSelection<usize>>>,
}

const COLUMNS: [&str; 6] = ["a", "b", "c", "d", "e", "f"];

impl<'a> Sheet<'a> {
    fn new() -> Self {
        Sheet {
            common: Common::new(),
            props: ContainerProps::new().sizing(craft_ui::contract::Sizing::Fill),
            items: CollectionProps::new().select(SelectMode::Cells),
            selection: None,
        }
    }

    fn selection(mut self, selection: Bind<'a, CellSelection<usize>>) -> Self {
        self.selection = Some(selection);
        self
    }
}

impl Component for Sheet<'_> {
    const ROLE: Role = Role::LIST;
    type Style = PlainStyle;
    type Output = CollectionResponse<usize, Dismissed>;

    fn style(_: &StyleCx<'_>, _: SheetStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl Collection for Sheet<'_> {
    type Key = usize;
    type ItemLayout = SheetLayout;
    type Selection = CellSelection<usize>;
    type Item<'r> = ProbeRow;
    type ItemEvent = Dismissed;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, CellSelection<usize>>> {
        self.selection.as_mut().map(Bind::reborrow)
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, _: f32, _: &StyleCx<'_>) -> SheetLayout {
        let columns = COLUMNS.iter().map(|key| (ColumnKey::new(key), CELL_W)).collect();
        SheetLayout::new(source, ROW_H, columns).frozen(1)
    }
}

fn press(frames: &mut Frames, key: Key, mods: Modifiers, show: &mut impl FnMut(&mut Ui)) {
    frames.frame(vec![input::key_press(key, mods)], &mut *show);
    frames.frame(vec![craft_ui::testing::focus::key_release(key, mods)], &mut *show);
}

/// A key press and release with the focus record cleared first, so it holds only what the key's frames saw.
fn tap(frames: &mut Frames, log: &Log, key: Key, show: &mut impl FnMut(&mut Ui)) {
    log.focus.borrow_mut().clear();
    press(frames, key, Modifiers::NONE, show);
}

fn focus_list(frames: &mut Frames, log: &Log, show: &mut impl FnMut(&mut Ui)) {
    request(&frames.ctx, FocusTarget::Id(log.id()));
    frames.idle(2, &mut *show);
}

/// 17.5: Forward enters a row's first child and moves on through them, Back returns to the row from the first,
/// Up, Down and Escape leave the children for the row, and the collection stays the one egui focus.
#[trial]
fn forward_enters_a_rows_children_and_arrows_and_escape_leave_them() {
    let (log, source) = (Log::new(), Count(4));
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            let list = Probe::new(SelectMode::Single).id_salt("list");
            log.take(list.show_items(ui, &source, &mut |key, _| ProbeRow::new(*key, log).children()));
        });
    };
    frames.idle(3, &mut show);
    focus_list(&mut frames, log, &mut show);
    let last_focus = || log.focus.borrow().last().copied();

    tap(&mut frames, log, Key::ArrowDown, &mut show);
    assert_eq!(log.leads().last(), Some(&0));
    assert_eq!(last_focus(), None, "the lead row alone holds focus until Forward");
    tap(&mut frames, log, Key::ArrowRight, &mut show);
    assert_eq!(last_focus(), Some((0, "a")), "Forward enters the row's first child");
    assert_eq!(craft_ui::testing::focus::egui_focused(&frames.ctx), Some(log.id()), "the children are members of the collection's arena");
    tap(&mut frames, log, Key::ArrowRight, &mut show);
    assert_eq!(last_focus(), Some((0, "b")));
    tap(&mut frames, log, Key::ArrowLeft, &mut show);
    assert_eq!(last_focus(), Some((0, "a")));
    tap(&mut frames, log, Key::ArrowLeft, &mut show);
    assert_eq!(last_focus(), None, "Back from the first child returns to the row");
    assert_eq!(log.leads().last(), Some(&0));

    tap(&mut frames, log, Key::ArrowRight, &mut show);
    tap(&mut frames, log, Key::ArrowDown, &mut show);
    assert_eq!((last_focus(), log.leads().last().copied()), (None, Some(1)), "Down leaves the children and moves the lead");
    tap(&mut frames, log, Key::ArrowRight, &mut show);
    assert_eq!(last_focus(), Some((1, "a")));
    tap(&mut frames, log, Key::Escape, &mut show);
    assert_eq!((last_focus(), log.leads().last().copied()), (None, Some(1)), "Escape returns to the row and keeps the lead");
}

/// 17.7: the arrows move the lead cell, Shift extends the range from the anchor cell, and the selection reports
/// the rows and the columns, with the active cell at the anchor. The lead stays with the active cell while the range
/// extends, and a plain arrow then steps from the active cell (Excel), here against the sheet's first column.
#[trial]
fn shift_arrows_extend_a_cell_selection_in_a_sheet() {
    let (log, source) = (Log::new(), Count(5));
    let selection = RefCell::new(CellSelection::<usize>::default());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let mut held = selection.borrow().clone();
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            log.take(Sheet::new().selection(Bind::bound(&mut held)).id_salt("sheet").show_items(ui, &source, &mut |key, _| ProbeRow::new(*key, log)));
        });
        *selection.borrow_mut() = held;
    };
    frames.idle(3, &mut show);
    focus_list(&mut frames, log, &mut show);
    press(&mut frames, Key::ArrowDown, Modifiers::NONE, &mut show);
    press(&mut frames, Key::ArrowRight, Modifiers::SHIFT, &mut show);
    press(&mut frames, Key::ArrowRight, Modifiers::SHIFT, &mut show);
    press(&mut frames, Key::ArrowDown, Modifiers::SHIFT, &mut show);
    let held = selection.borrow().clone();
    assert_eq!(held.ranges(), [CellRange::cells(CellAt::new(0, 0), CellAt::new(1, 2))], "the rows and columns between the anchor cell and the head");
    assert_eq!((held.lead(), held.lead_column()), (Some(&0), Some(0)), "the active cell stays at the anchor");
    press(&mut frames, Key::ArrowLeft, Modifiers::NONE, &mut show);
    let held = selection.borrow().clone();
    assert_eq!(
        (held.ranges(), held.lead(), held.lead_column()),
        ([CellRange::cells(CellAt::new(0, 0), CellAt::new(0, 0))].as_slice(), Some(&0), Some(0)),
        "a plain arrow after an extend steps from the active cell, and the lead stays with it"
    );
}

/// 17.3, I1: rows of different measured heights place one under the other and a press finds the row under it.
#[trial]
fn rows_of_different_measured_heights_place_and_hit_correctly() {
    let (log, source) = (Log::new(), Count(6));
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            let list = Probe::new(SelectMode::Single).auto().id_salt("list");
            log.take(list.show_items(ui, &source, &mut |key, _| ProbeRow::new(*key, log).tall_when(*key == 1)));
        });
    };
    frames.idle(4, &mut show);
    assert_eq!(log.rect(1).height(), ROW_H * TALL, "an auto row takes the height it measures");
    assert_eq!((log.rect(0).height(), log.rect(2).min.y), (ROW_H, log.rect(1).max.y), "the rows after it move down");
    let inside = |key: usize, fraction: f32| log.rect(key).min + Vec2::new(5.0, log.rect(key).height() * fraction);
    pointer::click(&mut frames, inside(1, 0.9), &mut show);
    assert_eq!(log.leads().last(), Some(&1), "the lower part of the tall row is the tall row");
    pointer::click(&mut frames, inside(2, 0.5), &mut show);
    assert_eq!(log.leads().last(), Some(&2));
}

/// 17.3 step 9: when the lead's key leaves the source (deleted, or hidden by a filter), the lead moves to the key now
/// in its place and the selection stays the host's: the list selects no row in place of the one that left.
#[trial]
fn a_selected_lead_that_leaves_the_source_moves_the_lead_and_leaves_the_selection() {
    let rows = RefCell::new(vec![(10_usize, "a"), (11, "b"), (12, "c"), (13, "d")]);
    let selection = RefCell::new(Selection::<usize>::default());
    let seen = Cell::new(None);
    let events = RefCell::new(Vec::new());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let mut held = selection.borrow().clone();
        let rows = rows.borrow();
        let source = keyed(&rows, |row| row.0);
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            let out = List::new(SelectMode::Single)
                .selection(Bind::bound(&mut held))
                .id_salt("list")
                .show_items(ui, &source, &mut |_, _| super::kit::Row::new());
            seen.set(Some(out.response.id));
            events.borrow_mut().extend(out.events);
        });
        *selection.borrow_mut() = held;
    };
    frames.idle(3, &mut show);
    pointer::click(&mut frames, ORIGIN + Vec2::new(5.0, ROW_H * 2.5), &mut show);
    assert_eq!(selection.borrow().items, vec![12]);
    events.borrow_mut().clear();
    rows.borrow_mut().remove(2);
    frames.idle(2, &mut show);
    assert_eq!(selection.borrow().items, vec![12], "the selection is the host's: no neighbour is picked");
    assert!(!events.borrow().iter().any(|event| matches!(event, CollectionEvent::Selection { .. })), "no selection change is reported");
    let leads: Vec<usize> =
        events.borrow().iter().filter_map(|event| if let CollectionEvent::LeadMoved { key, .. } = event { Some(*key) } else { None }).collect();
    assert_eq!(leads.last(), Some(&13), "the lead moves to the key now in its place");
}

/// A lead whose key leaves the source moves to the host's current item without a report: the host is there already
/// (an Undo that removes the current page, whose host has picked another).
#[trial]
fn a_lead_that_leaves_the_source_takes_the_hosts_current_item_unreported() {
    let rows = RefCell::new(vec![10_usize, 11, 12, 13]);
    let current = Cell::new(12);
    let events = RefCell::new(Vec::new());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let rows = rows.borrow();
        let source = keyed(&rows, |row| *row);
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            let out = List::new(SelectMode::Single)
                .arrows_move_only()
                .current(current.get())
                .id_salt("list")
                .show_items(ui, &source, &mut |_, _| super::kit::Row::new());
            events.borrow_mut().extend(out.events);
        });
    };
    frames.idle(3, &mut show);
    pointer::click(&mut frames, ORIGIN + Vec2::new(5.0, ROW_H * 2.5), &mut show);
    let lead = |events: &RefCell<Vec<CollectionEvent<usize, Dismissed>>>| {
        events.borrow().iter().rev().find_map(|event| if let CollectionEvent::LeadMoved { key, .. } = event { Some(*key) } else { None })
    };
    assert_eq!(lead(&events), Some(12), "the press made the current row the lead");
    events.borrow_mut().clear();
    rows.borrow_mut().retain(|row| *row != 12);
    current.set(10);
    frames.idle(2, &mut show);
    assert_eq!(lead(&events), None, "the host's own move is not reported back");
    press(&mut frames, Key::ArrowDown, Modifiers::NONE, &mut show);
    assert_eq!(lead(&events), Some(11), "the lead stood on the current item: Down steps to the row after it");
}

/// 17.3, I1: the lead row above the view grows (an auto row it measures taller): the rows in view stay where they
/// were, in the frame it grows, and the frame after runs no extra pass. Guards the collection's scroll anchor: its
/// position follows the first row at or after the view's start.
#[trial]
fn rows_in_view_stay_when_a_row_above_them_changes_height() {
    let (log, source) = (Log::new(), Count(60));
    let tall = Cell::new(None);
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            let list = Probe::new(SelectMode::Single).auto().id_salt("list");
            log.take(list.show_items(ui, &source, &mut |key, _| ProbeRow::new(*key, log).tall_when(tall.get() == Some(*key))));
        });
    };
    frames.idle(3, &mut show);
    pointer::click(&mut frames, ORIGIN + Vec2::new(5.0, ROW_H * 2.5), &mut show);
    assert_eq!(log.leads().last(), Some(&2), "the press made row 2 the lead");
    let over = ORIGIN + VIEW / 2.0;
    for _ in 0..4 {
        frames.frame(vec![pointer::moved(over)], &mut show);
        frames.frame(vec![pointer::wheel(-VIEW.y)], &mut show);
    }
    frames.idle(2, &mut show);
    let view = Rect::from_min_size(ORIGIN, VIEW);
    let in_view =
        |rects: &[(usize, Rect)]| -> Vec<(usize, Rect)> { rects.iter().copied().filter(|(key, rect)| *key != 2 && rect.intersects(view)).collect() };
    log.rects.borrow_mut().clear();
    let idle = frames.frame(vec![], &mut show);
    let before = in_view(&log.rects.borrow());
    assert!(log.rect(2).max.y < view.min.y, "the lead row is above the view");
    assert!(!before.is_empty(), "rows show in the view");
    tall.set(Some(2));
    log.rects.borrow_mut().clear();
    let grown = frames.frame(vec![], &mut show);
    assert!(log.rect(2).height() > ROW_H, "the lead row grew");
    let after = in_view(&log.rects.borrow());
    for (key, rect) in &before {
        let now =
            after.iter().rev().find(|(shown, _)| shown == key).map(|(_, rect)| *rect).unwrap_or_else(|| panic!("row {key} in view was built again"));
        assert!((now.min - rect.min).length() <= SAME, "row {key} in view stayed: {now:?}, then {rect:?}");
    }
    let next = frames.frame(vec![], &mut show);
    assert!(grown > idle, "the frame the row grew in ran the pass again at the anchored offset");
    assert_eq!(next, idle, "an idle frame after it runs no extra pass");
}

/// A field hosted in a row asks its scroll owner, the collection, to reveal row 25, out of view: the collection takes
/// the reveal in its next pass against that pass's rows, shows the row and scrolls once. Guards the reveal being
/// taken and dropped for want of a view.
#[trial]
fn a_reveal_from_a_hosted_field_scrolls_its_collection_in_the_same_pass() {
    let (log, source) = (Log::new(), Count(30));
    let scroll = Cell::new(ScrollPos::default());
    let offsets = RefCell::new(Vec::new());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let mut held = scroll.get();
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            let list = Probe::new(SelectMode::Single).scrolled(Bind::bound(&mut held)).id_salt("list");
            log.take(list.show_items(ui, &source, &mut |key, _| ProbeRow::new(*key, log)));
        });
        scroll.set(held);
        offsets.borrow_mut().push(held.y);
    };
    frames.idle(3, &mut show);
    let view = Rect::from_min_size(ORIGIN, VIEW);
    let first = log.rect(0);
    let row = Rect::from_min_size(first.min + Vec2::new(0.0, 25.0 * ROW_H), first.size());
    assert!(row.min.y >= view.max.y, "row 25 starts out of view");
    let before = scroll.get().y;
    offsets.borrow_mut().clear();
    log.rects.borrow_mut().clear();
    craft_ui::testing::contract::queue_reveal(&frames.ctx, log.id(), row);
    frames.frame(vec![], &mut show);
    let shown = log.rect(25);
    assert!(shown.min.y >= view.min.y - SAME && shown.max.y <= view.max.y + SAME, "row 25 lies in the view in the frame after the reveal: {shown:?}");
    frames.idle(2, &mut show);
    let held: Vec<f64> = std::iter::once(before).chain(offsets.borrow().iter().copied()).collect();
    let changes = held.windows(2).filter(|pair| (pair[0] - pair[1]).abs() > f64::EPSILON).count();
    assert_eq!(changes, 1, "the collection scrolls once for the reveal: {held:?}");
}

/// A reveal of a rect the view already shows moves nothing, so it asks for no second pass: a part that asks every frame
/// (a focused row that keeps itself in view) must not make every frame run twice (egui's "request_discard has been
/// called N frames in a row").
#[trial]
fn a_reveal_of_a_rect_in_view_runs_no_second_pass() {
    let (log, source) = (Log::new(), Count(30));
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            let list = Probe::new(SelectMode::Single).id_salt("list");
            log.take(list.show_items(ui, &source, &mut |key, _| ProbeRow::new(*key, log)));
        });
    };
    frames.idle(3, &mut show);
    let idle = frames.frame(vec![], &mut show);
    let row = log.rect(1);
    for _ in 0..3 {
        craft_ui::testing::contract::queue_reveal(&frames.ctx, log.id(), row);
        let passes = frames.frame(vec![], &mut show);
        assert_eq!(passes, idle, "a reveal of a row in view runs no extra pass");
    }
}

/// Enter on a rename field nobody typed in commits the draft the collection proposed, and ends with no event when the
/// draft is the row's own text.
#[trial]
fn enter_on_an_untyped_rename_commits_only_a_proposed_draft() {
    let renamed = |draft: &str| {
        let (log, enter, words) = (Log::new(), Cell::new(false), Words::flat(&["a", "b", "c"]));
        let mut frames = Frames::new();
        let mut show = |ui: &mut Ui| {
            in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
                let list = Probe::new(SelectMode::Single).id_salt("list").renaming(draft, &enter);
                log.take(list.show_items(ui, &words, &mut |key, _| ProbeRow::new(*key, log)));
            });
        };
        frames.idle(3, &mut show);
        request_rename(&frames.ctx, log.id(), &1_usize);
        frames.idle(3, &mut show);
        enter.set(true);
        frames.idle(2, &mut show);
        let commits: Vec<String> = log
            .events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                CollectionEvent::Rename(EditEvent::Commit { after, .. }) => Some(after.name.clone()),
                _ => None,
            })
            .collect();
        commits
    };
    assert_eq!(renamed("proposed"), ["proposed"], "Enter accepts the proposed draft");
    assert!(renamed("b").is_empty(), "a draft equal to the row's text reports no commit");
}

/// Shift+Enter in a rename field applies the name as a commit and the rename goes on, each Shift+Enter its own gesture;
/// plain Enter commits and ends it. The proposed draft does the same with Shift.
#[trial]
fn shift_enter_applies_a_rename_and_keeps_the_field_open() {
    let (log, typing, words) = (Log::new(), Typing::new(), Words::flat(&["a", "b", "c"]));
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            let list = Probe::new(SelectMode::Single).id_salt("list").typing("proposed", &typing);
            log.take(list.show_items(ui, &words, &mut |key, _| ProbeRow::new(*key, log)));
        });
    };
    let commits = || -> Vec<(GestureId, String)> {
        log.events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                CollectionEvent::Rename(EditEvent::Commit { id, after, .. }) => Some((*id, after.name.clone())),
                _ => None,
            })
            .collect()
    };
    frames.idle(3, &mut show);
    request_rename(&frames.ctx, log.id(), &1_usize);
    frames.idle(3, &mut show);
    assert!(typing.shown.get() > 0, "the rename shows its editor");

    typing.press(Some("one"), true);
    frames.frame(vec![], &mut show);
    assert_eq!(commits().iter().map(|(_, name)| name.as_str()).collect::<Vec<_>>(), ["one"], "Shift+Enter reports the typed name once");
    let shown = typing.shown.get();
    frames.idle(2, &mut show);
    assert!(typing.shown.get() > shown, "the rename goes on: the editor shows in the next passes");
    assert_eq!(commits().len(), 1, "an idle pass reports nothing more");

    typing.press(Some("two"), true);
    frames.frame(vec![], &mut show);
    let applied = commits();
    assert_eq!(applied.len(), 2, "a second Shift+Enter is a second commit");
    assert_ne!(applied[0].0, applied[1].0, "each is its own gesture, so its own undo step");

    typing.press(Some("three"), false);
    frames.frame(vec![], &mut show);
    assert_eq!(commits().len(), 3, "plain Enter commits");
    let shown = typing.shown.get();
    frames.idle(2, &mut show);
    assert_eq!(typing.shown.get(), shown, "and ends the rename");

    request_rename(&frames.ctx, log.id(), &1_usize);
    frames.idle(3, &mut show);
    typing.press(None, true);
    frames.frame(vec![], &mut show);
    assert_eq!(commits().last().map(|(_, name)| name.as_str()), Some("proposed"), "Shift+Enter on a proposed draft commits it");
    let shown = typing.shown.get();
    frames.idle(2, &mut show);
    assert!(typing.shown.get() > shown, "and keeps the rename going");
}

/// The renaming list's view: its rows, its log and its Enter, which the trial reads.
#[derive(craft_ui::contract::MemoProps)]
struct RenamingPanel {}

thread_local! {
    /// The log, Enter and rows the renaming panel shows: a memoized view's props are plain data.
    static RENAMING_PANEL: Cell<Option<(&'static Log, &'static Cell<bool>, &'static Words)>> = const { Cell::new(None) };
}

fn renaming_panel(_: &mut RenamingPanel, ui: &mut Ui) {
    let Some((log, enter, words)) = RENAMING_PANEL.with(Cell::get) else { return };
    let list = Probe::new(SelectMode::Single).id_salt("list").renaming("b", enter);
    log.take(list.show_items(ui, words, &mut |key, _| ProbeRow::new(*key, log)));
}

/// A rename requested from outside a settled memoized view (a docked panel's body) runs the view, so its collection
/// takes the request and opens the rename field.
#[trial]
fn a_rename_request_runs_the_settled_view_of_its_collection() {
    use craft_ui::contract::MemoView;
    let log = Log::new();
    let enter: &'static Cell<bool> = Box::leak(Box::new(Cell::new(false)));
    let words: &'static Words = Box::leak(Box::new(Words::flat(&["a", "b", "c"])));
    RENAMING_PANEL.with(|panel| panel.set(Some((log, enter, words))));
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            MemoView::new("panel", RenamingPanel {}, renaming_panel).show(ui);
        });
    };
    frames.idle(30, &mut show);
    request_rename(&frames.ctx, log.id(), &1_usize);
    frames.idle(3, &mut show);
    let begun = log.events.borrow().iter().any(|event| matches!(event, CollectionEvent::Rename(EditEvent::Begin { .. })));
    assert!(begun, "the settled view ran and its collection opened the requested rename");
}

/// A rename whose item leaves the source (here, the source shrinks past it) ends once with a removed cancel and writes
/// nothing: the draft is not committed, and the rename does not open again with the old draft.
#[trial]
fn a_rename_whose_item_leaves_the_source_ends_with_a_removed_cancel() {
    let (log, enter, shrunk) = (Log::new(), Cell::new(false), Cell::new(false));
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let words = if shrunk.get() { Words::flat(&["a"]) } else { Words::flat(&["a", "b", "c"]) };
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            let list = Probe::new(SelectMode::Single).id_salt("list").renaming("proposed", &enter);
            log.take(list.show_items(ui, &words, &mut |key, _| ProbeRow::new(*key, log)));
        });
    };
    frames.idle(3, &mut show);
    request_rename(&frames.ctx, log.id(), &1_usize);
    frames.idle(3, &mut show);
    let ends = |log: &Log| -> Vec<&'static str> {
        log.events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                CollectionEvent::Rename(EditEvent::Begin { .. }) => Some("begin"),
                CollectionEvent::Rename(EditEvent::Commit { .. }) => Some("commit"),
                CollectionEvent::Rename(EditEvent::Cancel { reason: CancelReason::Removed, .. }) => Some("removed"),
                CollectionEvent::Rename(EditEvent::Cancel { .. }) => Some("cancel"),
                _ => None,
            })
            .collect()
    };
    assert_eq!(ends(log), ["begin"], "the rename is open");
    shrunk.set(true);
    frames.idle(3, &mut show);
    assert_eq!(ends(log), ["begin", "removed"], "the rename ended once, with a removed cancel and no commit");
    enter.set(true);
    frames.idle(3, &mut show);
    assert_eq!(ends(log), ["begin", "removed"], "nothing more is written after the item left");
}
