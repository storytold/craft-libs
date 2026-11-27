//! The app shell's named slots: a bar in a top or bottom slot fills it with its members on the slot's middle, and a
//! versioned slot replays while its version holds and no input reaches it. A full-window page hides the slots and the
//! body, and its band keeps the window's ends and moves the window. What the body opens stops above the bottom slots.

use craft_host::target::{Os, ShellInfo};
use craft_theme::Size;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, CommandSet, Entry, Props as _, Role, Scroll, Show as _, ShowContainer as _, Sizing, Trigger};
use craft_ui::dialog::Dialog;
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::menu::bar::{MenuBar, MenuTitle};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::pickers::Select;
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::commands::{CommandScope, Commands as _};
use craft_ui::runtime::env::{PageBandRoom, RevealEdge, WindowKey};
use craft_ui::runtime::windows::{WindowHit, WindowHost};
use craft_ui::shell::{AppShell, Caption, CaptionFamily, Captions, Frame, MacInset, Slot, SlotSize, Title, TitleBar, slots};
use craft_ui::testing::in_audit_run;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::windows::NoWindows;
use craft_ui::testing::{Fixture, Frames, last_pass_runs, trial};
use craft_ui_test_kit::windows::Windows;
use craft_window::policy::{Decision, Desktop, DesktopKind};
use egui::{Event, Key, Modifiers, Rect, Ui, Vec2, ViewportId};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

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
            .options_bar(
                true,
                |_| None,
                |ui, _, _: &mut ()| {
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
                },
            )
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

/// A top slot sized by its content (a ribbon: a strip over a taller group row) stacks its content, is as tall as both
/// lines, gives the lower line the slot's width, and the body starts under it.
///
/// The lower line is a ribbon's group row: a child `Ui` (a fill-width row that scrolls along x) holding a column far
/// taller than one bar. The trial reads where the lines lie from what they allocated in the slot's `Ui`, not from their
/// responses: a response rect is clipped to the pane, so a slot left one bar high would clip the row's response to fit
/// it and hide the fault.
#[trial]
fn a_top_slot_sized_by_its_content_stacks_it_and_takes_its_height() {
    let seen = Rc::new(Cell::new(None::<(Rect, Rect, Rect, Rect)>));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let (mut strip, mut row) = (Rect::NOTHING, Rect::NOTHING);
        let shown = AppShell::new(WindowKey::main())
            .top(
                Slot::new("ribbon").size(SlotSize::Content),
                |_| None,
                |ui, _, _: &mut ()| {
                    let top = ui.cursor().min.y;
                    let strip_x = Row::new()
                        .show(ui, |ui, _| {
                            Button::new("Home").show(ui);
                            Button::new("Insert").show(ui);
                        })
                        .output
                        .rect
                        .x_range();
                    let strip_bottom = ui.min_rect().max.y;
                    let row_top = ui.cursor().min.y;
                    let row_x = Row::new()
                        .scroll(Scroll::horizontal())
                        .fill_width()
                        .show(ui, |ui, _| {
                            Stack::new().show(ui, |ui, _| {
                                for line in ["Paste", "Cut", "Copy", "Format Painter", "Clipboard"] {
                                    Button::new(line).show(ui);
                                }
                            });
                        })
                        .output
                        .rect
                        .x_range();
                    let bottom = ui.min_rect().max.y;
                    strip = Rect::from_x_y_ranges(strip_x, top..=strip_bottom);
                    row = Rect::from_x_y_ranges(row_x, row_top..=bottom);
                },
            )
            .body(|_, _, _| {})
            .show(ui, &mut ());
        if let Some(slot) = shown.regions.slots.iter().find(|(key, _)| *key == "ribbon").map(|(_, rect)| *rect) {
            out.set(Some((slot, strip, row, shown.regions.body)));
        }
    });
    fixture.room(Vec2::new(600.0, 400.0)).settle();
    let (slot, strip, row, body) = seen.get().expect("the ribbon slot shows");
    const SLACK: f32 = 1.0;
    // The rule toward the body, and rounding: a content-sized slot is no taller than its content and that.
    const RULE: f32 = 4.0;
    assert!(row.height() > 2.0 * strip.height(), "the row {row:?} is far taller than the strip {strip:?}, as a ribbon's group row is");
    assert!(row.min.y >= strip.max.y - SLACK, "the row {row:?} lies under the strip {strip:?}, not beside it");
    assert!(slot.min.y <= strip.min.y + SLACK && row.max.y <= slot.max.y + SLACK, "the slot {slot:?} holds the strip {strip:?} and the row {row:?}");
    assert!(slot.height() <= row.max.y - strip.min.y + RULE, "the slot {slot:?} is as tall as its content, from {strip:?} to {row:?}");
    assert!((row.width() - slot.width()).abs() <= SLACK, "the row {row:?} takes the slot's width {slot:?}");
    assert!(body.min.y >= row.max.y - SLACK, "the body {body:?} starts under the row {row:?}");
}

/// The labels of a column of buttons taller than a slot's default extent.
const TALL: [&str; 6] = ["Paste", "Cut", "Copy", "Format Painter", "Clipboard", "Styles"];

/// The rect of the fill-height column a content slot holds and of each of its buttons, from the last run.
type Column = Rc<RefCell<(Rect, Vec<Rect>)>>;

