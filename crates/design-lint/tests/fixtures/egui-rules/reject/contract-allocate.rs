fn f(ui: &mut Ui, rect: Rect, id: Id) {
    ui.allocate_exact_size(size, sense);
    ui.interact(rect, id, sense);
    ui.put(rect, widget);
    ui.add_sized(size, widget);
    ui.new_child(UiBuilder::new().max_rect(rect));
}
