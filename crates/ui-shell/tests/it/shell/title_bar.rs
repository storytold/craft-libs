//! A title bar in an embedded shell is a picture of a window: it reports its captions and acts on no window. An action
//! title is a button that reports its activation.

use craft_host::target::ShellInfo;
use craft_ui::contract::Role;
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::env::WindowKey;
use craft_ui::shell::{AppShell, Caption, CaptionFamily, Captions, Frame, Title, TitleBar};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::windows::NoWindows;
use craft_ui::testing::{Fixture, trial};
use craft_ui_test_kit::windows::Windows;
use craft_window::policy::{Decision, Desktop, DesktopKind};
use egui::{Event, Pos2, Rect, Ui, ViewportCommand, ViewportId, ViewportInfo, vec2};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

/// The window commands a title bar sends for a click on its Maximize button, a drag of its free band and a double
/// click on the band, and the caption it reports. With `embedded` the bar is the title bar of an embedded shell.
fn sent(embedded: bool) -> (Vec<ViewportCommand>, Option<Caption>) {
    let layout = craft_window::Layout { left: vec![], right: vec![Caption::Minimize, Caption::Maximize, Caption::Close] };
    let frame = Frame::Integrated(Captions { family: CaptionFamily::Windows, layout });
    let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(NoWindows)).frame(Decision { frame, overridden: None, desktop }));
    let (bar, clicked) = (Cell::new(Rect::NOTHING), Cell::new(None));
    let show_bar = |ui: &mut Ui| {
        let shown = TitleBar::new().show_titled(ui, Title::new("Landscape.psd"), |_, _| {}, |_, _| {});
        bar.set(shown.response.rect);
        clicked.set(clicked.get().or(shown.caption_clicked));
    };
    let frame = |events: Vec<Event>| {
        windows.frame(events, vec![], |ui| {
            if embedded {
                let _ = AppShell::embedded(WindowKey::new("preview")).title_bar(|_| None, |ui, _, _: &mut ()| show_bar(ui)).show(ui, &mut ());
            } else {
                show_bar(ui);
            }
        })
    };
    frame(vec![]);
    let settled = frame(vec![]);
    let tree = &settled.trees.iter().rev().find(|(viewport, _)| *viewport == ViewportId::ROOT).expect("the root pass ran").1;
    let maximize = tree
        .with_role(Role::BUTTON)
        .into_iter()
        .find(|key| tree.name_of(*key) == Some("Maximize"))
        .and_then(|key| tree.bounds_of(key))
        .expect("the bar shows its Maximize button")
        .center();
    // The title is no control: the band lies under it.
    let band = Pos2::new(bar.get().center().x, bar.get().center().y);
    let to = band + vec2(40.0, 0.0);
    let steps = [
        vec![moved(maximize)],
        vec![press(maximize)],
        vec![release(maximize)],
        vec![moved(band)],
        vec![press(band)],
        vec![moved(to)],
        vec![release(to)],
        vec![moved(band)],
        vec![press(band)],
        vec![release(band)],
        vec![press(band)],
        vec![release(band)],
    ];
    let mut commands = Vec::new();
    for events in steps {
        commands.extend(frame(events).commands.into_iter().map(|(_, command)| command).filter(|command| {
            matches!(
                command,
                ViewportCommand::StartDrag
                    | ViewportCommand::Maximized(_)
                    | ViewportCommand::Minimized(_)
                    | ViewportCommand::OuterPosition(_)
                    | ViewportCommand::BeginResize(_)
            )
        }));
    }
    (commands, clicked.get())
}

/// A title bar in an embedded shell (an app preview in a gallery card) reports a click on its Maximize button and
/// sends no window command for it, for a drag of its free band or for a double click on the band. The same bar in a
/// window acts on the window.
#[trial]
fn a_title_bar_in_an_embedded_shell_acts_on_no_window() {
    let (commands, clicked) = sent(true);
    assert_eq!(clicked, Some(Caption::Maximize), "the embedded bar reports the caption");
    assert!(commands.is_empty(), "the embedded bar sends nothing to the window: {commands:?}");
    let (commands, clicked) = sent(false);
    assert_eq!(clicked, Some(Caption::Maximize), "the window's bar reports the caption");
    assert!(!commands.is_empty(), "the window's bar acts on the window");
}

