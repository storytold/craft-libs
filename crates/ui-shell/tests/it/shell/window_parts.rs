//! The caption record: exclusive parts tagged with their window role read back through
//! `WindowHost::caption_regions`.

use craft_behavior::{PartKey, Sense};
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PartsCx, Props as _, Role, Show as _};
use craft_ui::runtime::windows::{ResizeEdge, WindowHit, WindowHost};
use craft_ui::shell::Caption;
use craft_ui::testing::trial;
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
