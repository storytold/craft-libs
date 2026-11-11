//! Stories of `Text`.

use super::{Elide, Shade, Text};
use crate::contract::Show as _;
use crate::style::TypeRole;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_theme::variant::Tone;
use egui::Ui;

#[story(Text, covers = [Default], drive = Drive::Never)]
fn text(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Text::new("Layers").show(ui).into()
}

#[story(Text, covers = [], drive = Drive::Never)]
fn text_role(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let role = *data.choice(
        "role",
        TypeRole::Caption,
        &[
            ("Caption", TypeRole::Caption),
            ("Label", TypeRole::Label),
            ("Body", TypeRole::Body),
            ("BodyStrong", TypeRole::BodyStrong),
            ("Title", TypeRole::Title),
            ("Heading", TypeRole::Heading),
            ("BodyLarge", TypeRole::BodyLarge),
            ("Mono", TypeRole::Mono),
        ],
    );
    let shade = *data.choice("shade", Shade::Faint, &[("Normal", Shade::Normal), ("Muted", Shade::Muted), ("Faint", Shade::Faint)]);
    Text::new("Blend mode").role(role).shade(shade).show(ui).into()
}

#[story(Text, covers = [], drive = Drive::Never)]
fn text_elided(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let text = data.value("text", String::from("A layer name much longer than the room it is shown in")).clone();
    let role = *data.choice(
        "role",
        TypeRole::Body,
        &[
            ("Caption", TypeRole::Caption),
            ("Label", TypeRole::Label),
            ("Body", TypeRole::Body),
            ("BodyStrong", TypeRole::BodyStrong),
            ("Title", TypeRole::Title),
            ("Heading", TypeRole::Heading),
            ("BodyLarge", TypeRole::BodyLarge),
            ("Mono", TypeRole::Mono),
        ],
    );
    let elide = *data.choice("elide", Elide::End, &[("End", Elide::End), ("Path", Elide::Path)]);
    Text::new(text).role(role).elide(elide).show(ui).into()
}

#[story(Text, covers = [], drive = Drive::Never)]
fn text_toned(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Text::new("Enter a value between 0 and 100").role(TypeRole::Caption).tone(Tone::Bad).show(ui).into()
}
