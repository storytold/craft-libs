use super::{Menu, Select};

impl Component for Select {
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {}
}

impl Component for Menu {
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {}
}
