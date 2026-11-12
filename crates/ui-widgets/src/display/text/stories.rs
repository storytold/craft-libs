//! Stories of `Text`.

use super::{Elide, Shade, Text};
use crate::contract::{Props as _, Show as _, ShowContainer as _};
use crate::layout_components::{Grid, Row};
use crate::style::TypeRole;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::ColumnWidth;
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
    let elide = *data.choice("elide", Elide::End, &[("End", Elide::End), ("Middle", Elide::Middle), ("Start", Elide::Start), ("Path", Elide::Path)]);
    Text::new(text).role(role).elide(elide).show(ui).into()
}

#[story(Text, covers = [], drive = Drive::Never)]
fn text_toned(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Text::new("Enter a value between 0 and 100").role(TypeRole::Caption).tone(Tone::Bad).show(ui).into()
}

/// One line each, cut at the end, the middle and the start, a path that drops leading folders, and a right-to-left
/// name that keeps its logical start.
#[story(Text, covers = [], drive = Drive::Never)]
fn text_elide_places(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    // A room narrower than every text, so each one shows its cut.
    const COLUMNS: &[ColumnWidth] = &[ColumnWidth::Content, ColumnWidth::Fixed(160.0)];
    let name = "IMG_2024_summer_holiday_final_edit.psd";
    let rows = [
        ("End", name, Elide::End),
        ("Middle", name, Elide::Middle),
        ("Start", name, Elide::Start),
        ("Path", "/home/ada/Pictures/2026/summer holiday/IMG_2024.psd", Elide::Path),
        ("Right to left", "שם קובץ ארוך מאוד שלא נכנס בשורה אחת של הפאנל.psd", Elide::End),
    ];
    Grid::new(COLUMNS)
        .show(ui, |ui, _| {
            for (place, text, elide) in rows {
                Text::new(place).shade(Shade::Muted).id_salt(("place", place)).show(ui);
                Text::new(text).elide(elide).id_salt(("text", place)).show(ui);
            }
        })
        .into()
}

/// Every content colour: the shades, the tones and the disabled colour. Text on a fill reads the host's ink.
#[story(Text, covers = [], drive = Drive::Never)]
fn text_contents(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let shades = [("Normal", Shade::Normal), ("Muted", Shade::Muted), ("Faint", Shade::Faint)];
    let tones = [("Accent", Tone::Accent), ("Info", Tone::Info), ("Good", Tone::Good), ("Warn", Tone::Warn), ("Bad", Tone::Bad)];
    Row::new()
        .wrap(true)
        .show(ui, |ui, _| {
            for (name, shade) in shades {
                Text::new(name).role(TypeRole::Label).shade(shade).id_salt(name).show(ui);
            }
            for (name, tone) in tones {
                Text::new(name).role(TypeRole::Label).tone(tone).id_salt(name).show(ui);
            }
            Text::new("Disabled").role(TypeRole::Label).enabled(false).id_salt("disabled").show(ui);
        })
        .into()
}