fn column() -> Column {
    Rc::new(RefCell::new((Rect::NOTHING, Vec::new())))
}

/// A column of `labels` that fills its height, as a ribbon group does, recording its rects in `seen`.
fn fill_column(ui: &mut Ui, labels: &[&'static str], seen: &Column) {
    let mut buttons = Vec::new();
    let column = Stack::new()
        .sizing(Sizing::FillHeight)
        .show(ui, |ui, _| {
            for label in labels {
                buttons.push(Button::new(*label).show(ui).rect);
            }
        })
        .output
        .rect;
    *seen.borrow_mut() = (column, buttons);
}

/// A content slot whose content fills its height grows to that content, at the top and at the bottom edge: its
/// content runs in the room the slot can grow into, not in the extent it was first laid out at, so the column takes
/// its buttons' height and the slot takes the column's.
#[trial]
fn a_slot_sized_by_its_content_grows_to_a_child_that_fills_its_height() {
    let (top, bottom) = (column(), column());
    let regions = Rc::new(RefCell::new(None));
    let (top_seen, bottom_seen, out) = (Rc::clone(&top), Rc::clone(&bottom), Rc::clone(&regions));
    let mut fixture = Fixture::new(move |ui| {
        let shown = AppShell::new(WindowKey::main())
            .top(Slot::new("ribbon").size(SlotSize::Content), |_| None, |ui, _, _: &mut ()| fill_column(ui, &TALL, &top_seen))
            .bottom(Slot::new("findBar").size(SlotSize::Content), |_| None, |ui, _, _: &mut ()| fill_column(ui, &TALL, &bottom_seen))
            .body(|_, _, _| {})
            .show(ui, &mut ());
        *out.borrow_mut() = Some(shown.regions);
    });
    fixture.room(Vec2::new(600.0, 700.0)).settle();
    let regions = regions.borrow().clone().expect("the shell shows");
    const SLACK: f32 = 1.0;
    for (key, seen) in [("ribbon", &top), ("findBar", &bottom)] {
        let slot = regions.slots.iter().find(|(shown, _)| *shown == key).map(|(_, rect)| *rect).expect("the slot shows");
        let (column, buttons) = seen.borrow().clone();
        assert_eq!(buttons.len(), TALL.len(), "{key}: every button shows");
        for button in &buttons {
            assert!(column.expand(SLACK).contains_rect(*button), "{key}: the button {button:?} lies in its column {column:?}");
        }
        assert!(slot.expand(SLACK).contains_rect(column), "{key}: the slot {slot:?} holds its column {column:?}");
    }
    let (top_slot, bottom_slot) = (regions.slots[0].1, regions.slots[1].1);
    assert!(
        regions.body.min.y >= top_slot.max.y - SLACK && regions.body.max.y <= bottom_slot.min.y + SLACK,
        "the body {:?} lies between the slots",
        regions.body
    );
    assert!((bottom_slot.max.y - 700.0).abs() <= SLACK, "the bottom slot {bottom_slot:?} stands on the window's bottom edge");
}

/// A settled frame runs a content slot's content once, in one pass: its measure is reused while its inputs are
/// unchanged. A frame where the content grows (a roomier density makes every button taller) lays the window out again
/// at most once, shows the new extent with the body under it in that frame and leaves the next frame settled.
#[trial]
fn a_content_slot_runs_its_content_once_per_settled_frame_and_grows_in_one_frame() {
    let runs = Rc::new(Cell::new(0_u32));
    let seen = column();
    let regions = Rc::new(RefCell::new(None));
    let (count, column, out) = (Rc::clone(&runs), Rc::clone(&seen), Rc::clone(&regions));
    let mut fixture = Fixture::new(move |ui| {
        let shown = AppShell::new(WindowKey::main())
            .top(
                Slot::new("ribbon").size(SlotSize::Content),
                |_| None,
                |ui, _, _: &mut ()| {
                    count.set(count.get() + 1);
                    fill_column(ui, &TALL, &column);
                },
            )
            .body(|_, _, _| {})
            .show(ui, &mut ());
        *out.borrow_mut() = Some(shown.regions);
    });
    fixture.room(Vec2::new(600.0, 700.0)).density(Size::Compact).settle();
    let before = seen.borrow().0;
    runs.set(0);
    fixture.frame(Vec::new());
    assert_eq!(fixture.passes(), 1, "a settled frame runs one pass");
    assert_eq!(runs.get(), 1, "a settled frame runs the slot's content once");
    fixture.density(Size::Comfortable).frame(Vec::new());
    assert!(fixture.passes() <= 2, "the frame the content grows in lays the window out again at most once: {} passes", fixture.passes());
    let slot = regions.borrow().as_ref().and_then(|regions| regions.slots.first().map(|(_, rect)| *rect)).expect("the slot shows");
    let body = regions.borrow().as_ref().map(|regions| regions.body).expect("the body shows");
    let grown = seen.borrow().0;
    assert!(grown.height() > before.height() + 1.0, "the roomier density makes the column taller: {before:?} to {grown:?}");
    assert!(slot.expand(1.0).contains_rect(grown), "in the frame it grows in, the slot {slot:?} holds the grown column {grown:?}");
    assert!((body.top() - slot.bottom()).abs() <= 1.0, "in the frame it grows in, the body {body:?} starts under the grown slot {slot:?}");
    runs.set(0);
    fixture.frame(Vec::new());
    assert_eq!(fixture.passes(), 1, "the frame after the grow is settled: one pass");
    assert_eq!(runs.get(), 1, "the frame after the grow runs the slot's content once");
}

/// Content rails sit at their own sides in either reading direction, each as wide as the column it holds, and a
/// settled frame runs one pass with each rail's content once: the start rail at the reading start, the end rail at the
/// reading end, the body between them.
#[trial]
fn content_rails_sit_at_their_sides_and_settle_in_one_pass() {
    const RAIL: [&str; 3] = ["Move", "Brush", "Eraser"];
    for rtl in [false, true] {
        let direction = if rtl { craft_i18n::Direction::Rtl } else { craft_i18n::Direction::Ltr };
        let (start, end) = (column(), column());
        let (start_runs, end_runs) = (Rc::new(Cell::new(0_u32)), Rc::new(Cell::new(0_u32)));
        let regions = Rc::new(RefCell::new(None));
        let (start_seen, end_seen, start_count, end_count, out) =
            (Rc::clone(&start), Rc::clone(&end), Rc::clone(&start_runs), Rc::clone(&end_runs), Rc::clone(&regions));
        let mut fixture = Fixture::new(move |ui| {
            let shown = AppShell::new(WindowKey::main())
                .start(
                    Slot::new("tools").size(SlotSize::Content),
                    |_| None,
                    |ui, _, _: &mut ()| {
                        start_count.set(start_count.get() + 1);
                        fill_column(ui, &RAIL, &start_seen);
                    },
                )
                .end(
                    Slot::new("inspector").size(SlotSize::Content),
                    |_| None,
                    |ui, _, _: &mut ()| {
                        end_count.set(end_count.get() + 1);
                        fill_column(ui, &RAIL, &end_seen);
                    },
                )
                .body(|_, _, _| {})
                .show(ui, &mut ());
            *out.borrow_mut() = Some(shown.regions);
        });
        fixture.direction(direction).room(Vec2::new(600.0, 400.0)).settle();
        let regions = regions.borrow().clone().expect("the shell shows");
        const SLACK: f32 = 1.0;
        let rect_of = |key| regions.slots.iter().find(|(shown, _)| *shown == key).map(|(_, rect)| *rect).expect("the rail shows");
        let (start_rail, end_rail) = (rect_of("tools"), rect_of("inspector"));
        for (key, rail, seen) in [("tools", start_rail, &start), ("inspector", end_rail, &end)] {
            let (column, buttons) = seen.borrow().clone();
            assert_eq!(buttons.len(), RAIL.len(), "RTL {rtl}, {key}: every button shows");
            for button in &buttons {
                assert!(column.expand(SLACK).contains_rect(*button), "RTL {rtl}, {key}: the button {button:?} lies in its column {column:?}");
            }
            assert!(rail.expand(SLACK).contains_rect(column), "RTL {rtl}, {key}: the rail {rail:?} holds its column {column:?}");
        }
        let body = regions.body;
        let (left, right) = if rtl { (end_rail, start_rail) } else { (start_rail, end_rail) };
        assert!(left.max.x <= body.min.x + SLACK, "RTL {rtl}: the rail {left:?} lies left of the body {body:?}");
        assert!(right.min.x >= body.max.x - SLACK, "RTL {rtl}: the rail {right:?} lies right of the body {body:?}");
        start_runs.set(0);
        end_runs.set(0);
        fixture.frame(Vec::new());
        assert_eq!(fixture.passes(), 1, "RTL {rtl}: a settled frame runs one pass");
        assert_eq!((start_runs.get(), end_runs.get()), (1, 1), "RTL {rtl}: a settled frame runs each rail's content once");
    }
}

/// A content slot holding a list that scrolls along its height is as tall as the list's rows, whether the list sizes
/// from its content or fills its height: the slot offers its content no bound along its edge, so the list shows all
/// of its rows instead of taking the window's height, and the body keeps the rest of the window.
#[trial]
fn a_content_slot_holding_a_scrolling_list_takes_the_list_height() {
    for sizing in [Sizing::Content, Sizing::FillHeight, Sizing::Fill] {
        let rows = Rc::new(RefCell::new(Vec::new()));
        let regions = Rc::new(RefCell::new(None));
        let (seen, out) = (Rc::clone(&rows), Rc::clone(&regions));
        let mut fixture = Fixture::new(move |ui| {
            let shown = AppShell::new(WindowKey::main())
                .top(
                    Slot::new("findBar").size(SlotSize::Content),
                    |_| None,
                    |ui, _, _: &mut ()| {
                        let mut buttons = Vec::new();
                        Stack::new().sizing(sizing).scroll(Scroll::vertical()).show(ui, |ui, _| {
                            for label in ["First", "Second", "Third"] {
                                buttons.push(Button::new(label).show(ui).rect);
                            }
                        });
                        *seen.borrow_mut() = buttons;
                    },
                )
                .body(|_, _, _| {})
                .show(ui, &mut ());
            *out.borrow_mut() = Some(shown.regions);
        });
        fixture.room(Vec2::new(600.0, 700.0)).settle();
        let regions = regions.borrow().clone().expect("the shell shows");
        let slot = regions.slots.iter().find(|(key, _)| *key == "findBar").map(|(_, rect)| *rect).expect("the slot shows");
        let rows = rows.borrow().iter().fold(Rect::NOTHING, |all, row| all.union(*row));
        const SLACK: f32 = 1.0;
        // The rule toward the body, the stack's padding and rounding.
        const AROUND: f32 = 24.0;
        assert!(slot.expand(SLACK).contains_rect(rows), "{sizing:?}: the slot {slot:?} holds the list's rows {rows:?}");
        assert!(slot.height() <= rows.height() + AROUND, "{sizing:?}: the slot {slot:?} is as tall as the list's rows {rows:?}, not the window");
        let body = regions.body;
        assert!(
            body.min.y >= slot.max.y - SLACK && (body.max.y - 700.0).abs() <= SLACK,
            "{sizing:?}: the body {body:?} keeps the window under the slot {slot:?}"
        );
    }
}

/// A check box with its label after it sits on the options slot's middle, as a button beside it does: the slot is
/// shorter than the bar asks for, and the bar centres its members in it.
#[trial]
fn a_labelled_check_box_in_the_options_slot_sits_on_its_middle() {
    let seen = Rc::new(Cell::new(None::<(Rect, Rect, Rect)>));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let (mut check, mut button) = (Rect::NOTHING, Rect::NOTHING);
        let mut on = true;
        let shown = AppShell::new(WindowKey::main())
            .options_bar(
                true,
                |_| None,
                |ui, _, _: &mut ()| {
                    let _ = Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
                        check = Checkbox::new(Bind::bound(&mut on)).label("Anti-alias").show(ui).response.rect;
                        button = Button::new("Select and Mask").show(ui).rect;
                    });
                },
            )
            .body(|_, _, _| {})
            .show(ui, &mut ());
        if let Some(slot) = shown.regions.slots.iter().find(|(key, _)| *key == "options").map(|(_, rect)| *rect) {
            out.set(Some((slot, check, button)));
        }
    });
    fixture.room(Vec2::new(600.0, 300.0)).settle();
    let (slot, check, button) = seen.get().expect("the options slot shows");
    assert!(check.height() > 0.0 && button.height() > 0.0, "both members show (check box {check:?}, button {button:?})");
    assert!((button.center().y - slot.center().y).abs() <= 1.0, "the button {button:?} sits on the slot's middle {slot:?}");
    assert!((check.center().y - slot.center().y).abs() <= 1.0, "the check box {check:?} sits on the slot's middle {slot:?}");
}

