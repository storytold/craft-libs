//! Sizing passes and `request_discard`.

use egui::{Context, Layout, Rect, Ui, UiBuilder, Vec2};

/// The reason of the rerun that delivers an update a given binding held in the pass before. That rerun writes the update to
/// the host, so the purity check of the suite's fixture lets it change state.
pub(crate) const HELD_UPDATE_RERUN: &str = "craft_ui: an update is held for the next pass";

/// Why a pass asks egui to discard it and run again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rerun {
    /// Layout memory: a size the pass placed by changed.
    Layout,
    /// An edit changed a value the pass showed.
    Edit,
    /// An update a binding held is delivered in the next pass.
    HeldUpdate,
    /// The input filter holds input the pass must see.
    Input(&'static str),
    /// An app or host's own reason.
    App(&'static str),
}

impl Rerun {
    fn reason(self) -> &'static str {
        match self {
            Rerun::Layout => "craft_ui: layout memory changed",
            Rerun::Edit => "craft_ui: an edit changed the value",
            Rerun::HeldUpdate => HELD_UPDATE_RERUN,
            Rerun::Input(reason) | Rerun::App(reason) => reason,
        }
    }
}

/// The pass that asked for a discard last, and every reason it asked for.
#[derive(Clone)]
struct Asked {
    pass: u64,
    reasons: Vec<Rerun>,
}

fn asked_id(ctx: &Context) -> egui::Id {
    egui::Id::NULL.with(("craft_ui::discard_asked", super::pass::current_viewport(ctx)))
}

fn running_pass(ctx: &Context) -> u64 {
    super::pass::pass_nr(ctx, super::pass::current_viewport(ctx))
}

/// Asks egui to discard the running pass and run it again.
pub(crate) fn request_discard(ctx: &Context, rerun: Rerun) {
    let (id, pass) = (asked_id(ctx), running_pass(ctx));
    super::memory::locked(ctx, |data| {
        let asked = data.get_temp_mut_or(id, Asked { pass, reasons: Vec::new() });
        if asked.pass != pass {
            *asked = Asked { pass, reasons: Vec::new() };
        }
        if !asked.reasons.contains(&rerun) {
            asked.reasons.push(rerun);
        }
    });
    ctx.request_discard(rerun.reason());
}

/// Why the running pass runs again: `None` in a frame's first pass, else the reasons the discarded pass asked for
/// (empty when egui discarded it on its own).
pub(crate) fn rerun_reasons(ctx: &Context) -> Option<Vec<Rerun>> {
    if ctx.current_pass_index() == 0 {
        return None;
    }
    let (id, pass) = (asked_id(ctx), running_pass(ctx));
    let asked = super::memory::peek_clone::<Asked>(ctx, id);
    Some(asked.filter(|asked| asked.pass + 1 == pass).map(|asked| asked.reasons).unwrap_or_default())
}

/// Whether a discard requested in the running pass would be honoured: egui runs at most `Options::max_passes` passes a
/// frame.
pub(crate) fn discardable(ctx: &Context) -> bool {
    ctx.current_pass_index() + 1 < ctx.options(|options| options.max_passes.get())
}

/// Runs `f` in an invisible sizing pass `width` wide, from the parent's cursor, and returns what it took.
/// The parent allocates nothing.
pub(crate) fn run_sized<R>(ui: &mut Ui, width: f32, layout: Layout, f: impl FnOnce(&mut Ui) -> R) -> (R, Vec2) {
    let room = Rect::from_min_size(ui.cursor().min, Vec2::new(width, f32::INFINITY));
    let mut child = ui.new_child(UiBuilder::new().max_rect(room).layout(layout).sizing_pass().invisible());
    let inner = f(&mut child);
    (inner, child.min_rect().size())
}
