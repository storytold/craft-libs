//! A footer row: a row of view modes, a thumbnail slider and a footer bar narrows its slider to leave the bar its
//! actions; a footer bar with a start group shows it at the start edge and the rest from the end edge.

use craft_behavior::IconId;
use craft_ui::buttons::IconButton;
use craft_ui::choice::Segmented;
use craft_ui::contract::{Bind, Entry, ListDrag, Props as _, RowAlign, Show as _, ShowContainer as _};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::sections::{
    DialogRole, DropAction, FooterAction, FooterBar, FooterButton, ReorderableSections, SectionKey, SectionLayout, SectionSpec,
};
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui::tile_grid::{ThumbnailSlider, TileZoom};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};
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
fn one_footer_action_takes_two_payload_types() {
    use craft_ui::contract::{SelectMode, ShowCollection as _};
    use craft_ui::list::{List, ListRow};
    use craft_ui::runtime::drag;

    fn source<P: Clone + Send + Sync + 'static>(ui: &mut Ui, label: &'static str, payload: P) {
        let keys = [0_u8];
        List::<u8>::new(label)
            .id_salt(label)
            .select(SelectMode::None)
            .natural_height(true)
            .drag_payload(move |_: &ListDrag<u8>| Some(payload.clone()))
            .show_items(ui, &craft_behavior::keyed(&keys, |key: &u8| *key), &mut |_, _| ListRow::new(label));
    }

    let taken = Rc::new(RefCell::new(Vec::new()));
    let second_type = Rc::new(Cell::new(true));
    let seen = Rc::clone(&taken);
    let with_second = Rc::clone(&second_type);
    let mut fixture = Fixture::new(move |ui| {
        source(ui, "Accepted number", 4_u8);
        source(ui, "Accepted word", "word");
        source(ui, "Rejected word", "other");
        let action =
            FooterAction::new(IconId::new("trash"), "Delete").enabled(false).takes_drops_disabled(true).accepts_drops(|number: &u8| *number == 4);
        let action = if with_second.get() { action.also_accepts_drops(|word: &&str| *word == "word") } else { action };
        let out = FooterBar::actions([action]).show_footer(ui);
        if let Some(drop) = out.dropped {
            assert_eq!(drop.index, 0, "both payloads belong to the same action");
            let report = (drop.payload::<u8>().copied(), drop.payload::<&str>().copied());
            seen.borrow_mut().push(report);
        }
    });
    fixture.room(Vec2::new(400.0, 300.0)).settle();
    let delete = fixture.by_label("Delete").expect("the one footer action");
    assert!(fixture.tree().is_disabled_key(delete.node));

    for (label, accepted) in [("Accepted number", true), ("Accepted word", true), ("Rejected word", false)] {
        let from = fixture.by_label(label).expect("the source").center();
        fixture.drag_hold_at(from, delete.center() - from);
        assert_eq!(drag::mark(fixture.ctx()).is_some(), accepted, "{label}: the indicator follows the predicate");
        let current = fixture.by_label("Delete").expect("the same footer action");
        assert_eq!(current.node, delete.node, "{label}: handling a different payload keeps the control's identity");
        assert_eq!(fixture.tree().is_disabled_key(current.node), !accepted, "{label}: an accepted drag enables the action");
        fixture.release_at(delete.center()).settle();
    }
    assert_eq!(&*taken.borrow(), &[(Some(4), None), (None, Some("word"))], "each accepted drop reports its original typed payload once");

    let from = fixture.by_label("Accepted number").unwrap().center();
    fixture.drag_hold_at(from, delete.center() - from);
    assert!(drag::mark(fixture.ctx()).is_some());
    second_type.set(false);
    fixture.release_at(delete.center()).settle();
    assert_eq!(fixture.by_label("Delete").unwrap().node, delete.node, "removing the second predicate also keeps the control's identity");
    assert_eq!(taken.borrow().get(2), Some(&(Some(4), None)), "the same action can still take the drop on the pass the predicate is removed");
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

