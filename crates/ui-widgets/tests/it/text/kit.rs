//! A text field driven frame by frame.

use craft_behavior::PartKey;
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::Frames;
use craft_ui::testing::pointer::click;
use craft_ui::testing::text::widget_id;
use craft_ui::text::TextField;
use egui::{Event, Id, Rect};

pub const TEXT: PartKey = PartKey::new("text");

/// What the last frame's field reported.
#[derive(Clone)]
#[non_exhaustive]
pub struct Seen {
    pub rect: Rect,
    pub id: Id,
    pub events: Vec<String>,
    pub submitted: bool,
    pub escaped: bool,
}

impl Default for Seen {
    fn default() -> Self {
        Seen { rect: Rect::NOTHING, id: Id::NULL, events: Vec::new(), submitted: false, escaped: false }
    }
}

#[non_exhaustive]
pub struct Rig {
    pub frames: Frames,
    pub text: String,
    pub seen: Seen,
}

impl Rig {
    pub fn new(text: &str) -> Rig {
        let mut rig = Rig { frames: Frames::new(), text: text.to_owned(), seen: Seen::default() };
        rig.idle(3);
        rig
    }

    fn show(frames: &mut Frames, events: Vec<Event>, text: &mut String, seen: &mut Seen, build: &dyn Fn(&mut String) -> Bind<'_, String>) {
        let _ = build;
        frames.frame(events, |ui| {
            let out = TextField::new("Name", Bind::bound(&mut *text)).id_salt("name").show(ui);
            seen.rect = out.response.rect;
            seen.id = out.response.id;
            seen.submitted = out.submitted;
            seen.escaped = out.escaped;
            for event in &out.events {
                seen.events.push(format!("{event:?}").split_whitespace().next().unwrap_or_default().to_owned());
            }
        });
    }

    pub fn frame(&mut self, events: Vec<Event>) {
        Self::show(&mut self.frames, events, &mut self.text, &mut self.seen, &|_| Bind::kept());
    }

    pub fn idle(&mut self, n: usize) {
        for _ in 0..n {
            self.frame(Vec::new());
        }
    }

    pub fn focus(&mut self) {
        let at = self.seen.rect.center();
        let (text, seen) = (&mut self.text, &mut self.seen);
        click(&mut self.frames, at, &mut |ui| {
            let out = TextField::new("Name", Bind::bound(&mut *text)).id_salt("name").show(ui);
            seen.rect = out.response.rect;
            seen.id = out.response.id;
        });
    }

    pub fn widget(&self) -> Id {
        widget_id(self.seen.id, TEXT)
    }
}