/// A title bar in the title slot shows its leading content inside the slot, on the slot's middle.
#[trial]
fn a_title_bar_in_the_title_slot_shows_its_content_there() {
    let seen = Rc::new(Cell::new(None::<(Rect, Rect, Rect)>));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let (mut bar, mut member) = (Rect::NOTHING, Rect::NOTHING);
        let shown = AppShell::new(WindowKey::main())
            .title_bar(
                |_| None,
                |ui, _, _: &mut ()| {
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
                },
            )
            .options_bar(
                true,
                |_| None,
                |ui, _, _: &mut ()| {
                    let _ = Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
                        Button::new("Alpha").show(ui);
                    });
                },
            )
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

thread_local! {
    /// The versioned slots that ran on this trial's thread, in order.
    static SLOT_RUNS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

/// Counts a run of the slot `name`. An audit run is the replay audit's, not the pass's: the trials count none.
fn slot_ran(ui: &Ui, name: &'static str) {
    if !in_audit_run(ui.ctx()) {
        SLOT_RUNS.with(|runs| runs.borrow_mut().push(name));
    }
}

/// Takes the versioned slots that ran since the last call.
fn slot_runs() -> Vec<&'static str> {
    SLOT_RUNS.with(|runs| std::mem::take(&mut *runs.borrow_mut()))
}

