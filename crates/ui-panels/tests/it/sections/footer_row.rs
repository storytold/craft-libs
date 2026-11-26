//! A footer row: a row of view modes, a thumbnail slider and a footer bar narrows its slider to leave the bar its
//! actions; a footer bar with a start group shows it at the start edge and the rest from the end edge.

use craft_behavior::IconId;
use craft_ui::buttons::IconButton;
use craft_ui::choice::Segmented;
use craft_ui::contract::{Bind, Entry, ListDrag, Props as _, RowAlign, Show as _, ShowContainer as _};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::sections::{DropAction, FooterAction, FooterBar, ReorderableSections, SectionKey, SectionLayout, SectionSpec};
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui::tile_grid::{ThumbnailSlider, TileZoom};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// The view modes' rect, the slider's rect and the rects of the footer bar's actions, in a footer row `width` wide.
fn footer(width: f32) -> (Rect, Rect, Vec<Rect>) {
    let seen = Rc::new(RefCell::new((Rect::NOTHING, Rect::NOTHING, Vec::new())));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let modes = [Entry::new(0u8, "List").icon(IconId::new("list")), Entry::new(1, "Icons").icon(IconId::new("grid"))];
        let mut zoom = 1.0;
        Row::new().fill_width().align(RowAlign::End).show(ui, |ui, _| {
            let modes = Segmented::new("View", Bind::given(0u8), &modes).show(ui).response.rect;
            let slider = ThumbnailSlider::new("Size", Bind::bound(&mut zoom), TileZoom::Ladder).show(ui).response.rect;
            let actions = ["new_folder", "trash", "more"].map(|icon| FooterAction::new(IconId::new(icon), icon));
            let bar = FooterBar::actions(actions).show_footer(ui);
            *out.borrow_mut() = (modes, slider, bar.responses.iter().map(|response| response.rect).collect());
        });
    });
    fixture.room(Vec2::new(width, 60.0)).settle();
    seen.borrow().clone()
}

#[trial]
fn a_footer_row_narrows_its_slider_to_leave_the_footer_bar_its_actions() {
    let (modes, wide, actions) = footer(900.0);
    let start = |rects: &[Rect]| rects.iter().map(|rect| rect.min.x).fold(f32::INFINITY, f32::min);
    let end = |rects: &[Rect]| rects.iter().map(|rect| rect.max.x).fold(f32::NEG_INFINITY, f32::max);
    let span = |rects: &[Rect]| end(rects) - start(rects);
    // A row with room for the modes, the actions and half the slider's natural length, with a member gap at each
    // seam: the track gives way to about a control's height, as a media bin's footer in a narrow panel needs.
    let gap = wide.min.x - modes.max.x;
    let width = (modes.width() + wide.width() / 2.0 + span(&actions) + 3.0 * gap).ceil();
    let (_, slider, actions) = footer(width);
    assert!(end(&actions) <= width + 0.5, "the actions end at {} past a {width}-point row", end(&actions));
    assert!(slider.width() < wide.width() - 0.5, "the slider kept its width {} in a narrow row", slider.width());
    assert!(slider.max.x <= start(&actions) + 0.5, "the slider {slider:?} runs into the actions {actions:?}");
}

/// The rects of a footer bar's four actions whose first two sit at the start edge, in a bar `WIDE` points wide.
fn split(direction: craft_ui::runtime::env::Direction) -> Vec<Rect> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let actions = ["ban", "check", "copy", "trash"].map(|icon| FooterAction::new(IconId::new(icon), icon));
        let bar = FooterBar::actions(actions).at_start(2).show_footer(ui);
        *out.borrow_mut() = bar.responses.iter().map(|response| response.rect).collect();
    });
    fixture.direction(direction).room(Vec2::new(WIDE, 60.0)).settle();
    seen.borrow().clone()
}

const WIDE: f32 = 400.0;

/// The start group sits at the start edge in the given order and the rest from the end edge, the first of them at the
/// edge, the room left between them; right to left the groups trade sides.
/// What a drop target reported in one pass: whether it took a drop, whether it was picked, whether Shift was among its
/// modifiers.
type Report = (bool, bool, bool);

/// A footer bar whose one action takes a dragged section.
fn footer_target(ui: &mut Ui) -> Report {
    let action = FooterAction::new(IconId::new("trash"), "Delete").accepts_drops(|_: &ListDrag<SectionKey>| true);
    let out = FooterBar::actions([action]).show_footer(ui);
    (out.dropped.is_some(), out.picked.is_some(), out.mods.shift)
}

