use egui::Widget;

impl Widget for Chip {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        ui.response()
    }
}

impl egui::Widget for Badge {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        ui.response()
    }
}

use egui::Widget as EguiWidget;

impl EguiWidget for Tag {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        ui.response()
    }
}

impl egui::Widget for &mut Chip {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        ui.response()
    }
}
