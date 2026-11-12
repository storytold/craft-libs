//! `LayersPanel` drags, menus and cursors: a mask thumbnail or the `fx` badge dropped on another layer, what a drag
//! carries for the app's own drop targets, the row menu's Label Color submenu, the menu of the empty space and the
//! load-selection cursor over a thumbnail.

use craft_behavior::EditEvent;
use craft_behavior::{Cursor, CursorBadge, CursorIcon, PaintedCursor, SelectionChange};
use craft_layers::ui::mock::FakeLayers;
use craft_layers::ui::panel::{LayerDrop, LayerSelect, LayersDrag, LayersEvent, LayersPanel, MaskMove};
use craft_layers::ui::tree::{LayerPart, LayerRowKey};
use craft_theme::LabelColor;
use craft_ui::contract::Entry;
use craft_ui::testing::{Fixture, Target, pointer, trial};
use egui::{Modifiers, Pos2, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

type Seen = Rc<RefCell<Vec<LayersEvent<u32>>>>;
/// The rows the host's row menu was asked for, `None` for the empty space.
type Asked = Rc<RefCell<Vec<Option<LayerRowKey<u32>>>>>;

/// The room of the panel: wide enough for a row's parts, tall enough for the sample tree and the footer.
const PANEL: Vec2 = Vec2::new(420.0, 560.0);

/// The panel over the sample tree, with a row menu of one entry, the events it reports and the rows its menu was
/// asked for.
fn rig() -> (Fixture, Seen, Asked) {
    let (seen, asked): (Seen, Asked) = (Rc::default(), Rc::default());
    let (sink, menus) = (Rc::clone(&seen), Rc::clone(&asked));
    let tree = FakeLayers::sample();
    let mut fixture = Fixture::new(move |ui| {
        let out = LayersPanel::new(&tree)
            .row_menu(|target| {
                menus.borrow_mut().push(target.row.clone());
                vec![Entry::new(String::from("rename"), "Rename Layer…")]
            })
            .show(ui);
        sink.borrow_mut().extend(out.events);
    });
    fixture.room(PANEL);
    fixture.settle();
    (fixture, seen, asked)
}

/// A right-click at `at`.
fn right_click(fixture: &mut Fixture, at: Pos2) {
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.idle(3);
}

/// The middle of the painted `fx` badge of the row named `layer`.
fn fx_badge(fixture: &Fixture, layer: &str) -> Pos2 {
    let row = node(fixture, layer).rect;
    let badge = fixture.texts().iter().find(|text| text.text == "fx" && row.contains(text.rect.center())).expect("the row's fx badge");
    badge.rect.center()
}

fn node(fixture: &Fixture, label: &str) -> Target {
    fixture.by_label(label).unwrap_or_else(|| panic!("no node named {label:?}"))
}

/// A point on the layer mask thumbnail of `layer`, which follows its link chain.
fn mask_thumb(fixture: &Fixture, layer: &str) -> Pos2 {
    let link = node(fixture, &format!("Unlink layer mask of {layer}"));
    Pos2::new(link.rect.right() + link.rect.width() * 0.75, link.rect.center().y)
}

/// Drags from `from` and releases on the middle of the row named `onto`.
fn drag_onto(fixture: &mut Fixture, from: Pos2, onto: &str) {
    let to = node(fixture, onto).center();
    fixture.drag_hold_at(from, to - from).release_at(to).idle(2);
}

/// The mask drops among the events reported since the last call.
fn mask_drops(seen: &Seen) -> Vec<MaskMove<u32>> {
    seen.borrow_mut()
        .drain(..)
        .filter_map(|event| match event {
            LayersEvent::Drop(EditEvent::Set { after: LayerDrop::Mask(mv), .. }) => Some(mv),
            _ => None,
        })
        .collect()
}

/// The effects transfers among the events reported since the last call: the rows, the layer they go to and the copy flag.
fn transfers(seen: &Seen) -> Vec<(Vec<LayerRowKey<u32>>, u32, bool)> {
    seen.borrow_mut()
        .drain(..)
        .filter_map(|event| match event {
            LayersEvent::Drop(EditEvent::Set { after: LayerDrop::Transfer(t), .. }) => Some((t.rows, t.to, t.copy)),
            _ => None,
        })
        .collect()
}

/// A layer mask's thumbnail dropped on another layer's row moves the mask there and Alt copies it; a layer that has a
/// layer mask already takes none.
#[trial]
fn a_mask_thumbnail_dropped_on_another_layer_moves_the_mask_there() {
    let (mut fixture, seen, _) = rig();
    let from = mask_thumb(&fixture, "Masked");
    drag_onto(&mut fixture, from, "Title");
    assert_eq!(mask_drops(&seen), [MaskMove::new(3, LayerPart::LayerMask, 4, false)], "onto Title");
    fixture.hold(Modifiers::ALT);
    drag_onto(&mut fixture, from, "Title");
    fixture.hold(Modifiers::NONE);
    assert_eq!(mask_drops(&seen), [MaskMove::new(3, LayerPart::LayerMask, 4, true)], "Alt copies");
    drag_onto(&mut fixture, from, "Levels 1");
    assert_eq!(mask_drops(&seen), [], "Levels 1 has a layer mask already");
}

/// While layer rows are dragged, the drag session carries their layers for the app's targets outside the panel (a
/// document tab); a mask's drag carries no layers.
#[trial]
fn a_row_drag_carries_its_layers_for_targets_outside_the_panel() {
    let (mut fixture, ..) = rig();
    let held = |fixture: &Fixture| craft_ui::runtime::drag::active::<LayersDrag<u32>>(fixture.ctx()).expect("a drag of the panel is held");
    let title = node(&fixture, "Title").center();
    let away = Vec2::new(0.0, 40.0);
    fixture.drag_hold_at(title, away);
    assert_eq!(held(&fixture).layers(), [&4], "the dragged row's layer");
    fixture.release_at(title + away).idle(2);
    let mask = mask_thumb(&fixture, "Masked");
    fixture.drag_hold_at(mask, away);
    assert!(held(&fixture).layers().is_empty(), "a mask is no layer");
    fixture.release_at(mask + away).idle(2);
}

/// The row menu ends with the Label Color submenu, whose pick colours the row's layer as the eye's menu does.
#[trial]
fn the_row_menus_label_color_submenu_colours_the_layer() {
    let (mut fixture, seen, _) = rig();
    let title = node(&fixture, "Title").center();
    right_click(&mut fixture, title);
    let submenu = node(&fixture, "Label Color");
    fixture.click(&submenu).idle(3);
    let red = node(&fixture, "Red");
    fixture.click(&red).idle(2);
    let picked: Vec<(Vec<u32>, Option<LabelColor>)> = seen
        .borrow()
        .iter()
        .filter_map(|event| match event {
            LayersEvent::LabelColor { rows, color, .. } => Some((rows.clone(), *color)),
            _ => None,
        })
        .collect();
    assert_eq!(picked, [(vec![4], Some(LabelColor::Red))]);
}

/// A right-click on the empty space below the rows opens the host's menu for no row.
#[trial]
fn a_right_click_below_the_rows_opens_the_menu_of_the_empty_space() {
    let (mut fixture, seen, asked) = rig();
    let last = node(&fixture, "Background").rect;
    right_click(&mut fixture, Pos2::new(last.center().x, last.bottom() + 20.0));
    assert_eq!(asked.borrow().last(), Some(&None), "the host's menu is asked for the empty space");
    assert!(!seen.borrow().iter().any(|event| matches!(event, LayersEvent::Select(_))), "the selection is left alone");
}

/// The `fx` badge drags every effect of its layer onto another layer, Alt copies; a click on it selects the row.
#[trial]
fn the_fx_badge_drags_the_layers_effects_and_a_click_on_it_selects_the_row() {
    let (mut fixture, seen, _) = rig();
    let badge = fx_badge(&fixture, "Styled");
    drag_onto(&mut fixture, badge, "Title");
    assert_eq!(transfers(&seen), [(vec![LayerRowKey::Effects(7, None)], 4, false)], "onto Title");
    fixture.hold(Modifiers::ALT);
    drag_onto(&mut fixture, badge, "Title");
    fixture.hold(Modifiers::NONE);
    assert_eq!(transfers(&seen), [(vec![LayerRowKey::Effects(7, None)], 4, true)], "Alt copies");
    fixture.click_at(badge).idle(2);
    let selects: Vec<LayerSelect<u32>> =
        seen.borrow().iter().filter_map(|event| if let LayersEvent::Select(select) = event { Some(select.clone()) } else { None }).collect();
    assert_eq!(selects.last().map(|select| &select.change), Some(&SelectionChange::Set(vec![7])), "the click selects Styled");
}

/// With Cmd/Ctrl held over a thumbnail a click would load as a selection, the cursor is the pointing hand, with the
/// mark of the operation Shift or Alt picks; without Cmd/Ctrl it is not.
#[trial]
fn cmd_over_a_thumbnail_shows_the_load_cursor() {
    let (mut fixture, ..) = rig();
    let cursor_over = |fixture: &mut Fixture, at: Pos2, mods: Modifiers| {
        fixture.hold(mods);
        fixture.frame(vec![pointer::moved(at)]).idle(2);
        let cursor = craft_ui::runtime::cursor::current(fixture.ctx());
        fixture.hold(Modifiers::NONE);
        cursor
    };
    let mask = mask_thumb(&fixture, "Masked");
    assert_eq!(cursor_over(&mut fixture, mask, Modifiers::COMMAND), Some(Cursor::System(CursorIcon::PointingHand)), "a new selection");
    assert_eq!(
        cursor_over(&mut fixture, mask, Modifiers::COMMAND | Modifiers::SHIFT),
        Some(Cursor::Painted(PaintedCursor::badged(CursorIcon::PointingHand, CursorBadge::Add))),
        "Shift adds"
    );
    assert_ne!(cursor_over(&mut fixture, mask, Modifiers::NONE), Some(Cursor::System(CursorIcon::PointingHand)), "no Cmd, no load cursor");
}

/// Alt-click on the line between two sibling layers clips the upper one, or releases it when it is clipped; the line
/// under an open group's row, which leads into the group, clips nothing.
#[trial]
fn alt_click_on_the_line_between_two_sibling_layers_clips_the_upper_one() {
    let (mut fixture, seen, _) = rig();
    let line = |fixture: &Fixture, upper: &str| {
        let rect = node(fixture, upper).rect;
        Pos2::new(rect.center().x, rect.bottom() - 1.0)
    };
    let alt_click = |fixture: &mut Fixture, at: Pos2| -> Vec<(u32, bool)> {
        seen.borrow_mut().clear();
        fixture.hold(Modifiers::ALT);
        fixture.frame(vec![pointer::moved(at)]).idle(1);
        fixture.click_at(at).idle(2);
        fixture.hold(Modifiers::NONE);
        seen.borrow()
            .iter()
            .filter_map(|event| if let LayersEvent::ClipLine { upper, release, .. } = event { Some((*upper, *release)) } else { None })
            .collect()
    };
    let title = line(&fixture, "Title");
    assert_eq!(alt_click(&mut fixture, title), [(4, false)], "Title clips to Levels 1");
    let clipped = line(&fixture, "Clipped");
    assert_eq!(alt_click(&mut fixture, clipped), [(2, true)], "Clipped is released");
    let group = line(&fixture, "Group 1");
    assert_eq!(alt_click(&mut fixture, group), [], "the line into an open group");
}
