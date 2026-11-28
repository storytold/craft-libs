//! Raw egui scenarios the `egui_surface` trials read: each runs a few frames on a bare context and returns what
//! egui did, so the trials pin the egui behaviours the runtime relies on without calling egui themselves.

pub use super::areas::{LayerOrder, layer_order};
use super::fixture;
use egui::{Context, Event, Plugin, Pos2, RawInput, Rect, Sense, Ui, ViewportBuilder, ViewportId};
use std::sync::{Arc, Mutex};

/// The screen the scenarios run on.
const SCREEN_WIDTH: f32 = 800.0;
const SCREEN_HEIGHT: f32 = 600.0;
/// The side of a click widget in the scenarios.
const WIDGET_SIDE: f32 = 40.0;

/// The rect of a click widget whose top left is at `(x, y)`.
fn widget_at(x: f32, y: f32) -> Rect {
    Rect::from_min_max(Pos2::new(x, y), Pos2::new(x + WIDGET_SIDE, y + WIDGET_SIDE))
}

pub(super) fn raw(time: f64, events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_max(Pos2::ZERO, Pos2::new(SCREEN_WIDTH, SCREEN_HEIGHT))),
        time: Some(time),
        events,
        ..RawInput::default()
    }
}

/// What the plugin hooks of one context saw around a nested immediate viewport.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct HookFacts {
    /// Input hooks that ran for the child viewport's raw input.
    pub child_input_hooks: usize,
    /// Every one of them ran while the context's current viewport was still the parent.
    pub input_hooks_see_parent: bool,
    /// Begin-pass hooks that ran with the child as the current viewport.
    pub child_begin_hooks: usize,
}

enum Entry {
    Input { raw: ViewportId, current: ViewportId },
    Begin { current: ViewportId },
}

struct Recorder(Arc<Mutex<Vec<Entry>>>);

impl Plugin for Recorder {
    fn debug_name(&self) -> &'static str {
        "craft_ui.surface.recorder"
    }

    fn input_hook(&mut self, ctx: &Context, input: &mut RawInput) {
        if let Ok(mut log) = self.0.lock() {
            log.push(Entry::Input { raw: input.viewport_id, current: ctx.viewport_id() });
        }
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        if let Ok(mut log) = self.0.lock() {
            log.push(Entry::Begin { current: ui.ctx().viewport_id() });
        }
    }
}

/// A renderer like eframe's: the child's pass runs nested inside the parent's `Ui` closure.
fn render_child(ctx: &Context, viewport: egui::ImmediateViewport<'_>) {
    let mut show = viewport.viewport_ui_cb;
    let mut input = raw(0.0, Vec::new());
    input.viewport_id = viewport.ids.this;
    input.viewports.insert(viewport.ids.this, egui::ViewportInfo { parent: Some(viewport.ids.parent), ..Default::default() });
    let mut output = ctx.run_ui(input, |ui| show(ui));
    output.textures_delta.clear();
}

