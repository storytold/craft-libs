#[derive(Props)]
pub struct Chip {
    count: u32,
    note: Option<String>,
    tone: Tone,
    key: Key,
}

impl Chip {
    pub fn count(mut self, n: u32) -> Self {
        self.count = n;
        self
    }

    pub fn note(mut self, text: impl Into<String>) -> Self {
        self.note = Some(text.into());
        self
    }

    pub fn tone(self, tone: Tone) -> Self {
        Self { tone, ..self }
    }
}
