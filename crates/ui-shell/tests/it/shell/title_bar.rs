//! A title bar in an embedded shell is a picture of a window: it reports its captions and acts on no window. An action
//! title is a button that reports its activation. A double click on a control in the bar is the control's.

use craft_host::target::{Os, ShellInfo};
use craft_ui::buttons::Button;
use craft_ui::contract::{Props as _, Role, Show as _};
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::env::WindowKey;
use craft_ui::shell::{AppShell, Caption, CaptionFamily, Captions, Frame, MacInset, Title, TitleBar};
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

/// The window commands a double click on each of a macOS title bar's controls sends, with each control's name: an
/// enabled and a disabled button in its leading content, then the same in its trailing content; last, the free band.
fn mac_double_clicks_send() -> Vec<(&'static str, Vec<ViewportCommand>)> {
    let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
    let decision = Decision { frame: Frame::Inset(MacInset::default()), overridden: None, desktop };
    let windows = Windows::with(InstallSpec::new(ShellInfo::default().os(Os::Mac), Arc::new(NoWindows)).frame(decision));
    let names = ["Lead", "Lead off", "Trail", "Trail off", "the band"];
    let rects = [(); 5].map(|()| Cell::new(Rect::NOTHING));
    let frame = |events: Vec<Event>| {
        windows.frame(events, vec![], |ui| {
            let shown = TitleBar::new().show_titled(
                ui,
                Title::new("Landscape.psd"),
                |ui, _| {
                    rects[0].set(Button::new(names[0]).show(ui).rect);
                    rects[1].set(Button::new(names[1]).enabled(false).show(ui).rect);
                },
                |ui, _| {
                    rects[2].set(Button::new(names[2]).show(ui).rect);
                    rects[3].set(Button::new(names[3]).enabled(false).show(ui).rect);
                },
            );
            // The title is no control: the band lies under it.
            rects[4].set(Rect::from_center_size(shown.response.rect.center(), vec2(1.0, 1.0)));
        })
    };
    frame(vec![]);
    frame(vec![]);
    let mut sent = Vec::new();
    for (name, rect) in names.into_iter().zip(&rects) {
        let at = rect.get().center();
        let mut commands = Vec::new();
        for events in [vec![moved(at)], vec![press(at)], vec![release(at)], vec![press(at)], vec![release(at)]] {
            commands.extend(
                frame(events)
                    .commands
                    .into_iter()
                    .map(|(_, command)| command)
                    .filter(|command| matches!(command, ViewportCommand::Maximized(_) | ViewportCommand::Minimized(_))),
            );
        }
        // Enough passes that the next control's first click is no third click of this one.
        for _ in 0..30 {
            frame(vec![]);
        }
        sent.push((name, commands));
    }
    sent
}

/// A double click on a control in a macOS title bar, enabled or disabled, at its start or its end, is the control's:
/// the window does not zoom. One on the free band still zooms it.
#[trial]
fn a_double_click_on_a_title_bar_control_leaves_the_window_alone() {
    let mut sent = mac_double_clicks_send();
    let band = sent.pop().expect("the band was double-clicked last");
    assert!(sent.iter().all(|(_, commands)| commands.is_empty()), "a double click on a control sends nothing to the window: {sent:?}");
    assert!(matches!(band.1.as_slice(), [ViewportCommand::Maximized(true)]), "a double click on the band zooms: {band:?}");
}