/// The shell regions that ran in the last pass.
fn regions_ran(ctx: &egui::Context) -> u32 {
    last_pass_runs(ctx).iter().filter(|(ty, _)| ty.rsplit("::").next() == Some("ShellRegion")).map(|(_, runs)| runs).sum()
}

/// Versioned slots replay while their version holds: at rest and with the pointer moving, pressing and releasing in
/// the body, no versioned slot runs, and of the shell's regions only the window's, the scope's, the body's and the
/// unversioned rail's run. The pointer over the status bar runs the status slot alone, a new status version runs it
/// alone, and the unversioned slot runs every pass.
#[trial]
fn versioned_slots_run_only_when_their_version_changes() {
    let (version, plain) = (Cell::new(1_u64), Cell::new(0));
    let (body, status) = (Cell::new(Rect::NOTHING), Cell::new(Rect::NOTHING));
    let frame = |frames: &mut Frames, events| {
        plain.set(0);
        frames.frame(events, |ui| {
            let out = AppShell::new(WindowKey::main())
                .title_bar(
                    |_| Some(0),
                    |ui, _, _: &mut ()| {
                        slot_ran(ui, "title");
                        Text::new("Title").show(ui);
                    },
                )
                .options_bar(
                    true,
                    |_| Some(0),
                    |ui, _, _| {
                        slot_ran(ui, "options");
                        Text::new("Options").show(ui);
                    },
                )
                .status(
                    true,
                    |_| Some(version.get()),
                    |ui, _, _| {
                        slot_ran(ui, "status");
                        let _ = Bar::new("Status", BarKind::Status).show(ui, |ui, _| {
                            Text::new(format!("Ready {}", version.get())).show(ui);
                        });
                    },
                )
                .start(
                    Slot::new("tools"),
                    |_| None,
                    |ui, _, _| {
                        plain.set(plain.get() + 1);
                        Text::new("Tools").show(ui);
                    },
                )
                .body(|ui, _, _| {
                    Button::new("Body").show(ui);
                })
                .show(ui, &mut ());
            body.set(out.regions.body);
            status.set(out.regions.slots.iter().find(|(key, _)| *key == "status").map_or(Rect::NOTHING, |(_, rect)| *rect));
        });
        (slot_runs(), plain.get(), regions_ran(&frames.ctx))
    };
    let mut frames = Frames::new();
    for _ in 0..30 {
        frame(&mut frames, vec![]);
    }
    let (ran, plain_runs, regions) = frame(&mut frames, vec![]);
    assert_eq!(ran, Vec::<&str>::new(), "the versioned slots replay at rest");
    assert!(plain_runs > 0, "the unversioned slot runs every pass");
    assert_eq!(regions, 4, "only the window's, the scope's, the body's and the unversioned rail's regions run");
    let at = body.get().center();
    for events in [vec![moved(at)], vec![press(at)], vec![release(at)], vec![]] {
        assert_eq!(frame(&mut frames, events.clone()).0, Vec::<&str>::new(), "the pointer in the body ({events:?}) leaves the slots replaying");
    }
    let (ran, ..) = frame(&mut frames, vec![moved(status.get().center())]);
    assert!(!ran.is_empty() && ran.iter().all(|name| *name == "status"), "the pointer over the status bar runs it alone: {ran:?}");
    // The status bar's hover fade runs it until it ends.
    frame(&mut frames, vec![moved(at)]);
    frames.advance(1.0);
    frame(&mut frames, vec![]);
    version.set(2);
    let (ran, ..) = frame(&mut frames, vec![]);
    assert!(!ran.is_empty() && ran.iter().all(|name| *name == "status"), "a new status version runs the status slot alone: {ran:?}");
    assert_eq!(frame(&mut frames, vec![]).0, Vec::<&str>::new(), "and it replays at it");
}

