#[derive(Component)]
pub struct Select {
    #[common]
    common: Common,
}

impl Component for Select {
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {}
}

#[derive(Component)]
#[component(exempt(case = overlay, reason = "none"))]
pub struct Combo {
    #[common]
    common: Common,
}

impl Component for Combo {
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {}
}
