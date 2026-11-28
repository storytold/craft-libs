pub enum Mode {
    A,
}

pub enum Event {
    Moved { x: f32 },
}

pub struct Settings {
    pub width: f32,
}

pub struct Handle(pub u32);
