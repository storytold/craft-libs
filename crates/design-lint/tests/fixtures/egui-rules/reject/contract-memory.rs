fn f(ctx: &Context, ui: &mut Ui) {
    ctx.data_mut(|d| d.insert_temp(id, 1));
    let _ = ui.make_persistent_id("a");
    let _ = egui::Id::unique();
}