/// What the pass-order trial's slots share: a count the title slot's button raises, and the count the status slot
/// last showed.
#[derive(Default)]
struct Count {
    count: u64,
    shown: Option<u64>,
}

/// A slot's version is read at its turn in the pass: a click on the title slot's button raises the count the status
/// slot's version holds, and the status bar shows the new count in the frame of the click.
#[trial]
fn a_slot_shows_what_an_earlier_slot_changed_in_the_pass() {
    let button = Cell::new(Rect::NOTHING);
    let frame = |frames: &mut Frames, events, app: &mut Count| {
        frames.frame(events, |ui| {
            AppShell::new(WindowKey::main())
                .title_bar(
                    |_| Some(0),
                    |ui, _, app: &mut Count| {
                        let shown = Button::new("Next").show(ui);
                        button.set(shown.rect);
                        if shown.clicks > 0 {
                            app.count += 1;
                        }
                    },
                )
                .status(
                    true,
                    |app| Some(app.count),
                    |ui, _, app| {
                        app.shown = Some(app.count);
                        Text::new(format!("Count {}", app.count)).show(ui);
                    },
                )
                .body(|_, _, _| {})
                .show(ui, &mut *app);
        });
    };
    let (mut frames, mut app) = (Frames::new(), Count::default());
    for _ in 0..3 {
        frame(&mut frames, vec![], &mut app);
    }
    assert_eq!(app.shown, Some(0), "the status bar shows the count");
    let at = button.get().center();
    frame(&mut frames, vec![press(at)], &mut app);
    frame(&mut frames, vec![release(at)], &mut app);
    assert_eq!(app.count, 1, "the click raised the count");
    assert_eq!(app.shown, Some(1), "the status bar showed the new count in the frame of the click");
}

/// A versioned slot sized by its content keeps its extent while it replays, and a version that grows its content lays
/// the window out again in that frame.
#[trial]
fn a_versioned_content_slot_keeps_its_extent_while_it_replays() {
    let (lines, body) = (Cell::new(1_usize), Cell::new(Rect::NOTHING));
    let frame = |frames: &mut Frames| {
        frames.frame(vec![], |ui| {
            let out = AppShell::new(WindowKey::main())
                .top(
                    Slot::new("ribbon").size(SlotSize::Content),
                    |_| Some(lines.get() as u64),
                    |ui, _, _: &mut ()| {
                        slot_ran(ui, "ribbon");
                        for line in 0..lines.get() {
                            Text::new(format!("Line {line}")).show(ui);
                        }
                    },
                )
                .body(|_, _, _| {})
                .show(ui, &mut ());
            body.set(out.regions.body);
        });
        (slot_runs(), body.get())
    };
    let mut frames = Frames::new();
    for _ in 0..5 {
        frame(&mut frames);
    }
    let (_, settled) = frame(&mut frames);
    for _ in 0..5 {
        let (ran, at) = frame(&mut frames);
        assert_eq!(ran, Vec::<&str>::new(), "the slot replays");
        assert_eq!(at, settled, "the body keeps its place under the replayed slot");
    }
    lines.set(3);
    let (ran, grown) = frame(&mut frames);
    assert!(ran.contains(&"ribbon"), "a new version runs the slot");
    assert!(grown.top() > settled.top(), "the grown slot pushes the body down in the frame it grew: {grown:?} after {settled:?}");
    let (_, next) = frame(&mut frames);
    assert_eq!(next, grown, "the frame after the grow keeps the body where the grow frame put it");
}