/// A bar of eight icon actions too narrow for them on one line keeps every action inside it, with or without a start
/// group or leading controls.
#[trial]
fn a_footer_bar_too_narrow_for_its_actions_keeps_them_inside_it() {
    for (start, leading) in [(0, false), (2, false), (0, true), (2, true)] {
        let seen = Rc::new(RefCell::new((Rect::NOTHING, Vec::new())));
        let out = Rc::clone(&seen);
        let mut fixture = Fixture::new(move |ui| {
            let icons = ["ban", "check", "copy", "trash", "plus", "more", "list", "grid"];
            let bar = FooterBar::actions(icons.map(|icon| FooterAction::new(IconId::new(icon), icon))).at_start(start);
            let output = if leading {
                bar.show_footer_leading(ui, |ui, _| {
                    IconButton::new(IconId::new("settings")).label("Settings").show(ui);
                })
                .0
            } else {
                bar.show_footer(ui)
            };
            *out.borrow_mut() = (output.response.rect, output.responses.iter().map(|response| response.rect).collect::<Vec<_>>());
        });
        fixture.room(Vec2::new(120.0, 200.0)).settle();
        let (bar, actions) = seen.borrow().clone();
        assert_eq!(actions.len(), 8);
        for rect in &actions {
            assert!(bar.expand(0.5).contains_rect(*rect), "start {start}, leading {leading}: {rect:?} lies outside the bar {bar:?}");
        }
    }
}

/// A bar too narrow for its start group and end group side by side, even with an action on each of their lines, keeps
/// every action inside it, and a focused action keeps the focus as the bar narrows.
#[trial]
fn a_footer_bar_too_narrow_for_its_two_groups_side_by_side_keeps_them_inside_it() {
    let seen = Rc::new(RefCell::new((Rect::NOTHING, Vec::new())));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let actions = ["ban", "check", "copy", "trash"].map(|icon| FooterAction::new(IconId::new(icon), icon));
        let output = FooterBar::actions(actions).at_start(2).show_footer(ui);
        *out.borrow_mut() = (output.response.rect, output.responses.iter().map(|response| response.rect).collect::<Vec<_>>());
    });
    fixture.room(Vec2::new(WIDE, 300.0)).settle();
    let trash = fixture.by_label("trash").expect("the trash action");
    fixture.access(craft_ui::testing::focus::AccessActionKind::Focus, &trash).settle();
    let tree = fixture.tree();
    assert_eq!(tree.name_of(craft_ui::testing::access::focused_key(tree)), Some("trash"), "the action takes the focus");
    fixture.room(Vec2::new(40.0, 300.0)).settle();
    let tree = fixture.tree();
    assert_eq!(tree.name_of(craft_ui::testing::access::focused_key(tree)), Some("trash"), "the focused action keeps its focus");
    let (bar, actions) = seen.borrow().clone();
    assert_eq!(actions.len(), 4);
    for rect in &actions {
        assert!(bar.expand(0.5).contains_rect(*rect), "{rect:?} lies outside the bar {bar:?}");
    }
}

/// The slider's rect and the rects of three actions of a footer bar whose leading control is a thumbnail slider, in a
/// room `width` wide.
fn slider_footer(width: f32) -> (Rect, Vec<Rect>) {
    slider_footer_in(width, craft_ui::runtime::env::Direction::Ltr)
}

fn slider_footer_in(width: f32, direction: craft_ui::runtime::env::Direction) -> (Rect, Vec<Rect>) {
    let seen = Rc::new(RefCell::new((Rect::NOTHING, Vec::new())));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let mut zoom = 1.0;
        let actions = ["trash", "plus", "new_folder"].map(|icon| FooterAction::new(IconId::new(icon), icon));
        let (output, slider) = FooterBar::actions(actions)
            .show_footer_leading(ui, |ui, _| ThumbnailSlider::new("Size", Bind::bound(&mut zoom), TileZoom::Ladder).show(ui).response.rect);
        *out.borrow_mut() = (slider.unwrap_or(Rect::NOTHING), output.responses.iter().map(|response| response.rect).collect());
    });
    fixture.direction(direction).room(Vec2::new(width, 200.0)).settle();
    seen.borrow().clone()
}