/// The shell sends the window title once after a change and not again while the viewport reports it; an embedded shell
/// sends none.
#[trial]
fn the_window_title_is_sent_once_per_change() {
    let titles = |out: &craft_ui_test_kit::windows::Out| -> Vec<String> {
        out.commands
            .iter()
            .filter_map(|(viewport, command)| match command {
                ViewportCommand::Title(title) if *viewport == ViewportId::ROOT => Some(title.clone()),
                _ => None,
            })
            .collect()
    };
    let windows = Windows::new();
    let frame = |title: &str, embedded: bool| {
        let key = WindowKey::new("titled");
        windows.frame(vec![], vec![], |ui| {
            let shell = if embedded { AppShell::embedded(key.clone()) } else { AppShell::new(key.clone()) };
            let _ = shell.window_title(title).show(ui, &mut ());
        })
    };
    let reported = |title: &str| windows.root_info(ViewportInfo { title: Some(title.to_owned()), ..ViewportInfo::default() });

    assert_eq!(titles(&frame("A", false)), ["A"], "a new title is sent");
    reported("A");
    assert!(titles(&frame("A", false)).is_empty(), "a title the viewport reports is not sent again");
    assert_eq!(titles(&frame("B", false)), ["B"], "a changed title is sent");
    reported("B");
    assert!(titles(&frame("B", false)).is_empty(), "the changed title is sent once");

    let embedded = Windows::new();
    let out = embedded.frame(vec![], vec![], |ui| {
        let _ = AppShell::embedded(WindowKey::new("preview")).window_title("C").show(ui, &mut ());
    });
    assert!(titles(&out).is_empty(), "an embedded shell sends no title");
}

/// The leading content of a bar is a row as wide as the bar: a document tab strip beside the Home and Open buttons
/// takes the width they and the title leave, and its tabs show and are in the tree. The strip used to be laid out at
/// its minimum there, with no tab drawn or reachable.
#[trial]
fn a_document_tab_strip_in_the_bar_takes_the_free_width() {
    use craft_ui::buttons::Button;
    use craft_ui::contract::Show as _;
    use craft_ui::tabs::{DocumentTab, DocumentTabs};
    use craft_ui::testing::Fixture;
    let mut fixture = Fixture::new(|ui| {
        let layout = craft_window::Layout { left: vec![], right: vec![Caption::Minimize, Caption::Maximize, Caption::Close] };
        let frame = Frame::Integrated(Captions { family: CaptionFamily::Windows, layout });
        let tabs = [DocumentTab::new(0_usize, "One.pdf"), DocumentTab::new(1, "Two.pdf"), DocumentTab::new(2, "Three.pdf")];
        let _ = TitleBar::new().frame(frame).show_titled(
            ui,
            Title::new("One.pdf"),
            |ui, _| {
                Button::new("Home").show(ui);
                let _ = DocumentTabs::new("Documents", &tabs).reorderable(true).selected(0).show(ui);
                Button::new("Open").show(ui);
            },
            |ui, _| {
                Button::new("Settings").show(ui);
            },
        );
    });
    fixture.room(vec2(1400.0, 300.0)).settle();
    let home = fixture.by_label("Home").expect("the Home button").rect;
    let open = fixture.by_label("Open").expect("the Open button").rect;
    let tabs: Vec<Rect> = fixture.by_role(Role::TAB).into_iter().map(|tab| tab.rect).collect();
    assert_eq!(tabs.len(), 3, "every tab is in the tree: {tabs:?}");
    for tab in &tabs {
        assert!(tab.width() > 0.0 && tab.left() >= home.right() && tab.right() <= open.left(), "a tab shows between Home and Open: {tab:?}");
    }
    let strip = fixture.by_role(Role::TAB_LIST).first().expect("the strip's node").rect;
    let settings = fixture.by_label("Settings").expect("the Settings button").rect;
    assert!(strip.width() > (settings.left() - home.right()) / 2.0, "the strip takes the free width: {strip:?} up to {settings:?}");
}

