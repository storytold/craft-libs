//! The app shell's named slots: a slot keeps its region name on either edge, and a bar in a top or bottom slot fills
//! it with its members on the slot's middle.

use craft_ui::bars::{Bar, BarKind};
use craft_ui::buttons::Button;
use craft_ui::contract::{PaneEdge, Role, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::runtime::env::WindowKey;
use craft_ui::shell::AppShell;
use craft_ui::testing::{Fixture, Frames, trial};
use egui::{Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

/// A tool rail placed at the end edge sits at the window's end and screen readers still find it as the Tools region.
#[trial]
fn the_tool_rail_at_the_end_edge_is_the_tools_region() {
    let mut frames = Frames::new();
    let mut placed = None;
    for _ in 0..3 {
        frames.frame(vec![], |ui| {
            let window = ui.max_rect();
            let out = AppShell::new(WindowKey::main())
                .tools(PaneEdge::End, true, |ui, _, _: &mut ()| {
                    Text::new("Rail").show(ui);
                })
                .body(|_, _, _| {})
                .show(ui, &mut ());
            placed = out.regions.slots.first().map(|(_, rect)| (*rect, window));
        });
    }
    let (rail, window) = placed.expect("the rail shows");
    assert!(rail.center().x > window.center().x, "the rail sits at the end edge: {rail:?} in {window:?}");
    let tree = frames.tree();
    let names: Vec<&str> = tree.with_role(Role::GROUP).into_iter().filter_map(|key| tree.name_of(key)).collect();
    assert!(names.contains(&"Tools"), "the end-edge rail is the Tools region: {names:?}");
}

/// What the options-bar trial saw: the slot, the bar, a member of the bar and a member of a row nested in it.
#[derive(Clone, Copy)]
struct InSlot {
    slot: Rect,
    bar: Rect,
    member: Rect,
    nested: Rect,
}

/// A bar in the options slot is as high as the slot, whatever its own padding and members take, and its members sit
/// on the slot's middle: a member of the bar's own row and a member of a row nested in it beside a child of two lines.
#[trial]
fn a_bar_in_a_top_slot_fills_it_and_centres_its_members_on_one_line() {
    let seen = Rc::new(Cell::new(None::<InSlot>));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let (mut bar, mut member, mut nested) = (Rect::NOTHING, Rect::NOTHING, Rect::NOTHING);
        let shown = AppShell::new(WindowKey::main())
            .options_bar(true, |ui, _, _: &mut ()| {
                bar = Bar::new("Options", BarKind::Options)
                    .show(ui, |ui, _| {
                        member = Button::new("Alpha").show(ui).rect;
                        Row::new().show(ui, |ui, _| {
                            nested = Button::new("Beta").show(ui).rect;
                            Stack::new().show(ui, |ui, _| {
                                Text::new("Above").show(ui);
                                Text::new("Below").show(ui);
                            });
                        });
                    })
                    .output
                    .response
                    .rect;
            })
            .body(|_, _, _| {})
            .show(ui, &mut ());
        let slot = shown.regions.slots.iter().find(|(key, _)| *key == "options").map(|(_, rect)| *rect);
        if let Some(slot) = slot {
            out.set(Some(InSlot { slot, bar, member, nested }));
        }
    });
    fixture.room(Vec2::new(600.0, 300.0)).settle();
    let InSlot { slot, bar, member, nested } = seen.get().expect("the options slot shows");
    const SLACK: f32 = 1.0;
    assert!((bar.min.y - slot.min.y).abs() <= SLACK && (bar.max.y - slot.max.y).abs() <= SLACK, "the bar {bar:?} fills the slot {slot:?}");
    for (name, rect) in [("the bar's member", member), ("the nested row's member", nested)] {
        assert!((rect.center().y - slot.center().y).abs() <= SLACK, "{name} {rect:?} sits on the middle of the slot {slot:?}");
    }
}
