use super::{BarItem, Button, Swatch};

impl Button {
    pub fn show(self, ui: &mut Ui) -> Response {
        ui.response()
    }

    pub fn count(mut self, n: u32) -> Self {
        self.count = n;
        self
    }
}

impl Swatch {
    pub fn show(self, ui: &mut Ui) -> Response {
        ui.response()
    }
}

impl BarItem {
    pub fn label(mut self, text: String) -> Self {
        self.text = text;
        self
    }
}
