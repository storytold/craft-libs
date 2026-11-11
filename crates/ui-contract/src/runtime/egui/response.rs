//! `Response` construction for members and the root's `Response` flag rewrite, and the root's egui widgets.

use craft_behavior::Sense;
use egui::response::Flags;
use egui::{Id, LayerId, Pos2, Rect, Response, Ui, Vec2};

/// Whether the `Response` reports a click.
pub(crate) fn clicked(response: &Response) -> bool {
    response.clicked()
}

/// Whether the press path released a pointer click on the component of `response`: a root's own widget or a
/// member's shape, never a key or AccessKit activation.
pub(crate) fn pointer_clicked(response: &Response) -> bool {
    response.flags.contains(Flags::CLICKED)
}

/// egui's sense for a library sense: `FOCUS` makes the widget a focus stop.
pub(crate) fn to_egui(sense: Sense) -> egui::Sense {
    let mut mapped = egui::Sense::hover();
    if sense.contains(Sense::CLICK) {
        mapped |= egui::Sense::click();
    }
    if sense.contains(Sense::DRAG) {
        mapped |= egui::Sense::drag();
    }
    if sense.contains(Sense::FOCUS) {
        mapped |= egui::Sense::focusable_noninteractive();
    }
    mapped
}

/// A root's egui widget over `rect`, clipped by the caller. egui gives a widget of a disabled `Ui` no focus stop and
/// drops the focus it holds; the sense the pipeline passes decides whether the root is a stop (a disabled control with
/// a reason is), so the stop and the focus are kept here.
pub(crate) fn widget(ui: &Ui, id: Id, rect: Rect, sense: Sense) -> Response {
    let ctx = ui.ctx();
    let held = ctx.memory(|memory| memory.has_focus(id));
    let response = ui.interact(rect, id, to_egui(sense));
    if sense.contains(Sense::FOCUS) && !ui.is_enabled() {
        ctx.memory_mut(|memory| {
            memory.interested_in_focus(id, ui.layer_id());
            if held {
                memory.request_focus(id);
            }
        });
    }
    response
}

/// Whether a pointer button went down on `response`'s widget and is still down.
pub(crate) fn is_pressed(response: &Response) -> bool {
    response.is_pointer_button_down_on()
}

/// What a member's `Response` reports, from the arena's resolution.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MemberView {
    pub hovered: bool,
    /// The member holds the open press.
    pub pressed: bool,
    pub drag_started: bool,
    pub dragged: bool,
    /// The press ended without a click.
    pub drag_stopped: bool,
    pub clicked: bool,
    /// The pointer position in the member's layer space.
    pub pointer: Option<Pos2>,
}

/// A member's `Response`, built from egui's public fields. It registers no egui widget.
pub(crate) fn member(ui: &Ui, id: Id, rect: Rect, enabled: bool, view: MemberView) -> Response {
    member_on(ui.ctx(), ui.layer_id(), ui.clip_rect(), id, rect, enabled, view)
}

fn member_on(ctx: &egui::Context, layer_id: LayerId, clip: Rect, id: Id, rect: Rect, enabled: bool, view: MemberView) -> Response {
    let mut flags = Flags::empty();
    let hovered = enabled && view.hovered;
    let mine = enabled && view.pressed;
    if enabled {
        flags |= Flags::ENABLED;
    }
    if hovered {
        flags |= Flags::CONTAINS_POINTER | Flags::HOVERED;
    }
    if mine {
        flags |= Flags::IS_POINTER_BUTTON_DOWN_ON;
        if view.drag_started {
            flags |= Flags::DRAG_STARTED | Flags::DRAGGED;
        } else if view.dragged {
            flags |= Flags::DRAGGED;
        }
        if view.drag_stopped {
            flags |= Flags::DRAG_STOPPED;
        }
    }
    if enabled && view.clicked {
        flags |= Flags::CLICKED | Flags::HOVERED;
    }
    Response {
        ctx: ctx.clone(),
        layer_id,
        id,
        rect,
        interact_rect: rect.intersect(clip),
        sense: egui::Sense::hover(),
        interact_pointer_pos_or_nan: if hovered || mine { view.pointer.unwrap_or(Pos2::NAN) } else { Pos2::NAN },
        intrinsic_size_or_nan: Vec2::NAN,
        flags,
    }
}

/// Removes egui's click and drag flags from the root's `Response` unless the root's own shapes won the press:
/// a press on a member belongs to the member, and a press the press-target rule refused belongs to the widget
/// egui chose.
pub(crate) fn keep_press_flags(response: &mut Response, own: bool) {
    if !own {
        response.flags.remove(
            Flags::CLICKED
                | Flags::FAKE_PRIMARY_CLICKED
                | Flags::LONG_TOUCHED
                | Flags::DRAG_STARTED
                | Flags::DRAGGED
                | Flags::DRAG_STOPPED
                | Flags::IS_POINTER_BUTTON_DOWN_ON,
        );
    }
}

/// Rewrites the click flags of the root's `Response` at step 17: egui's fake click on Enter, Space or AccessKit
/// `Click` is cleared, and set again only when the pipeline activated the component by key or by an AccessKit click on a part. A command-bound
/// component reports no click, because the command fires instead.
pub(crate) fn rewrite_clicks(response: &mut Response, activated_by_key: bool, command_bound: bool) {
    response.flags.remove(Flags::FAKE_PRIMARY_CLICKED);
    if activated_by_key {
        response.flags.insert(Flags::FAKE_PRIMARY_CLICKED);
    }
    if command_bound {
        response.flags.remove(Flags::CLICKED | Flags::FAKE_PRIMARY_CLICKED);
    }
}

/// An inert `Response` for the component `id`: no widget, no interaction. It stands in for the root widget until
/// a pass creates it, so the step-17 id assertion holds when no pass ran.
pub(crate) fn inert(ui: &Ui, id: Id) -> Response {
    Response {
        ctx: ui.ctx().clone(),
        layer_id: ui.layer_id(),
        id,
        rect: Rect::from_min_size(ui.next_widget_position(), Vec2::ZERO),
        interact_rect: Rect::NOTHING,
        sense: egui::Sense::hover(),
        interact_pointer_pos_or_nan: Pos2::NAN,
        intrinsic_size_or_nan: Vec2::NAN,
        flags: Flags::empty(),
    }
}

/// Marks the response as changed: the component's value changed in this pass.
pub(crate) fn mark_changed(response: &mut Response) {
    response.mark_changed();
}