/// With tabs that overflow the strip, the trailing control stays in sight at the end of the strip's row, left of the
/// bar's own trailing controls (it runs under none of them).
#[trial]
fn a_trailing_control_of_overflowing_tabs_stays_clear_of_the_bars_controls() {
    use craft_ui::buttons::Button;
    use craft_ui::contract::Show as _;
    use craft_ui::tabs::{DocumentTab, DocumentTabs};
    use craft_ui::testing::Fixture;
    let mut fixture = Fixture::new(|ui| {
        let layout = craft_window::Layout { left: vec![], right: vec![Caption::Minimize, Caption::Maximize, Caption::Close] };
        let frame = Frame::Integrated(Captions { family: CaptionFamily::Windows, layout });
        let tabs: Vec<_> = (0..9).map(|i| DocumentTab::new(i, format!("Statement {i} for the office records, scanned.pdf")).closable(true)).collect();
        let _ = TitleBar::new().frame(frame).show_titled(
            ui,
            Title::new("x"),
            |ui, _| {
                Button::new("Home").show(ui);
                let _ = DocumentTabs::new("Documents", &tabs)
                    .reorderable(true)
                    .selected(0)
                    .trailing(|ui| {
                        Button::new("Open").show(ui);
                    })
                    .show(ui);
            },
            |ui, _| {
                Button::new("Discord").show(ui);
            },
        );
    });
    fixture.room(vec2(1200.0, 300.0)).settle();
    let open = fixture.by_label("Open").expect("the Open button").rect;
    let discord = fixture.by_label("Discord").expect("the Discord button").rect;
    assert!(open.right() <= discord.left() + 0.5, "Open {open:?} runs under Discord {discord:?}");
}

/// What the action-title trial saw: the passes that reported an activation, and the bar's rect.
#[derive(Clone, Copy)]
struct TitleSeen {
    activations: usize,
    bar: Rect,
}

impl Default for TitleSeen {
    fn default() -> Self {
        Self { activations: 0, bar: Rect::NOTHING }
    }
}

/// A settled title bar whose title `title` is an action; what each pass reported.
fn titled(title: &'static str) -> (Fixture, Rc<Cell<TitleSeen>>) {
    let seen = Rc::new(Cell::new(TitleSeen::default()));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        // The title shows whatever frame the trial's host decides.
        let shown = TitleBar::new().title_when_native().show_titled(ui, Title::new(title).action("Rename the document"), |_, _| {}, |_, _| {});
        let mut now = out.get();
        now.bar = shown.response.rect;
        now.activations += usize::from(shown.title_activated);
        out.set(now);
    });
    fixture.room(vec2(600.0, 200.0)).settle();
    (fixture, seen)
}

const LONG: &str = "A very long document title that cannot fit in the free gap of a title bar six hundred points wide, \
                    so the bar cuts it with an ellipsis and keeps the whole of it for the tooltip and the name.psd";

