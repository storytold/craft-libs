#[derive(Component)]
pub struct Chip {
    #[common]
    common: Common,
}

impl Chip {
    pub fn with_style(self, style: ChipStyle) -> Self {
        self.apply(style)
    }

    pub fn custom(self, look: SurfaceLook) -> Self {
        self.apply(look)
    }

    pub fn fill(self, color: Color32) -> Self {
        self.apply(color)
    }

    pub fn text_color(self, color: impl Into<Color>) -> Self {
        self.apply(color)
    }
}
