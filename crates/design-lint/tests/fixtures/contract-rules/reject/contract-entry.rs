pub struct Swatch {
    color: Color,
}

impl Swatch {
    pub fn show(self, ui: &mut Ui) -> Response {
        ui.response()
    }
}

pub fn colour_row(ui: &mut Ui, colors: &[Color]) -> EditResponse<Color> {
    todo!()
}

impl Swatch {
    pub fn row(ui: &mut Ui, colors: &[Color]) -> Response {
        ui.response()
    }

    pub fn draw(&mut self, ui: &mut Ui) -> Response {
        ui.response()
    }

    fn private(ui: &mut Ui) -> Response {
        ui.response()
    }
}