/// An action title is a focusable button named by the shown title. A click and Enter (on the title Tab focused) each
/// report it in the pass they land in and no later pass. The button sits on the window's centre line, and a title too long for the gap is cut to
/// fit inside the bar.
#[trial]
fn an_action_title_is_a_button_and_reports_its_activation() {
    let (mut fixture, seen) = titled("Landscape.psd");
    let title = fixture.by_label("Landscape.psd").expect("the title is a named control");
    assert_eq!(fixture.tree().role_of(title.node), Some(Role::BUTTON), "the title is a button");
    let bar = seen.get().bar;
    assert!((title.rect.center().x - bar.center().x).abs() <= 1.0, "the title {:?} sits on the centre line of {bar:?}", title.rect);
    assert_eq!(seen.get().activations, 0, "nothing is activated before input");

    fixture.click(&title).idle(3);
    assert_eq!(seen.get().activations, 1, "a click reports the title once, in its pass only");
    // A button takes no focus from a click (it senses no `TAKE_FOCUS`); Tab reaches the title, and Enter then reports it.
    fixture.key(egui::Key::Tab, egui::Modifiers::NONE).idle(3);
    assert!(fixture.focused_stop().is_some(), "Tab focuses the title");
    fixture.key(egui::Key::Enter, egui::Modifiers::NONE).idle(3);
    assert_eq!(seen.get().activations, 2, "Enter on the focused title reports it once");

    let (fixture, seen) = titled(LONG);
    let title = fixture.by_label(LONG).expect("the long title is named in full");
    let bar = seen.get().bar;
    assert!(
        bar.contains_rect(title.rect) && title.rect.width() < bar.width(),
        "the long title {:?} is cut to fit inside the bar {bar:?}",
        title.rect
    );
}

/// What the mirrored-title trial saw, by label: the buttons' and the tabs' rects in the settled pass.
fn lights_right_bar(follows: bool, lights_right: bool) -> impl Fn(&str) -> Rect {
    use craft_ui::buttons::Button;
    use craft_ui::contract::{Show as _, ShowContainer as _};
    use craft_ui::shell::MacInset;
    use craft_ui::tabs::{DocumentTab, DocumentTabs};
    let frame = Frame::Inset(MacInset::default().with_lights_right(lights_right));
    let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(NoWindows)).frame(Decision { frame, overridden: None, desktop }));
    let tabs = [DocumentTab::new(0_usize, "first.pdf"), DocumentTab::new(1, "last.pdf")];
    let frame = || {
        windows.frame(vec![], vec![], |ui| {
            let _ = AppShell::new(WindowKey::main())
                .title_follows_lights(follows)
                .title_bar(
                    |_| None,
                    |ui, _, _: &mut ()| {
                        let _ = TitleBar::new()
                            .trailing(|ui, _| {
                                Button::new("Discord").show(ui);
                                Button::new("Preferences").show(ui);
                            })
                            .show(ui, |ui, _| {
                                Button::new("Home").show(ui);
                                let _ = DocumentTabs::new("Documents", &tabs)
                                    .selected(0)
                                    .trailing(|ui| {
                                        Button::new("Open").show(ui);
                                    })
                                    .show(ui);
                            });
                    },
                )
                .body(|_, _, _| {})
                .show(ui, &mut ());
        })
    };
    for _ in 0..3 {
        frame();
    }
    let settled = frame();
    let tree = settled.trees.into_iter().rev().find(|(viewport, _)| *viewport == ViewportId::ROOT).expect("the root pass ran").1;
    move |label: &str| {
        tree.with_role(Role::BUTTON)
            .into_iter()
            .chain(tree.with_role(Role::TAB))
            .find(|key| tree.name_of(*key) == Some(label))
            .and_then(|key| tree.bounds_of(key))
            .unwrap_or_else(|| panic!("{label} is in the tree"))
    }
}

