//! A title bar in an embedded shell is a picture of a window: it reports its captions and acts on no window. An action
//! title is a button that reports its activation.

use craft_host::target::{Backend, Os, ShellInfo};
use craft_ui::contract::{Role, Show as _};
use craft_ui::menu::bar::{MenuBar, MenuTitle};
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::env::WindowKey;
use craft_ui::shell::{AppShell, Caption, CaptionFamily, Captions, Frame, Title, TitleBar};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::windows::NoWindows;
use craft_ui::testing::{Fixture, Frames, install_spec, trial};
use craft_ui_test_kit::windows::Windows;
use craft_window::policy::{Decision, Desktop, DesktopKind};
use egui::{CursorIcon, Event, Pos2, Rect, Ui, ViewportCommand, ViewportId, ViewportInfo, vec2};
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

/// Frames of a Linux X11 window with an app-drawn frame.
fn linux_window() -> Frames {
    let layout = craft_window::Layout { left: vec![], right: vec![Caption::Minimize, Caption::Maximize, Caption::Close] };
    let frame = Frame::Integrated(Captions { family: CaptionFamily::Windows, layout });
    let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
    let mut spec = install_spec().frame(Decision { frame, overridden: None, desktop });
    spec.shell_info = ShellInfo::default().os(Os::Linux).backend(Backend::X11);
    let frames = Frames::uninstalled();
    craft_ui::install(&frames.ctx, spec).expect("the runtime installs once");
    frames
}

/// After an OS move the cursor changes for one frame, even if its first pass is discarded, and returns to the arrow:
/// egui-winit sends only a changed cursor, and the window would keep the compositor's.
#[trial]
fn a_title_bar_sends_the_arrow_again_after_an_os_move() {
    for discarded in [false, true] {
        let mut frames = linux_window();
        let bar = Cell::new(Rect::NOTHING);
        let discard = Cell::new(false);
        let mut show = |ui: &mut Ui| {
            if discard.take() {
                ui.ctx().request_discard("the trial reruns the pass");
            }
            bar.set(TitleBar::new().show_titled(ui, Title::new("Landscape.psd"), |_, _| {}, |_, _| {}).response.rect);
        };
        frames.frame(vec![], &mut show);
        frames.frame(vec![], &mut show);
        let at = bar.get().center();
        for events in [vec![moved(at)], vec![press(at)], vec![moved(at + vec2(40.0, 0.0))], vec![]] {
            frames.frame(events, &mut show);
        }
        discard.set(discarded);
        let passes = frames.frame(vec![moved(at)], &mut show);
        assert!(!discarded || passes > 1, "the frame reruns its discarded pass");
        assert_ne!(frames.cursor_icon(), CursorIcon::Default, "the first motion after the move changes the cursor");
        frames.frame(vec![], &mut show);
        assert_eq!(frames.cursor_icon(), CursorIcon::Default, "the next frame sends the arrow");
    }
}

/// A window's own title bar shows the arrow over its free band and its menu titles, as an OS title bar does.
#[trial]
fn a_window_title_bar_keeps_the_arrow() {
    let mut frames = linux_window();
    let bar = Cell::new(Rect::NOTHING);
    let mut show = |ui: &mut Ui| {
        let shown = TitleBar::new().show_titled(
            ui,
            Title::new("Landscape.psd"),
            |ui, _| {
                MenuBar::new("Main menu", vec![MenuTitle::new("File", |_| {})]).show(ui);
            },
            |_, _| {},
        );
        bar.set(shown.response.rect);
    };
    frames.frame(vec![], &mut show);
    frames.frame(vec![], &mut show);
    let tree = frames.tree();
    let file = tree.keys().into_iter().find(|key| tree.name_of(*key) == Some("File")).and_then(|key| tree.bounds_of(key));
    let file = file.expect("the bar shows its File title").center();
    for at in [bar.get().center(), file] {
        frames.frame(vec![moved(at)], &mut show);
        frames.frame(vec![], &mut show);
        assert_eq!(frames.cursor_icon(), CursorIcon::Default, "the cursor over {at:?}");
    }
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
