#[derive(Props)]
pub struct Bar {
    /// Width in points.
    #[prop]
    width: f32,
    /// Size in points.
    #[prop]
    size_hint: Option<Vec2>,
    /// A newtype over points.
    #[prop]
    stem: Px,
    /// An integer named like a size.
    #[prop]
    side_px: u16,
}

impl Bar {
    pub fn thumb_sized(mut self, side: Vec2) -> Self {
        self.width = side.x;
        self
    }
}

#[non_exhaustive]
pub enum Thumb {
    Fit,
    Px(u16),
}