/// macOS with a right-to-left system language lays the traffic lights out from the right end. A shell whose title
/// follows the lights mirrors the bar's flow: the lights' room is at the right, Home sits beside it, the tabs read from
/// the right, then Open; the trailing controls are at the far left, in mirrored order. Labels keep reading left to
/// right (production 4448ee940).
#[trial]
fn a_title_that_follows_the_lights_mirrors_when_they_are_at_the_right() {
    let (width, lights) = (800.0, craft_ui::shell::MacInset::default().leading);
    let rect = lights_right_bar(true, true);
    let home = rect("Home");
    for label in ["Home", "first.pdf", "last.pdf", "Open", "Discord", "Preferences"] {
        assert!(rect(label).right() <= width - lights + 0.5, "{label} {:?} clears the lights at the right", rect(label));
    }
    assert!(home.right() >= width - lights - 16.0, "Home {home:?} sits beside the lights");
    assert!(rect("first.pdf").right() <= home.left() + 0.5, "the first tab follows Home");
    assert!(rect("last.pdf").right() <= rect("first.pdf").left() + 0.5, "the tabs read from the right");
    assert!(rect("Open").right() <= rect("last.pdf").left() + 0.5, "Open follows the tabs");
    let (discord, preferences) = (rect("Discord"), rect("Preferences"));
    assert!(preferences.left() < 20.0, "Preferences {preferences:?} at the far end");
    assert!(preferences.right() <= discord.left() + 0.5, "Preferences, then Discord, mirroring their order");
    assert!(discord.right() < rect("Open").left(), "Discord stays clear of Open");
}

/// Without the opt-in the bar keeps its order and only the lights' room moves to the right end, so nothing sits under
/// them; with the lights at the left the bar is as it always was.
#[trial]
fn the_lights_room_follows_the_lights_without_mirroring_the_bar() {
    let (width, lights) = (800.0, craft_ui::shell::MacInset::default().leading);
    let rect = lights_right_bar(false, true);
    assert!(rect("Home").left() < 20.0, "Home {:?} keeps the start", rect("Home"));
    assert!(rect("first.pdf").left() >= rect("Home").right() - 0.5, "the tabs read from the left");
    let preferences = rect("Preferences");
    assert!(preferences.right() <= width - lights + 0.5, "Preferences {preferences:?} clears the lights at the right");
    let rect = lights_right_bar(true, false);
    assert!(rect("Home").left() >= lights - 0.5, "Home {:?} after the lights at the left", rect("Home"));
    assert!(rect("Preferences").right() >= width - 20.0, "Preferences {:?} at the right end", rect("Preferences"));
}

/// A Windows title bar with a brand button, a menu bar of an image editor's menus and the document title.
fn whole_bar(ui: &mut Ui) {
    use craft_ui::buttons::Button;
    use craft_ui::contract::Show as _;
    use craft_ui::menu::MenuItem;
    use craft_ui::menu::bar::{MenuBar, MenuTitle};
    let layout = craft_window::Layout { left: vec![], right: vec![Caption::Minimize, Caption::Maximize, Caption::Close] };
    let frame = Frame::Integrated(Captions { family: CaptionFamily::Windows, layout });
    let _ = TitleBar::new().frame(frame).show_titled(
        ui,
        Title::new("Untitled-1.psd"),
        |ui, _| {
            Button::new("Brand").show(ui);
            let names = ["&File", "&Edit", "&Image", "&Layer", "&Type", "&Select", "Fil&ter", "&3D", "&View", "&Plugins", "&Window", "&Help"];
            let titles = names
                .into_iter()
                .map(|name| {
                    MenuTitle::new(name, |ui| {
                        MenuItem::new("Row").show(ui);
                    })
                })
                .collect();
            MenuBar::new("Main menu", titles).show(ui);
        },
        |_, _| {},
    );
}

/// The cursor over the File title and over the band of the whole bar in a window, with the band's cursor `look`.
fn cursors_over_the_bar(look: craft_ui::contract::MoveBandCursor) -> (Option<craft_behavior::Cursor>, Option<craft_behavior::Cursor>) {
    let layout = craft_window::Layout { left: vec![], right: vec![Caption::Minimize, Caption::Maximize, Caption::Close] };
    let frame = Frame::Integrated(Captions { family: CaptionFamily::Windows, layout });
    let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(NoWindows)).frame(Decision { frame, overridden: None, desktop }));
    let mut root = craft_ui::runtime::env::RootEnv::default();
    root.values.set(look);
    windows.root_env(root);
    let show = whole_bar;
    windows.frame(vec![], vec![], show);
    let settled = windows.frame(vec![], vec![], show);
    let tree = &settled.trees.iter().rev().find(|(viewport, _)| *viewport == ViewportId::ROOT).expect("the root pass ran").1;
    let title = |name: &str| {
        tree.with_role(Role::MENU_ITEM).into_iter().find(|key| tree.name_of(*key) == Some(name)).and_then(|key| tree.bounds_of(key)).expect("a title")
    };
    let (file, help) = (title("File"), title("Help"));
    let over = |at: Pos2| {
        windows.frame(vec![moved(at)], vec![], show);
        windows.frame(vec![], vec![], show);
        craft_ui::runtime::cursor::current(&windows.ctx)
    };
    (over(file.center()), over(Pos2::new(help.right() + 40.0, help.center().y)))
}