/// A bare drop slot around a button, which takes a dragged section.
fn slot_target(ui: &mut Ui) -> Report {
    let out = DropAction::new(|_: &ListDrag<SectionKey>| true)
        .show(ui, |ui, _| {
            IconButton::new(IconId::new("trash")).label("Delete").show(ui);
        })
        .output;
    (out.dropped.is_some(), false, out.mods.shift)
}

/// Two sections whose headers drag, over `target`; every pass of `target` is noted in the list.
fn over_sections(target: fn(&mut Ui) -> Report) -> (Fixture, Rc<RefCell<Vec<Report>>>) {
    let layout = Rc::new(RefCell::new(SectionLayout::new(["albums", "folders"].map(SectionKey::new))));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let reports = Rc::clone(&seen);
    let specs = vec![SectionSpec::new("albums", "albums"), SectionSpec::new("folders", "folders")];
    let fixture = Fixture::new(move |ui| {
        Stack::new().show(ui, |ui, _| {
            let mut layout = layout.borrow_mut();
            let _ = ReorderableSections::new("Sidebar", Bind::bound(&mut *layout), &specs).show(ui, |ui, _, _| {
                ui.label("body");
            });
            reports.borrow_mut().push(target(ui));
        });
    });
    (fixture, seen)
}

/// The button going up at `pos` with `modifiers` in the event itself, as a release whose Shift went down after the press.
fn release_with(pos: Pos2, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers }
}

/// The reports from `from` on that satisfy `wanted`.
fn reported(seen: &Rc<RefCell<Vec<Report>>>, from: usize, wanted: impl Fn(&Report) -> bool) -> Vec<Report> {
    seen.borrow()[from..].iter().copied().filter(|report| wanted(report)).collect()
}

/// A drop and a pick report the Shift of the release that made them, not what the press or the frames before held.
#[trial]
fn a_drop_on_an_action_reports_the_modifiers_held_at_the_release() {
    for (name, target) in [("footer bar", footer_target as fn(&mut Ui) -> Report), ("drop slot", slot_target)] {
        let (mut fixture, seen) = over_sections(target);
        fixture.settle();
        let from = fixture.by_label("albums").expect("the Albums header").center();
        let to = fixture.by_label("Delete").expect("the drop target").center();
        fixture.drag_hold_at(from, to - from);
        let held = seen.borrow().len();
        assert!(reported(&seen, 0, |(dropped, ..)| *dropped).is_empty(), "{name}: nothing dropped while the button is held");
        fixture.frame(vec![release_with(to, Modifiers::SHIFT)]).idle(2);
        let dropped = reported(&seen, held, |(dropped, ..)| *dropped);
        assert!(!dropped.is_empty(), "{name}: the release dropped the section on the action");
        assert!(dropped.iter().all(|(.., shift)| *shift), "{name}: the drop reports the Shift of the release: {dropped:?}");
    }

    let (mut fixture, seen) = over_sections(footer_target);
    fixture.settle();
    let at = fixture.by_label("Delete").expect("the action").center();
    fixture.frame(vec![pointer::moved(at)]).frame(vec![pointer::press(at)]);
    let held = seen.borrow().len();
    fixture.frame(vec![release_with(at, Modifiers::SHIFT)]).idle(2);
    let picked = reported(&seen, held, |(_, picked, _)| *picked);
    assert!(!picked.is_empty(), "the release picked the action");
    assert!(picked.iter().all(|(.., shift)| *shift), "the pick reports the Shift of the release: {picked:?}");
}

#[trial]
fn the_start_and_end_groups_of_a_footer_bar_sit_at_their_edges() {
    use craft_ui::runtime::env::Direction;
    let ltr = split(Direction::Ltr);
    assert_eq!(ltr.len(), 4);
    let (pair, between) = (ltr[1].min.x - ltr[0].max.x, ltr[3].min.x - ltr[1].max.x);
    assert!(between > pair + WIDE / 4.0, "the room lies between the groups: {ltr:?}");
    assert!(ltr[0].min.x < WIDE / 4.0 && ltr[2].max.x > WIDE * 3.0 / 4.0, "the groups reach the edges: {ltr:?}");
    assert!(ltr[0].max.x < ltr[1].min.x, "the start group reads from the start edge: {ltr:?}");
    assert!(ltr[3].max.x < ltr[2].min.x, "the end group reads from the end edge: {ltr:?}");
    let rtl = split(Direction::Rtl);
    assert!(rtl[0].min.x > rtl[2].max.x, "right to left the start group sits at the right: {rtl:?}");
}
