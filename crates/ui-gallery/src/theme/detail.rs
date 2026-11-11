//! The detail of the picked token or size: where its value comes from, the contrast pairs it takes part in and the
//! ramp it steps through.

use super::browser::value_of;
use super::chip::chip;
use super::subject::Pick;
use super::words::{layer_words, lengths_words, pin_words, ramp_name, value_words};
use craft_behavior::keyed;
use craft_theme::{ColorOrigin, ComponentField, Explained, Rgba, SizeOrigin, TokenRef};
use craft_ui::contract::{Gap, Pattern, Props, Scroll, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::Stack;
use craft_ui::list::{List, ListRow, ThumbSize};
use craft_ui::sections::Section;
use craft_ui::style::TypeRole;
use egui::{Response, Ui};

/// One line of a facts list: a name, a value, an optional colour chip and an optional note under the name.
struct Fact {
    id: usize,
    name: String,
    value: String,
    color: Option<Rgba>,
    note: Option<String>,
}

/// The lines of one facts list.
#[derive(Default)]
struct Facts(Vec<Fact>);

impl Facts {
    fn push(&mut self, name: impl Into<String>, value: impl Into<String>) -> &mut Fact {
        let id = self.0.len();
        self.0.push(Fact { id, name: name.into(), value: value.into(), color: None, note: None });
        &mut self.0[id]
    }

    /// Shows the lines as a read-only list named `label`.
    fn show(&self, ui: &mut Ui, label: &'static str) {
        let source = keyed(&self.0, |fact| fact.id);
        List::<usize>::new(label).select(SelectMode::None).sizing(Sizing::Content).show_items(ui, &source, &mut |key, _| {
            let Some(fact) = self.0.get(*key) else { return ListRow::spacer() };
            let mut row = ListRow::new(fact.name.clone()).trailing(fact.value.clone());
            if let Some(color) = fact.color {
                row = row.thumb(ThumbSize::Small, chip(color));
            }
            match &fact.note {
                Some(note) => row.detail(note.clone()),
                None => row,
            }
        });
    }
}

/// The facts about the picked row of an explanation: for a colour its value, source and pin, the pairs it takes part
/// in and its ramp; for a component token or a size its values and where they are set.
#[derive(Pattern, Props)]
pub struct TokenDetail<'a> {
    explained: &'a Explained,
    /// The row to describe; without one the detail says to select a row.
    #[prop]
    pick: Option<&'a Pick>,
}

impl<'a> TokenDetail<'a> {
    /// A detail over `explained`.
    pub fn new(explained: &'a Explained) -> Self {
        TokenDetail { explained, pick: None }
    }

    /// Shows the detail across the room offered, scrolling.
    pub fn show(self, ui: &mut Ui) -> Response {
        let TokenDetail { explained, pick } = self;
        Stack::new()
            .fill()
            .scroll(Scroll::vertical())
            .gap(Gap::Section)
            .show(ui, |ui, _| match pick {
                None => {
                    Text::new("Select a row to see its facts, contrast pairs and ramp.").shade(Shade::Muted).show(ui);
                }
                Some(Pick::Size(path)) => match explained.sizes.iter().find(|origin| origin.path == *path) {
                    Some(origin) => size(ui, origin),
                    None => {
                        Text::new("This size is not in the explanation.").shade(Shade::Muted).show(ui);
                    }
                },
                Some(Pick::Token(TokenRef::Component(field))) => component(ui, explained, *field),
                Some(Pick::Token(TokenRef::Color(token))) => match explained.color(*token) {
                    Some(origin) => color(ui, explained, TokenRef::Color(*token), origin),
                    None => {
                        Text::new("This colour is not in the explanation.").shade(Shade::Muted).show(ui);
                    }
                },
            })
            .output
    }
}

fn heading(ui: &mut Ui, text: String) {
    Text::new(text).role(TypeRole::Heading).show(ui);
}

fn size(ui: &mut Ui, origin: &SizeOrigin) {
    heading(ui, origin.path.clone());
    let mut facts = Facts::default();
    facts.push("Value", value_words(origin.value));
    facts.push("Set by", layer_words(origin.layer));
    facts.push("Range", format!("{} to {}", origin.range.0, origin.range.1));
    facts.show(ui, "Size facts");
}

fn component(ui: &mut Ui, explained: &Explained, field: ComponentField) {
    heading(ui, field.path());
    let mut facts = Facts::default();
    facts.push("Standard / small", lengths_words(&explained.resolved.tokens.component, field));
    facts.push("Set in", "the theme file's components map");
    facts.show(ui, "Component token facts");
}

fn color(ui: &mut Ui, explained: &Explained, token: TokenRef, origin: &ColorOrigin) {
    heading(ui, token.path());
    Section::fixed("Value").show(ui, |ui, _| {
        let mut facts = Facts::default();
        let value = facts.push("Value", origin.value.to_hex());
        value.color = Some(origin.value);
        facts.push("Source", origin.source_words());
        let derived = facts.push("Derived", origin.derived.to_hex());
        derived.color = Some(origin.derived);
        facts.push("Pin", origin.pin.map_or("none", pin_words));
        if let Some(before) = origin.contrast_move {
            let moved = facts.push("Before the contrast pass", before.to_hex());
            moved.color = Some(before);
        }
        facts.show(ui, "Colour facts");
    });
    Section::fixed("Contrast pairs").show(ui, |ui, _| pairs(ui, explained, token));
    if let Some((shared, step, toward)) = explained.ramps.steps_of(origin.source) {
        // An anchored token's steps are the ramp moved to its anchor.
        let (title, steps) = match &origin.anchoring {
            Some(anchor) => (format!("Ramp (neutral, anchored on {})", anchor.on.path()), anchor.steps.0),
            None => (format!("Ramp ({})", ramp_name(origin.source)), shared.0),
        };
        Section::fixed(title).show(ui, |ui, _| ramp(ui, &steps, step, toward));
    }
}

/// Every pair `token` takes part in: the other colour's chip and path, the ratio against the minimum and the result in
/// words.
fn pairs(ui: &mut Ui, explained: &Explained, token: TokenRef) {
    let mut facts = Facts::default();
    for report in explained.pairs_of(token) {
        let pair = &report.pair;
        let (other, role) = if pair.fg == token { (pair.bg, "on") } else { (pair.fg, "under") };
        let result = if pair.passes() { "pass" } else { "FAIL" };
        let line = facts.push(format!("{role} {}", other.path()), format!("{:.2} of {:.1} {result}", pair.ratio, pair.minimum));
        line.color = Some(value_of(explained, other));
        if report.fixed_by_pass {
            line.note = Some(format!("fixed by pass (was {:.2})", report.ratio_before));
        } else if report.moved_by_pass {
            line.note = Some(format!("moved (was {:.2})", report.ratio_before));
        }
    }
    if facts.0.is_empty() {
        Text::new("No contrast rule measures this colour.").shade(Shade::Muted).show(ui);
    } else {
        facts.show(ui, "Contrast pairs");
    }
}

/// The twelve steps of a ramp, the used one and the one a mix moves toward marked.
fn ramp(ui: &mut Ui, steps: &[Rgba; 12], used: usize, toward: Option<usize>) {
    let mut facts = Facts::default();
    for (at, color) in steps.iter().enumerate() {
        let number = at + 1;
        let mark = if number == used {
            " (used)"
        } else if toward == Some(number) {
            " (moves toward)"
        } else {
            ""
        };
        let line = facts.push(format!("step {number}{mark}"), color.to_hex());
        line.color = Some(*color);
    }
    facts.show(ui, "Ramp");
}