/// Beside a leading slider, the slider narrows before the actions continue on a new line: a bar with room for the
/// actions and half the slider keeps the actions on one line at its end edge.
#[trial]
fn a_footer_bar_narrows_its_leading_slider_before_its_actions_wrap() {
    let (wide, actions) = slider_footer(900.0);
    let span =
        actions.iter().map(|rect| rect.max.x).fold(f32::NEG_INFINITY, f32::max) - actions.iter().map(|rect| rect.min.x).fold(f32::INFINITY, f32::min);
    let width = (wide.width() / 2.0 + span + 32.0).ceil();
    let (slider, actions) = slider_footer(width);
    assert_eq!(actions.len(), 3);
    assert!(actions.iter().all(|rect| (rect.min.y - actions[0].min.y).abs() < 0.5), "the actions stay on one line: {actions:?}");
    assert!(actions.iter().all(|rect| rect.max.x <= width + 0.5), "the actions end inside the {width}-point bar: {actions:?}");
    assert!(slider.width() < wide.width() - 0.5, "the slider kept its width {} in a narrow bar", slider.width());
}

#[trial]
fn a_footer_bar_moves_its_actions_below_a_leading_control_that_cannot_narrow_further() {
    let width = 40.0;
    for direction in [craft_ui::runtime::env::Direction::Ltr, craft_ui::runtime::env::Direction::Rtl] {
        let (slider, actions) = slider_footer_in(width, direction);
        assert!(actions.iter().all(|rect| rect.min.x >= -0.5 && rect.max.x <= width + 0.5), "the actions overflow: {actions:?}");
        assert!(actions.iter().all(|rect| rect.min.y >= slider.max.y), "the actions must follow the slider on later lines: {slider:?}, {actions:?}");
    }
}

/// A bar whose actions wrapped beside a leading slider puts them back on one line, narrowing the slider, once fewer of
/// them fit there.
#[trial]
fn a_footer_bar_whose_wrapped_actions_shorten_puts_them_back_on_one_line() {
    let (wide, three) = slider_footer(900.0);
    let span =
        three.iter().map(|rect| rect.max.x).fold(f32::NEG_INFINITY, f32::max) - three.iter().map(|rect| rect.min.x).fold(f32::INFINITY, f32::min);
    let width = (wide.width() / 2.0 + span + 32.0).ceil();
    let count = Rc::new(std::cell::Cell::new(8));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let (asked, out) = (Rc::clone(&count), Rc::clone(&seen));
    let mut fixture = Fixture::new(move |ui| {
        let mut zoom = 1.0;
        let icons = ["trash", "plus", "new_folder", "ban", "check", "copy", "more", "list"];
        let actions = icons.into_iter().take(asked.get()).map(|icon| FooterAction::new(IconId::new(icon), icon));
        let (output, _) = FooterBar::actions(actions)
            .show_footer_leading(ui, |ui, _| ThumbnailSlider::new("Size", Bind::bound(&mut zoom), TileZoom::Ladder).show(ui));
        *out.borrow_mut() = output.responses.iter().map(|response| response.rect).collect::<Vec<_>>();
    });
    fixture.room(Vec2::new(width, 200.0)).settle();
    let eight = seen.borrow().clone();
    assert!(eight.iter().any(|rect| (rect.min.y - eight[0].min.y).abs() >= 0.5), "eight actions take more lines: {eight:?}");
    count.set(3);
    fixture.settle();
    let three = seen.borrow().clone();
    assert_eq!(three.len(), 3);
    assert!(three.iter().all(|rect| (rect.min.y - three[0].min.y).abs() < 0.5), "three actions share one line: {three:?}");
    assert!(three.iter().all(|rect| rect.max.x <= width + 0.5), "the three actions end inside the bar: {three:?}");

    // A focused action keeps the focus as the bar widens and its actions go back on one line.
    count.set(8);
    fixture.settle();
    let trash = fixture.by_label("trash").expect("the trash action");
    fixture.access(craft_ui::testing::focus::AccessActionKind::Focus, &trash).settle();
    fixture.room(Vec2::new(900.0, 200.0)).settle();
    let eight = seen.borrow().clone();
    assert!(eight.iter().all(|rect| (rect.min.y - eight[0].min.y).abs() < 0.5), "eight actions share one line in a wide bar: {eight:?}");
    let tree = fixture.tree();
    assert_eq!(tree.name_of(craft_ui::testing::access::focused_key(tree)), Some("trash"), "the focused action keeps its focus");
    fixture.room(Vec2::new(40.0, 400.0)).frame(Vec::new());
    let tree = fixture.tree();
    assert_eq!(
        tree.name_of(craft_ui::testing::access::focused_key(tree)),
        Some("trash"),
        "the focused action keeps its focus below the leading control"
    );
}