/// A title bar's move band shows the open hand by default, also through the menu titles on it, which sense clicks
/// only; under `MoveBandCursor::Arrow` the band and the titles show the arrow, as an OS title bar does.
#[trial]
fn a_move_band_shows_the_arrow_when_the_app_asks() {
    use craft_behavior::{Cursor, CursorIcon};
    use craft_ui::contract::MoveBandCursor;
    let grab = Some(Cursor::System(CursorIcon::Grab));
    assert_eq!(cursors_over_the_bar(MoveBandCursor::Hand), (grab.clone(), grab), "the hand over the band and the titles on it");
    assert_eq!(cursors_over_the_bar(MoveBandCursor::Arrow), (None, None), "the arrow over the band and the titles on it");
}

/// A whole title bar in the shell's versioned title slot, which replays it as a memoized view (1600 × 1000 px at
/// 125 %), fills the slot as the bar shown directly does: the captions end at the slot's end edge and the document
/// title is drawn whole after the last menu.
#[trial]
fn a_memoized_title_bar_fills_the_title_slot() {
    use craft_host::target::Os;
    let slot = Rc::new(Cell::new(Rect::NOTHING));
    let seen = Rc::clone(&slot);
    let mut fixture = Fixture::new(move |ui| {
        let shown = AppShell::new(WindowKey::main()).title_bar(|_| Some(0), |ui, _, _: &mut ()| whole_bar(ui)).body(|_, _, _| {}).show(ui, &mut ());
        if let Some(rect) = shown.regions.slots.iter().find(|(key, _)| *key == "title").map(|(_, rect)| *rect) {
            seen.set(rect);
        }
    });
    fixture.os(Os::Windows);
    fixture.ctx().set_pixels_per_point(1.25);
    fixture.room(vec2(1280.0, 800.0)).settle();
    let slot = slot.get();
    let close = fixture.by_label("Close").expect("the Close caption").rect;
    assert!((close.right() - slot.right()).abs() <= 1.0, "the Close caption {close:?} ends at the slot's end {slot:?}");
    let help = fixture.by_label("Help").expect("Help is a title of the menu bar").rect;
    let (title, elided) =
        fixture.texts().iter().find(|run| run.text == "Untitled-1.psd").map(|run| (run.rect, run.elided)).expect("the document title is painted");
    assert!(title.left() > help.right() && title.right() < close.left(), "the title {title:?} lies between Help {help:?} and the captions");
    assert!(!elided, "the document title is whole");
}

