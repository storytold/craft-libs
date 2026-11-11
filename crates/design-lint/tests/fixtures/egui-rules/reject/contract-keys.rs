fn f(ui: &mut Ui, event: &egui::Event) {
    ui.input(|i| i.key_pressed(Key::A));
    if let egui::Event::Key { .. } = event {}
}
