fn f(ui: &mut Ui, response: Response) {
    egui::Area::new(id).show(ctx, |ui| {});
    ui.menu_button("x", |ui| {});
    response.on_hover_text("x");
}
