fn paint(response: &Response, tokens: &Palette) -> Color32 {
    if response.hovered() {
        tokens.hover_color
    } else {
        tokens.rest_color
    }
}

fn timed(response: &Response, hover_color: Color32, rest_color: Color32) -> Color32 {
    response.hovered().then_some(hover_color).unwrap_or(rest_color)
}

fn pair(response: &Response) -> Color32 {
    match (response.hovered(), response.is_pointer_button_down_on()) {
        (true, _) => hover_color,
        _ => rest_color,
    }
}

fn egui_pick(ui: &Ui, response: &Response) -> Color32 {
    ui.style().interact(response).bg_fill
}
