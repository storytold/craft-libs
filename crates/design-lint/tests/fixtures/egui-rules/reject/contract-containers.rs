fn f(ui: &mut Ui) {
    egui::CentralPanel::default().show(ctx, |ui| {});
    egui::Window::new("x");
    area.show_rows(ui, 20.0, 100, |ui, range| {});
}
