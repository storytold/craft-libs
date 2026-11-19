//! The app shell's named slots: a bar in a top or bottom slot fills it with its members on the slot's middle, and a
//! slot whose content is a memoized view runs only when its props change.

use craft_ui::bars::{Bar, BarKind};
use craft_ui::buttons::Button;
use craft_ui::contract::{MemoProps, MemoView, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::runtime::env::WindowKey;
use craft_ui::shell::{AppShell, Slot, Title, TitleBar};
use craft_ui::testing::pointer::{press, release};
use craft_ui::testing::{Fixture, Frames, trial};
use egui::{Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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

thread_local! {
    /// The memoized slots that ran on this trial's thread, in order.
    static SLOT_RUNS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

/// Takes the memoized slots that ran since the last call.
fn slot_runs() -> Vec<&'static str> {
    SLOT_RUNS.with(|runs| std::mem::take(&mut *runs.borrow_mut()))
}

/// What a memoized slot shows.
#[derive(MemoProps)]
struct SlotText {
    name: &'static str,
    text: String,
}

fn slot_text(props: &mut SlotText, ui: &mut Ui) {
    SLOT_RUNS.with(|runs| runs.borrow_mut().push(props.name));
    Text::new(props.text.clone()).show(ui);
}

/// Slots whose content is a memoized view run only when their props change: at rest and for a click in the body they
/// replay, a new status runs the status slot alone, once, and a slot the app does not memoize runs every pass.
#[trial]
fn memoized_slots_run_only_when_their_props_change() {
    let (version, plain, body) = (Cell::new(1), Cell::new(0), Cell::new(Rect::NOTHING));
    let frame = |frames: &mut Frames, events| {
        plain.set(0);
        frames.frame(events, |ui| {
            let out = AppShell::new(WindowKey::main())
                .options_bar(true, |ui, _, _: &mut ()| {
                    let _ = MemoView::new("options", SlotText { name: "options", text: "Options".to_owned() }, slot_text).show(ui);
                })
                .status(true, |ui, _, _| {
                    let _ = MemoView::new("status", SlotText { name: "status", text: format!("Ready {}", version.get()) }, slot_text).show(ui);
                })
                .start(Slot::new("tools"), |ui, _, _| {
                    plain.set(plain.get() + 1);
                    Text::new("Tools").show(ui);
                })
                .body(|ui, _, _| {
                    Button::new("Body").show(ui);
                })
                .show(ui, &mut ());
            body.set(out.regions.body);
        });
        (slot_runs(), plain.get())
    };
    let mut frames = Frames::new();
    for _ in 0..30 {
        frame(&mut frames, vec![]);
    }
    let (ran, plain_runs) = frame(&mut frames, vec![]);
    assert_eq!(ran, Vec::<&str>::new(), "the memoized slots replay at rest");
    assert!(plain_runs > 0, "the slot the app does not memoize runs every pass");
    let at = body.get().center();
    for events in [vec![press(at)], vec![release(at)], vec![]] {
        assert_eq!(frame(&mut frames, events.clone()).0, Vec::<&str>::new(), "a click in the body ({events:?}) leaves the slots replaying");
    }
    version.set(2);
    assert_eq!(frame(&mut frames, vec![]).0, vec!["status"], "a new status runs the status slot alone, once");
    assert_eq!(frame(&mut frames, vec![]).0, Vec::<&str>::new(), "and it replays at it");
}