/// A versioned slot that registers a revealed rect registers it again when it replays: the shell reads the same rects
/// in a replayed pass as in the run.
#[trial]
fn a_replayed_slot_registers_its_revealed_rects() {
    let at = Rect::from_min_size(egui::pos2(0.0, 40.0), Vec2::new(40.0, 120.0));
    let seen = RefCell::new(Vec::new());
    let frame = |frames: &mut Frames| {
        frames.frame(vec![], |ui| {
            AppShell::new(WindowKey::main())
                .start(
                    Slot::new("tools"),
                    |_| Some(0),
                    |ui, bcx, _: &mut ()| {
                        slot_ran(ui, "revealing");
                        if let Some(shell) = bcx.shell() {
                            shell.reveal_rect(ui.ctx(), RevealEdge::Start, at);
                        }
                        Text::new("Tools").show(ui);
                    },
                )
                .body(|ui, bcx, _| {
                    *seen.borrow_mut() = bcx.shell().map(|shell| shell.revealed_rects(ui.ctx())).unwrap_or_default();
                })
                .show(ui, &mut ());
        });
        (slot_runs(), seen.borrow().clone())
    };
    let mut frames = Frames::new();
    let (ran, first) = frame(&mut frames);
    assert!(ran.contains(&"revealing"), "the slot runs in its first frame");
    assert_eq!(first, vec![(RevealEdge::Start, at)], "its run registers the rect");
    for _ in 0..30 {
        frame(&mut frames);
    }
    let (ran, replayed) = frame(&mut frames);
    assert_eq!(ran, Vec::<&str>::new(), "the slot replays");
    assert_eq!(replayed, first, "the replay registers the same rects as the run");
}

/// What the page trial's shell ran in one frame, and the band room the page read.
#[derive(Default)]
struct PageRuns {
    band: Option<PageBandRoom>,
    page: usize,
    body: usize,
    slots: usize,
    shown: usize,
}

/// A window whose frame is `frame`, as the running platform would decide it.
fn framed(frame: Frame) -> Windows {
    let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
    Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(NoWindows)).frame(Decision { frame, overridden: None, desktop }))
}

/// One frame with `events` of a window shell with a title bar, a rail and a body, and a page shown over them; with
/// `dialog` the page has a modal dialog open. Returns the commands the frame sent to the root window.
fn page_frame(windows: &Windows, runs: &mut PageRuns, dialog: bool, events: Vec<Event>) -> Vec<egui::ViewportCommand> {
    *runs = PageRuns::default();
    let out = windows.frame(events, vec![], |ui| {
        let out = AppShell::new(WindowKey::main())
            .title_bar(
                |_| None,
                |ui, _, runs: &mut PageRuns| {
                    runs.slots += 1;
                    Text::new("Title").show(ui);
                },
            )
            .start(
                Slot::new("tools"),
                |_| None,
                |ui, _, runs: &mut PageRuns| {
                    runs.slots += 1;
                    Text::new("Tools").show(ui);
                },
            )
            .body(|ui, _, runs: &mut PageRuns| {
                runs.body += 1;
                Text::new("Document").show(ui);
            })
            .page(
                true,
                CommandScope::APP,
                |_| None,
                |ui, cx, runs: &mut PageRuns| {
                    runs.page += 1;
                    runs.band = cx.shell().and_then(|shell| shell.page_band());
                    Text::new("Print").show(ui);
                    if dialog {
                        Dialog::new("Page setup").opened(Bind::given(true)).show(ui, |ui, _| {
                            Text::new("Setup").show(ui);
                        });
                    }
                },
            )
            .show(ui, &mut *runs);
        runs.shown = out.regions.slots.len();
    });
    out.commands.into_iter().filter(|(viewport, _)| *viewport == ViewportId::ROOT).map(|(_, command)| command).collect()
}

/// The widths of the caption buttons the root window's last pass named, in the order of `names`.
fn caption_widths(windows: &Windows, runs: &mut PageRuns, names: &[&str]) -> Vec<f32> {
    let out = windows.frame(vec![], vec![], |ui| {
        let _ = AppShell::new(WindowKey::main())
            .body(|ui, _, _: &mut PageRuns| {
                Text::new("Document").show(ui);
            })
            .page(
                true,
                CommandScope::APP,
                |_| None,
                |ui, _, _: &mut PageRuns| {
                    Text::new("Print").show(ui);
                },
            )
            .show(ui, &mut *runs);
    });
    let tree = &out.trees.iter().rev().find(|(viewport, _)| *viewport == ViewportId::ROOT).expect("the root pass ran").1;
    let buttons = tree.with_role(craft_ui::contract::Role::BUTTON);
    names
        .iter()
        .map(|name| {
            buttons.iter().find(|key| tree.name_of(**key) == Some(*name)).and_then(|key| tree.bounds_of(*key)).map_or(0.0, |rect| rect.width())
        })
        .collect()
}

