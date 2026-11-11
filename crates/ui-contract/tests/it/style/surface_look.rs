//! What a surface look shows in each state: its kind, its edges and its fill.

use craft_theme::{Contrast, ControlSize, Mode, Variant};
use craft_ui::style::{Flags, State, StateMix, StateSet, Stateful, StyleColor, SurfaceKind, SurfaceLook};
use craft_ui::testing::style::{tokens, with_cx};
use craft_ui::testing::trial;
use egui::Color32;

#[trial]
fn a_toggle_look_sinks_while_on_or_pressed_and_a_mixed_one_stays_raised() {
    let look = SurfaceLook::new(Stateful::new(StyleColor(Color32::GRAY)))
        .kind(SurfaceKind::Raised)
        .kind_down(SurfaceKind::Pressed)
        .kind_on(SurfaceKind::Pressed);
    let at = |state: State, flags: Flags| look.kind_in(&StateSet::new(state, flags, StateMix::REST));
    assert_eq!(at(State::Rest, Flags::empty()), Some(SurfaceKind::Raised));
    assert_eq!(at(State::Pressed, Flags::empty()), Some(SurfaceKind::Pressed), "a press sinks it");
    assert_eq!(at(State::Hover, Flags::CHECKED), Some(SurfaceKind::Pressed), "on sinks it");
    assert_eq!(at(State::Rest, Flags::CHECKED | Flags::MIXED), Some(SurfaceKind::Raised), "mixed shows the off look");
    let segment = SurfaceLook::new(Stateful::new(StyleColor(Color32::GRAY))).kind_on(SurfaceKind::Raised);
    let at = |state: State, flags: Flags| segment.kind_in(&StateSet::new(state, flags, StateMix::REST));
    assert_eq!(at(State::Pressed, Flags::empty()), None, "a pressed off segment draws no surface");
    assert_eq!(at(State::Rest, Flags::SELECTED), Some(SurfaceKind::Raised), "the selected segment rises");
}

#[trial]
fn a_focused_invalid_field_keeps_its_focus_edge_with_the_failure_line_inside() {
    let ctx = egui::Context::default();
    let tokens = tokens(Mode::Dark, Contrast::Standard);
    with_cx(&ctx, &tokens, Variant::SECONDARY, ControlSize::Md, |cx| {
        let look = cx.field();
        let (edge, inner) = (look.border.as_ref().expect("a field draws an edge"), look.inner.as_ref().expect("a field has an inner line"));
        let at = |flags: Flags| StateSet::new(State::Rest, flags, StateMix::REST);
        let (focused, invalid, both) = (at(Flags::FOCUSED), at(Flags::INVALID), at(Flags::FOCUSED | Flags::INVALID));
        assert_eq!(both.resolve(edge), focused.resolve(edge), "focus keeps its edge while the value is invalid");
        assert_eq!(both.resolve(inner), invalid.resolve(edge), "the failure colour shows inside the focus edge");
        assert_eq!(invalid.resolve(inner).egui().a(), 0, "an unfocused invalid field shows the failure colour on its edge alone");
    });
}

#[trial]
fn a_dropdown_box_fills_under_the_pointer_and_while_open() {
    let ctx = egui::Context::default();
    let tokens = tokens(Mode::Dark, Contrast::Standard);
    with_cx(&ctx, &tokens, Variant::SECONDARY, ControlSize::Md, |cx| {
        let look = cx.dropdown();
        let at = |state: State, flags: Flags| StateSet::new(state, flags, StateMix::REST).resolve(&look.fill);
        let rest = at(State::Rest, Flags::empty());
        let hover = StateSet::new(State::Hover, Flags::empty(), StateMix::new(1.0, 0.0, 0.0)).resolve(&look.fill);
        // An open popup counts as pressed (`StateSet::state`).
        let open = StateSet::new(State::Pressed, Flags::OPEN, StateMix::new(0.0, 1.0, 0.0)).resolve(&look.fill);
        assert_ne!(hover, rest, "the box lights under the pointer");
        assert_eq!(open, hover, "an open box keeps the hover fill");
    });
}
