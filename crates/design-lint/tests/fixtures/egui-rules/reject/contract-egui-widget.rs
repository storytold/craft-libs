use egui::Slider;

fn f(ui: &mut Ui, gap: f32) {
    ui.button("x");
    ui.checkbox(&mut on, "x");
    ui.add_space(gap);
    ui.add(Slider::new(&mut v, 0.0..=1.0));
    ui.add(egui::Button::new("x"));
}
