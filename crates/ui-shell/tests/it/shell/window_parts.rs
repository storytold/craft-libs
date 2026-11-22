//! The caption record: exclusive parts tagged with their window role read back through
//! `WindowHost::caption_regions`.

use craft_behavior::{PartKey, Sense};
use craft_ui::contract::{Common, Component, Cx, MeasureCx, MemoView, PartsCx, Props as _, Role, Show as _};
use craft_ui::runtime::windows::{ResizeEdge, WindowHit, WindowHost};
use craft_ui::shell::Caption;
use craft_ui::testing::{MemoRun, last_pass_memo_runs, trial};
use craft_ui_test_kit::contract::PlainStyle;
use craft_ui_test_kit::windows::Windows;
use egui::{Rect, Response, Vec2, ViewportId, pos2, vec2};

const MOVE: PartKey = PartKey::new("move");
const CLOSE: PartKey = PartKey::new("close");
const EDGE: PartKey = PartKey::new("edge");

const BAR: Vec2 = Vec2::new(400.0, 32.0);
const BUTTON: f32 = 32.0;
const GRIP: f32 = 4.0;

/// A title bar: a move band over the whole bar, a close button at its end and a grip along its top edge.
#[derive(craft_ui::contract::Component)]
struct Bar {
    #[common]
    common: Common,
}