/// A bar with a leading control narrowed until its actions wrap keeps them inside it in the first frame at the new
/// width, and each action keeps its id on the new lines.
#[trial]
fn a_footer_bar_wraps_its_actions_beside_a_leading_control_in_the_first_frame() {
    let seen = Rc::new(RefCell::new((Rect::NOTHING, Vec::new())));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let icons = ["ban", "check", "copy", "trash", "plus", "more", "list", "grid"];
        let (output, _) = FooterBar::actions(icons.map(|icon| FooterAction::new(IconId::new(icon), icon))).show_footer_leading(ui, |ui, _| {
            IconButton::new(IconId::new("settings")).label("Settings").show(ui);
        });
        *out.borrow_mut() = (output.response.rect, output.responses.iter().map(|response| (response.id, response.rect)).collect::<Vec<_>>());
    });
    fixture.room(Vec2::new(900.0, 200.0)).settle();
    let wide = seen.borrow().1.clone();
    assert!(wide.iter().all(|(_, rect)| (rect.min.y - wide[0].1.min.y).abs() < 0.5), "one line in a wide bar: {wide:?}");
    for width in [120.0, 40.0] {
        fixture.room(Vec2::new(width, 400.0)).frame(Vec::new());
        let (bar, narrow) = seen.borrow().clone();
        assert_eq!(narrow.len(), 8);
        assert!(narrow.iter().any(|(_, rect)| (rect.min.y - narrow[0].1.min.y).abs() >= 0.5), "the actions take more lines: {narrow:?}");
        for (_, rect) in &narrow {
            assert!(bar.expand(0.5).contains_rect(*rect), "{rect:?} lies outside the bar {bar:?} in the first narrow frame");
        }
        for ((was, _), (now, _)) in wide.iter().zip(&narrow) {
            assert_eq!(was, now, "an action's id changed when the actions wrapped");
        }
    }
}

const DIALOG_BUTTONS: [&str; 4] = ["Reset", "Don't Save", "Cancel", "OK"];

#[trial]
fn a_footer_bar_moves_its_buttons_below_leading_controls_when_they_do_not_fit_beside_them() {
    let (_, wide) = dialog_buttons(900.0, false);
    let width = wide.iter().map(Rect::width).fold(0.0, f32::max) + 8.0;
    let (bar, buttons) = dialog_buttons(width, true);
    assert!(buttons.iter().all(|rect| bar.expand(0.5).contains_rect(*rect)), "buttons overflow the bar {bar:?}: {buttons:?}");
}

