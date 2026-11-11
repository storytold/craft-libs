mod builder;
mod component;
mod pattern;

#[derive(Component)]
pub struct Button {
    #[common]
    common: Common,
    count: u32,
}

#[derive(Component)]
pub struct Select {
    #[common]
    common: Common,
}

#[derive(Component)]
#[component(overlay)]
pub struct Menu {
    #[common]
    common: Common,
}

pub struct Swatch {
    color: Color,
}

pub struct Inspector;

pub struct BarItem {
    text: String,
}
