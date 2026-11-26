#[derive(Component, Props)]
pub struct Chip {
    #[common]
    common: Common,
    /// The count shown.
    #[prop]
    count: u32,
    /// A label; the setter has its own name.
    #[prop(name = "caption")]
    text: String,
    /// A width from content.
    #[prop]
    width: ColumnWidth,
    gap: f32,
    /// A fraction of full strength, not a length.
    #[prop(quantity = "opacity")]
    opacity: f32,
    /// How many rows show; an integer with no size word in its name.
    #[prop]
    rows: u16,
}

pub struct Plain {
    enabled: bool,
    count: u32,
}

impl Plain {
    pub fn count(mut self, n: u32) -> Self {
        self.count = n;
        self
    }
}

impl Chip {
    pub fn enabled(&self) -> bool {
        true
    }

    pub fn count_doubled(mut self, n: u32) -> Self {
        self.count = n * 2;
        self
    }

    pub fn both(mut self, n: u32, text: String) -> Self {
        self.count = n;
        self.text = text;
        self
    }

    pub fn reset(mut self) -> Self {
        self.count = 0;
        self
    }
}

impl Props for Chip {
    fn enabled(self, on: bool) -> Self {
        self
    }
}

impl Chip {
    pub fn key_tone(mut self, tone: Tone) -> Self {
        self.key.tone = tone;
        self
    }

    pub fn both_fields(self, tone: Tone) -> Self {
        Self { count: 1, tone, ..self }
    }
}
