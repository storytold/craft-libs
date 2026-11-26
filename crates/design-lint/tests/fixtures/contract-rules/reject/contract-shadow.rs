#[derive(Component)]
pub struct Chip {
    #[common]
    common: Common,
}

impl Chip {
    pub fn show(self, ui: &mut Ui) -> Response {
        ui.response()
    }
}
