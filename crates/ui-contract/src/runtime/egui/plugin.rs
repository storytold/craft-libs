//! The plugin impl, its registration and hook order, and the plugin lock.

use crate::runtime::frame;
use egui::{Context, FullOutput, Plugin, RawInput, Ui};

/// The runtime's plugin. egui holds a plugin's mutex while it calls a hook, so the struct holds no state:
/// every record lives in context data behind its own lock.
struct RuntimePlugin;

impl Plugin for RuntimePlugin {
    fn debug_name(&self) -> &'static str {
        "craft_ui.runtime"
    }

    fn input_hook(&mut self, ctx: &Context, input: &mut RawInput) {
        crate::runtime::egui::input_filter::input_hook(ctx, input);
        frame::input_hook(ctx, input);
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        frame::begin_pass(ui);
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        crate::runtime::egui::access::note_pass_end(ui.ctx());
        frame::end_pass(ui);
    }

    fn output_hook(&mut self, ctx: &Context, output: &mut FullOutput) {
        // Dangling `controls` go first, so the settled tree the testing module reads is the one the platform gets.
        crate::runtime::egui::access::drop_dangling_relations(output);
        crate::runtime::egui::access::settle_output(ctx, output);
    }
}

/// Whether the runtime plugin is on `ctx`.
pub(crate) fn is_registered(ctx: &Context) -> bool {
    ctx.plugin_opt::<RuntimePlugin>().is_some()
}

/// Adds the runtime plugin to `ctx`.
pub(crate) fn register(ctx: &Context) {
    ctx.add_plugin(RuntimePlugin);
}
