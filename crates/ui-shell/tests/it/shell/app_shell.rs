//! The app shell's named slots: a bar in a top or bottom slot fills it
//! with its members on the slot's middle, and a slot with a content version settles. A full-window page hides the
//! slots and the body, and its band keeps the window's ends and moves the window.

use craft_host::target::ShellInfo;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, CommandSet, Props as _, Show as _, ShowContainer as _};
use craft_ui::dialog::Dialog;
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::commands::{CommandScope, Commands as _};
use craft_ui::runtime::env::{PageBandRoom, WindowKey};
use craft_ui::runtime::windows::{WindowHit, WindowHost};
use craft_ui::shell::{AppShell, Caption, CaptionFamily, Captions, Frame, MacInset, Slot, SlotSize, Title, TitleBar, slots};
use craft_ui::testing::pointer::{press, release};
use craft_ui::testing::windows::NoWindows;
use craft_ui::testing::{Fixture, Frames, trial};
use craft_ui_test_kit::windows::Windows;
use craft_window::policy::{Decision, Desktop, DesktopKind};
use egui::{Event, Key, Modifiers, Rect, Vec2, ViewportId};
use std::cell::Cell;
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

/// A top slot sized by its content (a ribbon: a strip over a taller group row) stacks its content, is as tall as both
/// lines, gives the lower line the slot's width, and the body starts under it.
#[trial]
fn a_top_slot_sized_by_its_content_stacks_it_and_takes_its_height() {
    let seen = Rc::new(Cell::new(None::<(Rect, Rect, Rect, Rect)>));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let (mut strip, mut row) = (Rect::NOTHING, Rect::NOTHING);
        let shown = AppShell::new(WindowKey::main())
            .top(Slot::new("ribbon").size(SlotSize::Content), |ui, _, _: &mut ()| {
                strip = Row::new()
                    .show(ui, |ui, _| {
                        Button::new("Home").show(ui);
                        Button::new("Insert").show(ui);
                    })
                    .output
                    .rect;
                row = Row::new()
                    .fill_width()
                    .show(ui, |ui, _| {
                        Stack::new().show(ui, |ui, _| {
                            for line in ["Paste", "Cut", "Copy", "Format Painter"] {
                                Button::new(line).show(ui);
                            }
                        });
                    })
                    .output
                    .rect;
            })
            .body(|_, _, _| {})
            .show(ui, &mut ());
        if let Some(slot) = shown.regions.slots.iter().find(|(key, _)| *key == "ribbon").map(|(_, rect)| *rect) {
            out.set(Some((slot, strip, row, shown.regions.body)));
        }
    });
    fixture.room(Vec2::new(600.0, 400.0)).settle();
    let (slot, strip, row, body) = seen.get().expect("the ribbon slot shows");
    const SLACK: f32 = 1.0;
    assert!(row.min.y >= strip.max.y - SLACK, "the row {row:?} lies under the strip {strip:?}, not beside it");
    assert!(slot.min.y <= strip.min.y + SLACK && row.max.y <= slot.max.y + SLACK, "the slot {slot:?} holds the strip {strip:?} and the row {row:?}");
    assert!(slot.height() >= strip.height() + row.height() - SLACK, "the slot {slot:?} is as tall as both lines, not one bar");
    assert!((row.width() - slot.width()).abs() <= SLACK, "the row {row:?} takes the slot's width {slot:?}");
    assert!(body.min.y >= row.max.y - SLACK, "the body {body:?} starts under the row {row:?}");
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
            .title_bar(|ui, _, runs: &mut PageRuns| {
                runs.slots += 1;
                Text::new("Title").show(ui);
            })
            .start(Slot::new("tools"), |ui, _, runs: &mut PageRuns| {
                runs.slots += 1;
                Text::new("Tools").show(ui);
            })
            .body(|ui, _, runs: &mut PageRuns| {
                runs.body += 1;
                Text::new("Document").show(ui);
            })
            .page(true, CommandScope::APP, |ui, cx, runs: &mut PageRuns| {
                runs.page += 1;
                runs.band = cx.shell().and_then(|shell| shell.page_band());
                Text::new("Print").show(ui);
                if dialog {
                    Dialog::new("Page setup").opened(Bind::given(true)).show(ui, |ui, _| {
                        Text::new("Setup").show(ui);
                    });
                }
            })
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
            .page(true, CommandScope::APP, |ui, _, _: &mut PageRuns| {
                Text::new("Print").show(ui);
            })
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
    for events in [vec![craft_ui::testing::pointer::moved(free)], vec![press(free)], vec![craft_ui::testing::pointer::moved(to)], vec![release(to)]] {
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
                .page(true, PAGE, |ui, cx, _: &mut ()| {
                    while cx.take_command::<PageKeys>().is_some() {
                        printed.set(printed.get() + 1);
                    }
                    Text::new("Print").show(ui);
                })
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
