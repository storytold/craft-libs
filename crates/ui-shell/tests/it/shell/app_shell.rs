//! The app shell's named slots: a slot keeps its region name on either edge, a bar in a top or bottom slot fills it
//! with its members on the slot's middle, and a slot with a content version settles.

use craft_ui::bars::{Bar, BarKind};
use craft_ui::buttons::Button;
use craft_ui::contract::{PaneEdge, Role, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::runtime::env::WindowKey;
use craft_ui::shell::{AppShell, Title, TitleBar, slots};
use craft_ui::testing::pointer::{press, release};
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
                .tools(PaneEdge::End, 1, true, |ui, _, _: &mut ()| {
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

/// A two-column tool rail gets a slot two rail columns wide, so its second column is not cut off.
#[trial]
fn a_two_column_tool_rail_is_twice_as_wide_as_one_column() {
    let width = |columns: usize| {
        let mut frames = Frames::new();
        let mut width = 0.0;
        for _ in 0..2 {
            frames.frame(vec![], |ui| {
                let out = AppShell::new(WindowKey::main())
                    .tools(PaneEdge::Start, columns, true, |ui, _, _: &mut ()| {
                        Text::new("Rail").show(ui);
                    })
                    .body(|_, _, _| {})
                    .show(ui, &mut ());
                width = out.regions.slots.first().map_or(0.0, |(_, rect)| rect.width());
            });
        }
        width
    };
    let (one, two) = (width(1), width(2));
    assert!(one > 0.0, "the rail shows");
    assert!((two - 2.0 * one).abs() < 0.5, "two columns take two rail widths: {one} and {two}");
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

/// A title bar in the title slot shows its leading content inside the slot, on the slot's middle.
#[trial]
fn a_title_bar_in_the_title_slot_shows_its_content_there() {
    let seen = Rc::new(Cell::new(None::<(Rect, Rect, Rect)>));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let (mut bar, mut member) = (Rect::NOTHING, Rect::NOTHING);
        let shown = AppShell::new(WindowKey::main())
            .title_bar(|ui, _, _: &mut ()| {
                bar = TitleBar::new()
                    .show_titled(
                        ui,
                        Title::new("S"),
                        |ui, _| {
                            member = Button::new("File").show(ui).rect;
                        },
                        |_, _| {},
                    )
                    .response
                    .rect;
            })
            .options_bar(true, |ui, _, _: &mut ()| {
                let _ = Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
                    Button::new("Alpha").show(ui);
                });
            })
            .body(|_, _, _| {})
            .show(ui, &mut ());
        if let Some(slot) = shown.regions.slots.iter().find(|(key, _)| *key == "title").map(|(_, rect)| *rect) {
            out.set(Some((slot, bar, member)));
        }
    });
    fixture.room(Vec2::new(600.0, 300.0)).settle();
    let (slot, bar, member) = seen.get().expect("the title slot shows");
    assert!(member.width() > 0.0 && slot.contains_rect(member), "the bar's member {member:?} shows in the slot {slot:?} (bar {bar:?})");
    assert!((member.center().y - slot.center().y).abs() <= 1.0, "the member {member:?} sits on the slot's middle {slot:?}");
}

/// A status slot that gives a content version runs only in the frame its version changes: frames with no input and a
/// click in the body leave it settled.
#[trial]
fn a_slot_with_a_content_version_runs_only_when_its_version_changes() {
    let (runs, version, body) = (Cell::new(0), Cell::new(1), Cell::new(Rect::NOTHING));
    let frame = |frames: &mut Frames, events| {
        runs.set(0);
        frames.frame(events, |ui| {
            let out = AppShell::new(WindowKey::main())
                .status(true, |ui, _, _: &mut ()| {
                    runs.set(runs.get() + 1);
                    Text::new("Ready").show(ui);
                })
                .content(slots::STATUS, version.get())
                .body(|ui, _, _| {
                    Button::new("Body").show(ui);
                })
                .show(ui, &mut ());
            body.set(out.regions.body);
        });
        runs.get()
    };
    let mut frames = Frames::new();
    for _ in 0..30 {
        frame(&mut frames, vec![]);
    }
    assert_eq!(frame(&mut frames, vec![]), 0, "the status slot settles");
    let at = body.get().center();
    for events in [vec![press(at)], vec![release(at)], vec![]] {
        assert_eq!(frame(&mut frames, events.clone()), 0, "a click in the body ({events:?}) leaves the slot settled");
    }
    version.set(2);
    assert!(frame(&mut frames, vec![]) > 0, "a new version runs the slot");
    assert_eq!(frame(&mut frames, vec![]), 0, "and it settles at it");
}
