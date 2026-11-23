//! A title bar in an embedded shell is a picture of a window: it reports its captions and acts on no window.

use craft_host::target::ShellInfo;
use craft_ui::contract::Role;
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::env::WindowKey;
use craft_ui::shell::{AppShell, Caption, CaptionFamily, Captions, Frame, Title, TitleBar};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::trial;
use craft_ui::testing::windows::NoWindows;
use craft_ui_test_kit::windows::Windows;
use craft_window::policy::{Decision, Desktop, DesktopKind};
use egui::{Event, Pos2, Rect, Ui, ViewportCommand, ViewportId, ViewportInfo, vec2};
use std::cell::Cell;
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
                let _ = AppShell::embedded(WindowKey::new("preview")).title_bar(|ui, _, _: &mut ()| show_bar(ui)).show(ui, &mut ());
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
                DocumentTabs::new("Documents", &tabs).reorderable(true).selected(0).show(ui);
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
                DocumentTabs::new("Documents", &tabs)
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
