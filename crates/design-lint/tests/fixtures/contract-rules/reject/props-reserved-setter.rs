#[derive(Props)]
pub struct Slider {
    common: Common,
    step: f32,
}

impl Slider {
    pub fn enabled(mut self, on: bool) -> Self {
        self.common.enabled = if on { Some(true) } else { None };
        self
    }

    pub fn label(self, text: &str) -> Slider {
        self.with_label(text)
    }
}
