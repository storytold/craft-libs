use egui::Memory;

fn f(ctx: &egui::Context) {
    ctx.memory_mut(|m| m.reset_areas());
    ctx.options(|o| o.clone());
}
