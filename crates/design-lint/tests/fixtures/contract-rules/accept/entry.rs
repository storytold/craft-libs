#[derive(Component)]
pub struct Chip {
    #[common]
    common: Common,
}

impl Chip {
    pub fn count(&self) -> u32 {
        0
    }
}

impl Component for Chip {
    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {}
}

#[derive(Pattern)]
pub struct Inspector;

impl Inspector {
    pub fn show(self, ui: &mut Ui) -> Response {
        ui.response()
    }
}

struct Hidden;

impl Hidden {
    pub fn show(self, ui: &mut Ui) -> Response {
        ui.response()
    }
}

pub fn header(ui: &mut Ui) {}

pub(crate) fn inner(ui: &mut Ui) -> Response {
    ui.response()
}

pub fn measure(ui: &Ui) -> Response {
    todo!()
}

#[derive(Component)]
#[component(overlay)]
pub struct Select {
    #[common]
    common: Common,
}

impl Component for Select {
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {}
}

#[derive(Component)]
#[component(container, overlay)]
pub struct Menu {
    #[common]
    common: Common,
}

impl Component for Menu {
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {}
}

impl Widget for Chip {}

impl SomeTrait for Chip {}

pub struct Plain;

impl Component for Plain {}

register_contract!(Plain);
