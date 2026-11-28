use super::Inspector;

impl Pattern for Inspector {}

register_contract!(Inspector; pattern);

impl Inspector {
    pub fn show(self, ui: &mut Ui) -> Response {
        ui.response()
    }
}