/// What a bar with Windows captions `width` wide shows in `direction`, with its trailing content fitted after the
/// leading content or, without `fitted`, in the end region, by label: the leading button, the trailing buttons ranked 1
/// and 2 when they are there, the unranked trailing button and the Minimize caption.
fn fitted(width: f32, direction: craft_i18n::Direction, fitted: bool) -> impl Fn(&str) -> Option<Rect> {
    use craft_ui::buttons::Button;
    use craft_ui::contract::{BodyCx, Show as _, ShowContainer as _};
    use craft_ui::layout_components::Row;
    let mut fixture = Fixture::new(move |ui| {
        let layout = craft_window::Layout { left: vec![], right: vec![Caption::Minimize, Caption::Maximize, Caption::Close] };
        let frame = Frame::Integrated(Captions { family: CaptionFamily::Windows, layout });
        let leading = |ui: &mut Ui, _: &mut BodyCx<'_>| {
            Button::new(MENUS).show(ui);
        };
        let trailing = |ui: &mut Ui, _: &mut BodyCx<'_>| {
            let _ = Row::new().fit_rank(1).show(ui, |ui, _| {
                Button::new("Community").show(ui);
            });
            let _ = Row::new().fit_rank(2).show(ui, |ui, _| {
                Button::new("Learn").show(ui);
            });
            Button::new("Share").show(ui);
        };
        let _ = if fitted {
            TitleBar::new().frame(frame).show_fitted(ui, leading, trailing)
        } else {
            TitleBar::new().frame(frame).trailing(trailing).show(ui, leading).output
        };
    });
    fixture.direction(direction).room(vec2(width, 80.0)).settle();
    move |label: &str| fixture.by_label(label).map(|node| node.rect)
}

/// The leading button of the fitted bar trial: menus wider than the trailing content.
const MENUS: &str = "File Edit Object Type Select Effect View Window Help";

/// A fitted bar's trailing content takes what the leading content leaves: as the bar narrows, its ranked members
/// leave whole, lowest rank first, while the leading content keeps its width; a ranked member that stays keeps its
/// whole width, even where the member before it took the room; and the rest of the trailing content stays at the end,
/// clear of the captions and no further from them than a wide unfitted bar puts it. Right to left places it the same,
/// mirrored.
#[trial]
fn a_fitted_bar_drops_its_trailing_members_by_rank_before_the_leading_content_narrows() {
    use craft_i18n::Direction;
    let wide = fitted(1400.0, Direction::Ltr, true);
    let (whole, learn) = (wide(MENUS).expect("the leading button"), wide("Learn").expect("Learn on a wide bar"));
    assert!(wide("Community").is_some(), "a wide bar shows every trailing member");
    let unfitted = fitted(1400.0, Direction::Ltr, false);
    let end_gap = unfitted("Minimize").expect("Minimize").left() - unfitted("Share").expect("Share").right();
    let mut dropped_alone = false;
    for width in (360..=900).step_by(30).map(|width| width as f32) {
        let rect = fitted(width, Direction::Ltr, true);
        let (leading, share, minimize) =
            (rect(MENUS).expect("the leading button"), rect("Share").expect("Share"), rect("Minimize").expect("Minimize"));
        let (low, high) = (rect("Community").is_some(), rect("Learn"));
        assert!(!(low && high.is_none()), "rank 2 left before rank 1 at {width}");
        dropped_alone |= !low && high.is_some();
        if let Some(high) = high {
            assert!(leading.width() >= whole.width() - 0.5, "the leading content narrowed to {leading:?} at {width} while a ranked member stayed");
            assert!(high.width() >= learn.width() - 0.5, "Learn narrowed to {high:?} at {width} instead of leaving");
        }
        let gap = minimize.left() - share.right();
        assert!((-0.5..=end_gap + 0.5).contains(&gap), "Share {share:?} stands {gap} before the captions at {width}, an unfitted bar {end_gap}");
        let mirrored = fitted(width, Direction::Rtl, true);
        assert_eq!(
            [mirrored("Community").is_some(), mirrored("Learn").is_some()],
            [low, high.is_some()],
            "right to left keeps the same members at {width}"
        );
        let (rtl_leading, rtl_share) = (mirrored(MENUS).expect("the leading button"), mirrored("Share").expect("Share"));
        assert!((rtl_share.left() - (width - share.right())).abs() <= 0.5, "Share {rtl_share:?} mirrors {share:?} at {width}");
        assert!((rtl_leading.right() - (width - leading.left())).abs() <= 0.5, "the leading content {rtl_leading:?} mirrors {leading:?} at {width}");
    }
    assert!(dropped_alone, "rank 1 never left alone");
}