/// While a page shows, no slot and no body run and no slot is laid out. The band over the page keeps the window's
/// ends: under an Inset frame its start room is the traffic lights' inset, under an Integrated frame its end room is
/// the caption buttons it shows. Its free part moves the window, and a modal over the page gets a drag strip over it.
#[trial]
fn a_page_hides_the_slots_and_its_band_keeps_the_window_ends_and_moves_the_window() {
    let mut runs = PageRuns::default();

    let inset = MacInset::default();
    let windows = framed(Frame::Inset(inset));
    for _ in 0..3 {
        page_frame(&windows, &mut runs, false, vec![]);
    }
    assert!(runs.page > 0, "the page runs");
    assert_eq!((runs.slots, runs.body, runs.shown), (0, 0, 0), "no slot and no body runs while the page shows");
    let band = runs.band.expect("the page reads its band");
    assert_eq!(band.rect.top(), 0.0, "the band lies at the window's top: {band:?}");
    assert_eq!((band.start, band.end), (inset.leading, 0.0), "under an Inset frame the start room is the lights' inset");

    let layout = craft_window::Layout { left: vec![], right: vec![Caption::Minimize, Caption::Maximize, Caption::Close] };
    let windows = framed(Frame::Integrated(Captions { family: CaptionFamily::Windows, layout }));
    for _ in 0..3 {
        page_frame(&windows, &mut runs, false, vec![]);
    }
    let band = runs.band.expect("the page reads its band");
    let widths = caption_widths(&windows, &mut runs, &["Minimize", "Maximize", "Close"]);
    assert!(widths.iter().all(|width| *width > 0.0), "the band shows the window's caption buttons: {widths:?}");
    let captions: f32 = widths.iter().sum();
    assert_eq!(band.start, 0.0, "the start end holds nothing: {band:?}");
    assert!((band.end - captions).abs() <= 0.5, "the end room {} is the caption buttons' {captions}", band.end);

    // The free band between the ends moves the window.
    let free = egui::pos2((band.rect.left() + band.start + band.rect.right() - band.end) / 2.0, band.rect.center().y);
    for _ in 0..2 {
        page_frame(&windows, &mut runs, false, vec![]);
    }
    let host = WindowHost::of(&windows.ctx);
    let regions = host.caption_regions(ViewportId::ROOT).expect("the band registered its parts");
    assert_eq!(regions.hit(free), Some(WindowHit::Move), "the free band is a drag area: {regions:?}");
    let bands = regions.moves.iter().filter(|area| band.rect.contains_rect(**area)).count();
    let to = free + egui::vec2(40.0, 0.0);
    let mut sent = Vec::new();
    for events in [vec![moved(free)], vec![press(free)], vec![moved(to)], vec![release(to)]] {
        sent.extend(page_frame(&windows, &mut runs, false, events));
    }
    assert!(sent.contains(&egui::ViewportCommand::StartDrag), "a drag of the free band moves the window: {sent:?}");

    // The strips read the last complete pass: the first publishes the band's drag area, the next ones the strips.
    for _ in 0..4 {
        page_frame(&windows, &mut runs, true, vec![]);
    }
    let regions = host.caption_regions(ViewportId::ROOT).expect("the band registered its parts");
    let strips = regions.moves.iter().filter(|area| band.rect.contains_rect(**area)).count();
    assert!(strips > bands, "a modal over the page shows a drag strip over the band: {regions:?}");
}

/// The commands of a page's own scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "trialPage", scope = "trialPage")]
enum PageKeys {
    #[command(label = "Print", default = "Enter")]
    Print,
}

const PAGE: CommandScope = <PageKeys as CommandSet>::SCOPE;

/// With nothing focused and a page shown, the page takes the window's keys as a modal's root does: Enter reaches the
/// command scope on the page, once per press.
#[trial]
fn a_page_takes_the_windows_keys_with_nothing_focused() {
    let printed = Cell::new(0);
    let frame = |frames: &mut Frames, events: Vec<Event>| {
        frames.frame(events, |ui| {
            let _ = AppShell::new(WindowKey::main())
                .body(|ui, _, _: &mut ()| {
                    Button::new("Body").show(ui);
                })
                .page(
                    true,
                    PAGE,
                    |_| None,
                    |ui, cx, _: &mut ()| {
                        while cx.take_command::<PageKeys>().is_some() {
                            printed.set(printed.get() + 1);
                        }
                        Text::new("Print").show(ui);
                    },
                )
                .show(ui, &mut ());
        });
    };
    let mut frames = Frames::new();
    for _ in 0..3 {
        frame(&mut frames, vec![]);
    }
    let key = |pressed| Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    frame(&mut frames, vec![key(true)]);
    frame(&mut frames, vec![key(false)]);
    for _ in 0..3 {
        frame(&mut frames, vec![]);
    }
    assert_eq!(printed.get(), 1, "Enter with nothing focused runs the page's command once");
}

