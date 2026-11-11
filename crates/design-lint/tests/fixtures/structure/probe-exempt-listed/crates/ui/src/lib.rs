#[derive(Component)]
#[component(exempt(case = "stories", reason = "shown by its host"))]
pub struct Face;

register_contract!(Manual; overlay, exempt(case = "sizes", reason = "hand-written"));
