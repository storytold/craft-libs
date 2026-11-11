//! Stories of `Watermark`.

use super::{Watermark, WatermarkSettings, WatermarkStyle};
use crate::contract::Bind;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Text,
    Graphic,
}

const KINDS: [(&str, Kind); 2] = [("Text", Kind::Text), ("Graphic", Kind::Graphic)];

fn kind_of(style: &WatermarkStyle) -> Kind {
    if style.is_graphic() { Kind::Graphic } else { Kind::Text }
}

/// The editor over the settings kept under `name`, with the `style` and `vertical` values in step with them: a value
/// that differs from the settings is applied, and a change made in the editor is written back to the value.
fn watermark_story(ui: &mut Ui, data: &mut StoryData, name: &'static str, start: WatermarkSettings) -> StoryOutput {
    let kind = *data.choice("style", kind_of(&start.style), &KINDS);
    let vertical = *data.value("vertical", start.vertical);
    let settings = data.value(name, start);
    if kind_of(&settings.style) != kind {
        settings.style = match kind {
            Kind::Text => WatermarkStyle::Text("© Studio".to_owned()),
            Kind::Graphic => WatermarkStyle::Graphic("logo.png".to_owned()),
        };
    }
    settings.vertical = vertical;
    let output: StoryOutput = Watermark::new("Watermark", Bind::bound(&mut *settings)).show(ui).into();
    let (kind, vertical) = (kind_of(&settings.style), settings.vertical);
    *data.choice("style", kind, &KINDS) = kind;
    *data.value("vertical", vertical) = vertical;
    output
}

#[story(Watermark<'static>, covers = [], drive = Drive::Never)]
fn watermark(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = WatermarkSettings { style: WatermarkStyle::Text("© Studio".to_owned()), ..WatermarkSettings::default() };
    watermark_story(ui, data, "watermark", start)
}

#[story(Watermark<'static>, covers = [], drive = Drive::Never)]
fn watermark_graphic(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = WatermarkSettings { style: WatermarkStyle::Graphic("logo.png".to_owned()), ..WatermarkSettings::default() };
    watermark_story(ui, data, "watermark-graphic", start)
}