/// A window shell with a status slot and `body`, 400 x 300, and the status slot's rect.
fn shell_with_status(body: fn(&mut Ui)) -> (Fixture, Rc<Cell<Rect>>) {
    let slot = Rc::new(Cell::new(Rect::NOTHING));
    let seen = Rc::clone(&slot);
    let mut fixture = Fixture::new(move |ui| {
        let shown = AppShell::new(WindowKey::main())
            .status(
                true,
                |_| None,
                |ui, _, _: &mut ()| {
                    Text::new("Ready").show(ui);
                },
            )
            .body(|ui, _, _| body(ui))
            .show(ui, &mut ());
        if let Some(rect) = shown.regions.slots.iter().find(|(key, _)| *key == slots::STATUS).map(|(_, rect)| *rect) {
            seen.set(rect);
        }
    });
    // The in-window menu bar is PC behaviour: a Mac app's menu bar is native.
    fixture.os(Os::Linux);
    fixture.room(Vec2::new(400.0, 300.0)).settle();
    (fixture, slot)
}

fn button_menu(ui: &mut Ui) {
    let opener = Button::new("Open").show(ui);
    Menu::new("Long").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
        for at in 0..40 {
            MenuItem::new(format!("Row {at}")).show(ui);
        }
    });
}

fn menu_bar(ui: &mut Ui) {
    let titles = vec![MenuTitle::new("&File", |ui| {
        for at in 0..40 {
            MenuItem::new(format!("Row {at}")).show(ui);
        }
    })];
    MenuBar::new("Main menu", titles).show(ui);
}

fn select_list(ui: &mut Ui) {
    let entries: Vec<Entry<u8>> = (0..60u8).map(|at| Entry::new(at, format!("Item {at}"))).collect();
    let mut value = 0u8;
    Select::new("Pick", Bind::bound(&mut value), &entries).show(ui);
}

/// What a shell's body opens (a menu, a menu bar's menu, a select's list) ends above the shell's bottom slot, so the
/// status bar is never covered.
#[trial]
fn popups_stop_above_the_bottom_slots_in_each_window() {
    type Case = (&'static str, fn(&mut Ui));
    let cases: [Case; 3] = [("Open", button_menu), ("File", menu_bar), ("Pick", select_list)];
    for (opener, body) in cases {
        let (mut fixture, slot) = shell_with_status(body);
        let opener_node = fixture.by_label(opener).unwrap_or_else(|| panic!("the opener {opener:?} shows"));
        fixture.click(&opener_node).settle();
        let popups: Vec<Rect> = [Role::MENU, Role::LIST_BOX]
            .into_iter()
            .flat_map(|role| fixture.by_role(role).iter().map(|node| node.rect).collect::<Vec<_>>())
            .collect();
        let slot = slot.get();
        assert!(!popups.is_empty() && slot.height() > 0.0, "{opener:?} opens a popup over a shown status slot");
        for popup in popups {
            assert!(popup.bottom() <= slot.top() + 0.5, "{opener:?}: the popup {popup:?} ends above the status slot {slot:?}");
        }
    }
}

thread_local! {
    /// The rect the options bar's button took when it last ran.
    static MEMO_BUTTON: Cell<Rect> = const { Cell::new(Rect::NOTHING) };
}

/// An options bar of one text button, as an app's tool options.
fn memo_bar(ui: &mut Ui) {
    let _ = Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
        let rect = Button::new("Straighten").show(ui).rect;
        MEMO_BUTTON.with(|button| button.set(rect));
    });
}

/// photocraft#16405: an options bar in the shell's versioned options slot, which replays it as a memoized view, sits
/// on the slot's line as one shown directly does: its text button is drawn whole, centred in the slot, not cut off at
/// the bottom.
#[trial]
fn a_memoized_options_bar_centres_its_button_in_the_slot() {
    for ppp in [1.0, 1.25, 1.5] {
        let seen = Rc::new(Cell::new(None::<Rect>));
        let out = Rc::clone(&seen);
        let mut fixture = Fixture::new(move |ui| {
            let shown = AppShell::new(WindowKey::main())
                .options_bar(true, |_| Some(0), |ui, _, _: &mut ()| memo_bar(ui))
                .body(|_, _, _| {})
                .show(ui, &mut ());
            if let Some(slot) = shown.regions.slots.iter().find(|(key, _)| *key == "options").map(|(_, rect)| *rect) {
                out.set(Some(slot));
            }
        });
        fixture.ctx().set_pixels_per_point(ppp);
        fixture.room(Vec2::new(600.0, 300.0)).settle();
        let slot = seen.get().expect("the options slot shows");
        let button = MEMO_BUTTON.with(Cell::get);
        assert!(button.height() > 0.0, "{ppp}: the button shows: {button:?}");
        assert!(button.max.y <= slot.max.y + 0.01 && button.min.y >= slot.min.y - 0.01, "{ppp}: the button {button:?} lies inside the slot {slot:?}");
        assert!((button.center().y - slot.center().y).abs() <= 1.0, "{ppp}: the button {button:?} sits on the slot's middle {slot:?}");
    }
}
