//! Focus after the focused stop goes away.

use craft_ui::contract::{ContainerProps, Props as _, Scroll, Show as _, ShowContainer as _};
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::testing::layout::{Block, in_region};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::body::Slab;
use craft_ui_test_kit::contract::{Probe, Seen};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

const ORIGIN: Pos2 = Pos2::new(10.0, 10.0);
const VIEW: Vec2 = Vec2::new(200.0, 100.0);

/// The focused stop goes away while the next stop in its scope lies far below a scrolling body's view: the focus goes
/// nowhere and the body stays where it is (#15555: the rulers' corner panned the canvas when a closed text editor
/// handed it the focus).
#[trial]
fn a_removed_stop_hands_its_focus_to_no_neighbour_and_the_body_stays() {
    let (first, next) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()));
    let shown = Cell::new(true);
    let top = Cell::new(0.0f32);
    let mut show = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(ORIGIN, VIEW), |ui| {
            Slab::new(ContainerProps::new().scroll(Scroll::vertical())).id_salt("panel").show(ui, |ui, _| {
                top.set(ui.next_widget_position().y);
                if shown.get() {
                    Probe::new(&first).label("First").id_salt("first").show(ui);
                }
                Block::new(Vec2::new(VIEW.x, VIEW.y * 4.0)).show(ui);
                Probe::new(&next).label("Next").id_salt("next").show(ui);
            });
        });
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    let first_id = first.borrow().id.expect("the first stop ran");
    request(&frames.ctx, FocusTarget::Id(first_id));
    frames.idle(3, &mut show);
    assert_eq!(craft_ui::testing::focus::egui_focused(&frames.ctx), Some(first_id), "the first stop took the focus");
    let start = top.get();
    shown.set(false);
    frames.idle(6, &mut show);
    assert!(next.borrow().id.is_some(), "the next stop ran");
    assert_eq!(craft_ui::testing::focus::egui_focused(&frames.ctx), None, "no neighbour inherits the focus");
    assert!((top.get() - start).abs() < 0.5, "the body keeps its offset: content top {} from {start}", top.get());
}

/// A component that asks for the focus in the pass of a press on another component keeps it: the press's own focus
/// move at the pass's end does not take it back, as a text editor that appears under a click on the canvas and asks
/// for the keys keeps them. Guards against the end hook's press move overriding every request of the pass but a
/// layer's that appeared in it.
#[trial]
fn a_request_in_a_press_pass_keeps_the_focus_from_the_press() {
    use craft_behavior::{Input, Sense};
    use craft_ui::runtime::focus::KeyUse;
    use craft_ui::testing::focus::{Stop, egui_focused, named_id, press_shape, register_stop, widget};
    let (canvas, editor) = (named_id("press_canvas"), named_id("press_editor"));
    let at = Cell::new(Rect::NOTHING);
    let mut show = |ui: &mut Ui| {
        let ctx = ui.ctx().clone();
        let rect = widget(ui, canvas);
        register_stop(&ctx, Stop::new(canvas, KeyUse::CANVAS));
        press_shape(ui, rect, Sense::CLICK | Sense::TAKE_FOCUS, canvas);
        at.set(rect);
        widget(ui, editor);
        register_stop(&ctx, Stop::new(editor, KeyUse::of(Input::Text)));
        if ctx.input(|input| input.pointer.primary_pressed()) {
            request(&ctx, FocusTarget::Id(editor));
        }
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    craft_ui::testing::pointer::click(&mut frames, at.get().center(), &mut show);
    assert_eq!(egui_focused(&frames.ctx), Some(editor), "the request of the press's pass holds");
}

/// A popover opened with nothing focused closes while a stop in it holds the focus: from the first pass of its exit
/// fade, while its body still draws, the focus is nowhere, so a key pressed during the fade is not the stop's
/// (VectorCraft: Cmd+A after the Artboards flyout closed still selected the list's rows).
#[trial]
fn a_stop_in_a_layer_that_fades_out_loses_the_focus_at_once() {
    use craft_behavior::overlay::{Align, Side};
    use craft_ui::contract::overlay::{Anchor, Place};
    use craft_ui::contract::{Mount, Overlay, Sizing};
    let seen = RefCell::new(Seen::default());
    let (close, drawn) = (Cell::new(false), Cell::new(0u32));
    let mut show = |ui: &mut Ui| {
        let overlay = Overlay::popover().place(Place::anchored(Anchor::Rect(Rect::from_min_size(ORIGIN, VIEW)), Side::Below, Align::Start));
        Slab::new(ContainerProps::new().mount(Mount::Layer(overlay)).sizing(Sizing::Content)).id_salt("flyout").show(ui, |ui, cx| {
            drawn.set(drawn.get() + 1);
            if close.get() {
                cx.request_close();
            }
            Probe::new(&seen).label("List").id_salt("list").show(ui);
        });
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    let list = seen.borrow().id.expect("the list ran");
    request(&frames.ctx, FocusTarget::Id(list));
    frames.idle(2, &mut show);
    assert_eq!(craft_ui::testing::focus::egui_focused(&frames.ctx), Some(list), "the list took the focus");
    close.set(true);
    frames.idle(1, &mut show);
    close.set(false);
    drawn.set(0);
    frames.idle(1, &mut show);
    assert!(drawn.get() > 0, "the layer still draws its fade");
    assert_eq!(craft_ui::testing::focus::egui_focused(&frames.ctx), None, "the fading layer's stop has no focus");
}