/// Two frames of a root viewport that shows an immediate child viewport, with a recording plugin on the context.
pub fn child_viewport_hooks() -> HookFacts {
    let ctx = Context::default();
    ctx.set_embed_viewports(false);
    let log = Arc::new(Mutex::new(Vec::new()));
    ctx.add_plugin(Recorder(log.clone()));
    Context::set_immediate_viewport_renderer(render_child);
    let child = ViewportId::from_hash_of("craft_ui.surface.child");
    for frame in 0..2 {
        let mut output = ctx.run_ui(raw(f64::from(frame), Vec::new()), |ui| {
            ui.ctx().show_viewport_immediate(child, ViewportBuilder::default(), |_, _| {});
        });
        output.textures_delta.clear();
    }
    let log = log
        .lock()
        .map(|log| {
            log.iter()
                .map(|e| match e {
                    Entry::Input { raw, current } => (Some((*raw, *current)), None),
                    Entry::Begin { current } => (None, Some(*current)),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let inputs: Vec<(ViewportId, ViewportId)> = log.iter().filter_map(|(input, _)| *input).filter(|(raw, _)| *raw == child).collect();
    HookFacts {
        child_input_hooks: inputs.len(),
        input_hooks_see_parent: inputs.iter().all(|(_, current)| *current == ViewportId::ROOT),
        child_begin_hooks: log.iter().filter_map(|(_, begin)| *begin).filter(|current| *current == child).count(),
    }
}

/// The widget `ViewportState::hits` names for a press, by the order of the frame's pointer events.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PressHits {
    /// Events: pointer moved to A, then press at B. `hits.click` is the widget at B.
    pub press_last_is_press_target: bool,
    /// Events: press at B, then pointer moved to A. `hits.click` is the widget at A, where the pointer ended.
    pub move_last_is_move_target: bool,
}

/// Three frames over two click widgets: one to register them, then the two event orders.
pub fn press_hits() -> PressHits {
    let ctx = fixture::bare_context();
    let (a, b) = (super::memory::unique("craft_ui.surface.a"), super::memory::unique("craft_ui.surface.b"));
    let rect_a = widget_at(10.0, 10.0);
    let rect_b = widget_at(100.0, 10.0);
    let mut show = |ui: &mut Ui| {
        ui.interact(rect_a, a, Sense::click());
        ui.interact(rect_b, b, Sense::click());
    };
    let mut run = |time: f64, events: Vec<Event>| {
        let mut output = ctx.run_ui(raw(time, events), &mut show);
        output.textures_delta.clear();
        ctx.viewport(|viewport| viewport.hits.click.map(|widget| widget.id))
    };
    run(1.0, Vec::new());
    let first = run(2.0, vec![Event::PointerMoved(rect_a.center()), fixture::primary_button(rect_b.center(), true)]);
    let second = run(3.0, vec![fixture::primary_button(rect_b.center(), true), Event::PointerMoved(rect_a.center())]);
    PressHits { press_last_is_press_target: first == Some(b), move_last_is_move_target: second == Some(a) }
}

/// Whether a member that registered no egui widget, given `register_accesskit_parent`, has its node among the
/// children of its parent's node.
pub fn registered_parent_owns_member_node() -> bool {
    let ctx = fixture::bare_context();
    let (parent, member) = (super::memory::unique("craft_ui.surface.parent"), super::memory::unique("craft_ui.surface.member"));
    let rect = widget_at(10.0, 10.0);
    let mut output = ctx.run_ui(raw(1.0, Vec::new()), |ui| {
        let ctx = ui.ctx().clone();
        ui.interact(rect, parent, Sense::click());
        ctx.accesskit_node_builder(parent, |_| ());
        ctx.register_accesskit_parent(member, parent);
        ctx.accesskit_node_builder(member, |_| ());
    });
    output.textures_delta.clear();
    let Some(update) = output.platform_output.accesskit_update else { return false };
    update.nodes.iter().find(|(id, _)| *id == parent.accesskit_id()).is_some_and(|(_, node)| node.children().contains(&member.accesskit_id()))
}

/// Whether a widget that took focus with `request_focus_with_filter` still holds it after a bare Escape: `keep`
/// says whether the filter lists Escape.
pub fn focus_survives_escape(keep: bool) -> bool {
    let ctx = fixture::bare_context();
    let id = super::memory::unique("craft_ui.surface.escape");
    let rect = widget_at(10.0, 10.0);
    let run = |time: f64, events: Vec<Event>, focus: bool| {
        let mut output = ctx.run_ui(raw(time, events), |ui| {
            ui.interact(rect, id, Sense::focusable_noninteractive());
            if focus {
                let filter = egui::EventFilter { escape: keep, ..egui::EventFilter::default() };
                ui.memory_mut(|memory| memory.request_focus_with_filter(id, filter));
            }
        });
        output.textures_delta.clear();
    };
    run(1.0, Vec::new(), true);
    run(2.0, Vec::new(), false);
    run(3.0, vec![fixture::key_press(egui::Key::Escape, egui::Modifiers::NONE)], false);
    run(4.0, Vec::new(), false);
    ctx.memory(|memory| memory.has_focus(id))
}

/// Writes the direction into the context's styles before any pass, as an egui backend that derives it from the
/// system locale would.
pub fn preset_style_direction(ctx: &Context, right_to_left: bool) {
    ctx.all_styles_mut(|style| style.right_to_left = right_to_left);
}

/// What a pass saw of one frame that discarded its first pass.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PassFacts {
    /// Input events the pass received.
    pub events: usize,
    /// The cumulative frame number during the pass.
    pub frame: u64,
    /// The cumulative pass number during the pass.
    pub pass: u64,
}

/// One frame with a key event whose first pass asks for a discard.
pub fn discard_passes() -> Vec<PassFacts> {
    let ctx = fixture::bare_context();
    let mut seen = Vec::new();
    let event = fixture::key_press(egui::Key::A, egui::Modifiers::NONE);
    let mut output = ctx.run_ui(raw(1.0, vec![event]), |ui| {
        seen.push(PassFacts {
            events: ui.input(|input| input.events.len()),
            frame: ui.ctx().cumulative_frame_nr(),
            pass: ui.ctx().cumulative_pass_nr(),
        });
        if seen.len() == 1 {
            ui.ctx().request_discard("surface test");
        }
    });
    output.textures_delta.clear();
    seen
}
