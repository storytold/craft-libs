//! Stories of `Swatch`.

use super::{CellMarks, Swatch, SwatchFill, SwatchImage, SwatchShape};
use crate::choice::RadioGroup;
use crate::contract::{Gap, Props as _, Show as _, ShowContainer as _};
use crate::layout_components::{Row, Stack};
use crate::primitives::FacePaint;
use crate::testing::pictures::sample_picture;
use crate::testing::{Drive, StoryData, StoryOutput, show_as_only_item, story};
use craft_behavior::Mixed;
use craft_theme::Srgba;
use egui::Ui;

const BLUE: Srgba = Srgba::rgb(0.24, 0.39, 0.87);

/// The twelve colours of the palette story.
const PALETTE: [Srgba; 12] = [
    Srgba::rgb(0.90, 0.28, 0.30),
    Srgba::rgb(0.97, 0.42, 0.08),
    Srgba::rgb(0.96, 0.65, 0.14),
    Srgba::rgb(0.96, 0.85, 0.04),
    Srgba::rgb(0.48, 0.77, 0.36),
    Srgba::rgb(0.19, 0.64, 0.42),
    Srgba::rgb(0.07, 0.65, 0.58),
    Srgba::rgb(0.00, 0.62, 0.88),
    Srgba::rgb(0.24, 0.39, 0.87),
    Srgba::rgb(0.56, 0.31, 0.78),
    Srgba::rgb(0.84, 0.25, 0.62),
    Srgba::rgb(0.53, 0.55, 0.57),
];

#[story(Swatch<u8>, covers = [Default])]
fn swatch(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Swatch::new("Fill", BLUE).show(ui).into()
}

/// Opaque, translucent, clear, mixed, selected and disabled, each named by its label.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_states(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let clicks = data.note("clicks", 0u32);
    Row::new().show(ui, |ui, _| {
        *clicks += u32::from(Swatch::new("Opaque", BLUE).show(ui).clicked);
        *clicks += u32::from(Swatch::new("Translucent", Srgba { r: 0.90, g: 0.28, b: 0.30, a: 0.43 }).show(ui).clicked);
        *clicks += u32::from(Swatch::new("Clear", Srgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }).show(ui).clicked);
        *clicks += u32::from(Swatch::new("Mixed", Mixed::<Srgba>::Mixed).show(ui).clicked);
        *clicks += u32::from(Swatch::new("Selected", PALETTE[5]).selected(true).show(ui).clicked);
        *clicks += u32::from(Swatch::new("Disabled", PALETTE[2]).enabled(false).show(ui).clicked);
    });
    StoryOutput::default().readout(format!("Clicked {clicks} times"))
}

/// Optional swatches: one of no colour (the sunken well with the diagonal, read as "None"), one with a colour, one
/// mixed.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_none(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let clicks = data.note("clicks", 0u32);
    Row::new().show(ui, |ui, _| {
        *clicks += u32::from(Swatch::optional("No colour", Mixed::<Option<Srgba>>::Same(None)).show(ui).clicked);
        *clicks += u32::from(Swatch::optional("Colour", Mixed::Same(Some(BLUE))).show(ui).clicked);
        *clicks += u32::from(Swatch::optional("Mixed", Mixed::<Option<Srgba>>::Mixed).show(ui).clicked);
    });
    StoryOutput::default().readout(format!("Clicked {clicks} times"))
}

/// Twelve swatches in a `RadioGroup`: one Tab stop, arrows move, Space picks.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_palette(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    RadioGroup::new("Palette", data.bind("palette", 8u8))
        .show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                for (n, color) in (0u8..).zip(PALETTE) {
                    Swatch::new(format!("Colour {}", n + 1), color).member(n).show(ui);
                }
            });
        })
        .into()
}

/// A row of `cell` swatches, one selected: the owning table places them against each other.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_table_cells(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Row::new().gap(Gap::Tight).show(ui, |ui, _| {
        for (n, color) in PALETTE.into_iter().take(6).enumerate() {
            // A cell's place in its set is the owning table's item step; each story cell stands alone in its own.
            Row::new().show(ui, |ui, _| {
                show_as_only_item(ui, Swatch::new(format!("Cell {}", n + 1), color).cell().selected(n == 2));
            });
        }
    });
    StoryOutput::default()
}

/// The well shows a host face over its checker instead of the colour.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_face(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let face = FacePaint::new(1, |cx, rect| {
        cx.painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(0x2f, 0x6f, 0xd9));
    });
    Swatch::new("Face", BLUE).face(face).show(ui).into()
}

/// The colour as a ring, opaque, translucent, selected and mixed.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_stroke(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Row::new().show(ui, |ui, _| {
        Swatch::new("Ring", BLUE).stroke().show(ui);
        Swatch::new("Translucent ring", Srgba { r: 0.90, g: 0.28, b: 0.30, a: 0.43 }).stroke().show(ui);
        Swatch::new("Selected ring", PALETTE[5]).stroke().selected(true).show(ui);
        Swatch::new("Mixed ring", Mixed::<Srgba>::Mixed).stroke().show(ui);
    });
    StoryOutput::default()
}

/// One cell for each mark: edge, locked, web safe, transparent and source colour.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_marks(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let marks = [
        ("Edge", CellMarks::default().edge(true)),
        ("Locked", CellMarks::default().locked(true)),
        ("Web safe", CellMarks::default().web_safe(true)),
        ("Transparent", CellMarks::default().transparent(true)),
        ("Source", CellMarks::default().source(PALETTE[0])),
    ];
    Row::new().gap(Gap::Tight).show(ui, |ui, _| {
        for (name, marks) in marks {
            Row::new().show(ui, |ui, _| {
                show_as_only_item(ui, Swatch::new(name, BLUE).cell().marks(marks));
            });
        }
    });
    StoryOutput::default()
}

/// A hexagon well, the cell of a honeycomb.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_hexagon(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Swatch::new("Hexagon", BLUE).shape(SwatchShape::Hexagon).show(ui).into()
}

/// A round well.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_round(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Swatch::new("Round", BLUE).shape(SwatchShape::Round).show(ui).into()
}

/// A colour before and after a change, in a square and a round well: before at the start edge.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_split(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Row::new().show(ui, |ui, _| {
        for shape in [SwatchShape::Square, SwatchShape::Round] {
            Swatch::with_fill("Reds", SwatchFill::Split(PALETTE[0], PALETTE[2])).shape(shape).show(ui);
        }
    });
    StoryOutput::default()
}

/// The hue wheel in a square and a round well.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_spectrum(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Row::new().show(ui, |ui, _| {
        for shape in [SwatchShape::Square, SwatchShape::Round] {
            Swatch::with_fill("Master", SwatchFill::Spectrum).shape(shape).show(ui);
        }
    });
    StoryOutput::default()
}

/// A picture in a square and a round well.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_image(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let image = SwatchImage::new(sample_picture(7, [64, 64]));
    Row::new().show(ui, |ui, _| {
        for shape in [SwatchShape::Square, SwatchShape::Round] {
            Swatch::with_fill("Sampled", SwatchFill::Image(image.clone())).shape(shape).show(ui);
        }
    });
    StoryOutput::default()
}

/// An empty well: the checkerboard alone.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Swatch::with_fill("Empty", SwatchFill::Empty).show(ui).into()
}

/// A well and a line that counts its clicks.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_clicks(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let clicks = data.note("clicks", 0u32);
    Stack::new().show(ui, |ui, _| {
        if Swatch::new("Fill", BLUE).show(ui).clicked {
            *clicks += 1;
        }
    });
    StoryOutput::default().readout(format!("Clicked {clicks} times"))
}