/// A focused leading control keeps the focus when the bar narrows and its end group moves below it, for a bar of
/// actions and a bar of dialog buttons.
#[trial]
fn a_focused_leading_control_keeps_its_focus_when_the_end_group_moves_below_it() {
    for buttons in [false, true] {
        let mut fixture = Fixture::new(move |ui| {
            let leading = |ui: &mut Ui, _: &mut craft_ui::contract::BodyCx<'_>| {
                IconButton::new(IconId::new("settings")).label("Settings").show(ui);
            };
            if buttons {
                let roles = [DialogRole::Other, DialogRole::Alternate, DialogRole::Cancel, DialogRole::Accept];
                FooterBar::buttons(DIALOG_BUTTONS.into_iter().zip(roles).map(|(label, role)| FooterButton::new(label, role)))
                    .show_footer_leading(ui, leading);
            } else {
                let icons = ["trash", "plus", "new_folder"];
                FooterBar::actions(icons.into_iter().map(|icon| FooterAction::new(IconId::new(icon), icon))).show_footer_leading(ui, leading);
            }
        });
        fixture.room(Vec2::new(900.0, 200.0)).settle();
        let settings = fixture.by_label("Settings").expect("the leading control");
        fixture.access(craft_ui::testing::focus::AccessActionKind::Focus, &settings).settle();
        let tree = fixture.tree();
        assert_eq!(tree.name_of(craft_ui::testing::access::focused_key(tree)), Some("Settings"), "buttons {buttons}: the control takes the focus");
        fixture.room(Vec2::new(60.0, 400.0)).frame(Vec::new());
        let tree = fixture.tree();
        assert_eq!(
            tree.name_of(craft_ui::testing::access::focused_key(tree)),
            Some("Settings"),
            "buttons {buttons}: the leading control keeps its focus once the end group moves below it"
        );
    }
}

/// The rect of a bar of four dialog buttons and the rect of each button, in a room `width` wide; `leading` puts a
/// control at the start of the buttons' line.
fn dialog_buttons(width: f32, leading: bool) -> (Rect, Vec<Rect>) {
    let seen = Rc::new(RefCell::new(Rect::NOTHING));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let roles = [DialogRole::Other, DialogRole::Alternate, DialogRole::Cancel, DialogRole::Accept];
        let buttons = DIALOG_BUTTONS.into_iter().zip(roles).map(|(label, role)| FooterButton::new(label, role));
        let bar = FooterBar::buttons(buttons);
        let output = if leading {
            bar.show_footer_leading(ui, |ui, _| {
                IconButton::new(IconId::new("settings")).label("Settings").show(ui);
            })
            .0
        } else {
            bar.show_footer(ui)
        };
        *out.borrow_mut() = output.response.rect;
    });
    fixture.room(Vec2::new(width, 200.0)).settle();
    let buttons = DIALOG_BUTTONS.iter().map(|label| fixture.by_label(label).map_or(Rect::NOTHING, |target| target.rect)).collect();
    (*seen.borrow(), buttons)
}

/// A bar of dialog buttons too narrow for them on one line keeps every button inside it, on more lines; a bar wide
/// enough keeps them on one line at its end edge.
#[trial]
fn a_footer_bar_too_narrow_for_its_dialog_buttons_keeps_them_inside_it() {
    for leading in [false, true] {
        let (bar, buttons) = dialog_buttons(240.0, leading);
        for (label, rect) in DIALOG_BUTTONS.iter().zip(&buttons) {
            assert!(rect.is_positive(), "leading {leading}: {label} is shown: {rect:?}");
            assert!(bar.expand(0.5).contains_rect(*rect), "leading {leading}: {label} at {rect:?} lies outside the bar {bar:?}");
        }
        assert!(buttons.iter().any(|rect| rect.min.y > buttons[0].max.y - 0.5), "leading {leading}: the buttons take more lines: {buttons:?}");

        let (bar, buttons) = dialog_buttons(900.0, leading);
        assert!(buttons.iter().all(|rect| (rect.min.y - buttons[0].min.y).abs() < 0.5), "leading {leading}: one line in a wide bar: {buttons:?}");
        let end = buttons.iter().map(|rect| rect.max.x).fold(f32::NEG_INFINITY, f32::max);
        assert!(end > bar.max.x - 1.0, "leading {leading}: the buttons end at {end}, short of the bar's end {}", bar.max.x);
    }
}