impl Component for Bar {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &craft_ui::style::StyleCx<'_>, _: BarStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        BAR
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(MOVE, rect).sense(Sense::DRAG).exclusive(WindowHit::Move).pointer_only("the window command set moves the window");
        cx.part(CLOSE, close_of(rect)).sense(Sense::CLICK).exclusive(WindowHit::Caption(Caption::Close)).pointer_only("the Close command");
        cx.part(EDGE, top_of(rect)).sense(Sense::DRAG).exclusive(WindowHit::Resize(ResizeEdge::Top)).pointer_only("the Size command");
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

fn close_of(bar: Rect) -> Rect {
    Rect::from_min_size(pos2(bar.max.x - BUTTON, bar.min.y), vec2(BUTTON, bar.height()))
}

fn top_of(bar: Rect) -> Rect {
    Rect::from_min_size(bar.min, vec2(bar.width(), GRIP))
}

/// Parts tagged exclusive show up in the window's record with their role, and the hit test prefers a button to an
/// edge and an edge to the drag area. A window that registered none, or that stopped, has no record.
#[trial]
fn exclusive_parts_are_the_caption_record() {
    let windows = Windows::new();
    let host = WindowHost::of(&windows.ctx);
    let other = ViewportId::from_hash_of("other");
    let mut bar = Rect::NOTHING;
    for _ in 0..3 {
        windows.frame(vec![], vec![], |ui| bar = Bar { common: Common::new() }.id_salt("bar").show(ui).rect);
    }
    let regions = host.caption_regions(ViewportId::ROOT).expect("the bar registered its parts");
    assert_eq!(regions.buttons.iter().map(|(button, _)| *button).collect::<Vec<_>>(), [Caption::Close]);
    assert_eq!(regions.edges.iter().map(|(edge, _)| *edge).collect::<Vec<_>>(), [ResizeEdge::Top]);
    let rects = regions.buttons.iter().map(|(_, rect)| rect).chain(regions.edges.iter().map(|(_, rect)| rect)).chain(&regions.moves);
    assert!(rects.clone().count() == 3 && rects.into_iter().all(|rect| bar.contains_rect(*rect)), "the parts lie in the bar: {regions:?}");
    assert_eq!(regions.hit(close_of(bar).center()), Some(WindowHit::Caption(Caption::Close)), "a button wins over the drag area");
    assert_eq!(regions.hit(top_of(bar).center()), Some(WindowHit::Resize(ResizeEdge::Top)), "an edge wins over the drag area");
    assert_eq!(regions.hit(bar.center()), Some(WindowHit::Move));
    assert_eq!(regions.hit(bar.max + bar.size()), None);
    assert_eq!(host.caption_regions(other), None, "another window has its own record");
    for _ in 0..3 {
        windows.frame(vec![], vec![], |_| {});
    }
    assert_eq!(host.caption_regions(ViewportId::ROOT), None, "a bar that stops registering leaves no record");
}

/// A title bar shown as a memoized view keeps its caption record while it replays its last run, so the Windows hit test
/// still finds its drag area and buttons.
#[trial]
fn a_replayed_bar_keeps_its_caption_record() {
    let windows = Windows::new();
    let host = WindowHost::of(&windows.ctx);
    let frame = || {
        windows.frame(vec![], vec![], |ui| {
            let _ = MemoView::new("bar", (), title_bar).show(ui);
        });
        last_pass_memo_runs(&windows.ctx).iter().any(|(_, _, run)| *run == MemoRun::Ran)
    };
    assert!(frame(), "the bar runs in its first frame");
    for _ in 0..3 {
        frame();
    }
    assert!(!frame(), "the bar replays");
    let regions = host.caption_regions(ViewportId::ROOT).expect("the replayed bar keeps its parts");
    assert_eq!(regions.buttons.len() + regions.edges.len() + regions.moves.len(), 3, "every part stays: {regions:?}");
}

fn title_bar(_: &mut (), ui: &mut egui::Ui) {
    Bar { common: Common::new() }.id_salt("bar").show(ui);
}

/// The maximize button under a window procedure that owns the pointer there (Windows 11 snap layouts answer the hit
/// test `HTMAXBUTTON`): the procedure reports hover, press and release through the sink, and the button shows and acts
/// as it does under the app's own pointer, so a release on it is a click and a press that ends elsewhere is none.
#[trial]
fn caption_events_from_the_window_procedure_drive_the_maximize_button() {
    use craft_ui::shell::{CaptionButton, CaptionFamily};
    use craft_window::native::{CaptionEvent, Hits};

    let windows = Windows::new();
    let maximize = |windows: &Windows| {
        let mut shown = None;
        windows.frame(vec![], vec![], |ui| shown = Some(CaptionButton::new(Caption::Maximize, CaptionFamily::Windows).id_salt("maximize").show(ui)));
        shown.expect("the button was shown")
    };
    for _ in 0..3 {
        maximize(&windows);
    }
    let sink = WindowHost::of(&windows.ctx).install_hit_sink(ViewportId::ROOT);
    let mut button = maximize(&windows);
    assert!(!button.hovered() && !button.is_pointer_button_down_on(), "the pointer is not on the button");

    sink.caption(CaptionEvent::Hover(Caption::Maximize));
    button = maximize(&windows);
    assert!(button.hovered(), "the reported hover lights the button");

    sink.caption(CaptionEvent::Press(Caption::Maximize));
    button = maximize(&windows);
    assert!(button.is_pointer_button_down_on(), "the reported press presses it");

    sink.caption(CaptionEvent::Release(Caption::Maximize));
    button = maximize(&windows);
    assert!(button.clicked(), "the reported release on the button is its click");

    sink.caption(CaptionEvent::Press(Caption::Maximize));
    maximize(&windows);
    sink.caption(CaptionEvent::Cancel);
    button = maximize(&windows);
    assert!(!button.clicked() && !button.is_pointer_button_down_on(), "a press that ended away from the button is no click");
}

/// The root window of an app on Windows that draws its own frame and hands the runtime its native window
/// (`embed::before_ui`), so the hit sink is installed: the platform's hit test finds a corner resize 3 points inside the
/// window's corner and the move area on the title bar, so the OS starts both at the press. A drag on the corner then
/// sends no resize of the app's.
#[trial]
fn with_the_hit_sink_the_window_region_answers_the_platforms_hit_test() {
    use craft_host::target::{Caps, Os, ShellInfo, WindowChrome, WindowInfo, WindowKind};
    use craft_ui::contract::ShowContainer as _;
    use craft_ui::runtime::InstallSpec;
    use craft_ui::shell::{CaptionFamily, Captions, Frame, ShellRegion, Title, TitleBar};
    use craft_ui::testing::pointer::{moved, press, release};
    use craft_ui::testing::windows::NoWindows;
    use craft_window::policy::{Decision, Desktop, DesktopKind};
    use egui::ViewportCommand;
    use std::cell::Cell;
    use std::sync::Arc;

    let shell = ShellInfo::default()
        .os(Os::Windows)
        .caps(Caps::native(Os::Windows))
        .window(WindowInfo::new(WindowKind::Main).chrome(WindowChrome::Integrated));
    let layout = craft_window::Layout { left: vec![], right: vec![Caption::Minimize, Caption::Maximize, Caption::Close] };
    let frame = Frame::Integrated(Captions { family: CaptionFamily::Windows, layout });
    let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
    let windows = Windows::with(InstallSpec::new(shell, Arc::new(NoWindows)).frame(Decision { frame, overridden: None, desktop }));
    let host = WindowHost::of(&windows.ctx);
    let _sink = host.install_hit_sink(ViewportId::ROOT);
    let (window, bar) = (Cell::new(Rect::NOTHING), Cell::new(Rect::NOTHING));
    let frame = |events: Vec<egui::Event>| -> Vec<ViewportCommand> {
        windows
            .frame(events, vec![], |ui| {
                let shown = ShellRegion::window().id_salt("root").show(ui, |ui, _| {
                    bar.set(TitleBar::new().show_titled(ui, Title::new("Landscape.psd"), |_, _| {}, |_, _| {}).response.rect);
                });
                window.set(shown.output.response.rect);
            })
            .commands
            .into_iter()
            .map(|(_, command)| command)
            .collect()
    };
    for _ in 0..3 {
        frame(vec![]);
    }
    let regions = host.caption_regions(ViewportId::ROOT).expect("the window registered its parts");
    let corner = window.get().max - vec2(3.0, 3.0);
    assert_eq!(regions.hit(corner), Some(WindowHit::Resize(ResizeEdge::BottomRight)), "the corner is a resize for the platform: {regions:?}");
    let band = bar.get().center();
    assert_eq!(regions.hit(band), Some(WindowHit::Move), "the title bar is the move area for the platform: {regions:?}");
    let mut sent = frame(vec![moved(corner)]);
    sent.extend(frame(vec![press(corner)]));
    for step in 1..=4 {
        sent.extend(frame(vec![moved(corner + vec2(10.0, 10.0) * step as f32)]));
    }
    sent.extend(frame(vec![release(corner + vec2(40.0, 40.0))]));
    assert!(sent.iter().all(|command| !matches!(command, ViewportCommand::BeginResize(_))), "the platform resizes the window, not the app: {sent:?}");
}
