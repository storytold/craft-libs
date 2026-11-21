//! Stories of `SwatchesPanel`.

use super::{SwatchKey, SwatchLibrary, SwatchesPanel, SwatchesState};
use crate::palette::{SwatchColor, SwatchEntry, SwatchGroup, SwatchPaint};
use craft_theme::Srgba;
use craft_ui::choice::SwatchFace;
use craft_ui::contract::Show as _;
use craft_ui::display::Text;
use craft_ui::patterns::presets::ThumbInfo;
use craft_ui::primitives::{ContentPainterCx, IconSpec};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Rect, Ui};

/// The panel over a library and state kept in the story's data; the reported edits run on the library, so the story stays
/// interactive.
fn interactive(ui: &mut Ui, data: &mut StoryData, name: &'static str, start: SwatchLibrary) -> StoryOutput {
    let ((library, state), library_state_at) = data.value_at(name, (start, SwatchesState::default()));
    let out = SwatchesPanel::new(&library.groups, library_state_at.bind(state), craft_theme::Srgba::rgb(0.2, 0.4, 0.6)).show(ui);
    for edit in &out.edits {
        library.apply(edit);
    }
    out.into()
}

#[story(SwatchesPanel<'static>, covers = [], drive = Drive::Never)]
fn swatches_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    interactive(ui, data, "panel", SwatchLibrary::default())
}

#[story(SwatchesPanel<'static>, covers = [], drive = Drive::Never)]
fn swatches_panel_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    interactive(ui, data, "empty", SwatchLibrary { groups: Vec::new() })
}

/// The host's thumbnail painter: a colour standing for each entry's paint.
fn host_thumb(_: &(), cx: &ContentPainterCx<'_>, rect: Rect, info: ThumbInfo<'_, SwatchKey>) {
    if let SwatchKey::Swatch(name) = info.key {
        let color = match name.as_str() {
            "Sunset" => Srgba::rgb(0.95, 0.5, 0.2),
            "Dots" => Srgba::rgb(0.4, 0.4, 0.45),
            _ => Srgba::rgb(1.0, 0.0, 0.0),
        };
        SwatchFace::new(color).selected(info.selected).paint(cx, rect);
    }
}

/// The host's own rows: its thumbnail painter on a gradient and a pattern entry, an icon trailing each of them, its tools,
/// and no footer.
#[story(SwatchesPanel<'static>, covers = [], drive = Drive::Never)]
fn swatches_panel_host_rows(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = vec![
        SwatchEntry::new("Red", SwatchColor::Rgb([1.0, 0.0, 0.0])),
        SwatchEntry::with_paint("Sunset", SwatchPaint::Gradient),
        SwatchEntry::with_paint("Dots", SwatchPaint::Pattern),
    ];
    let start = SwatchLibrary::new(vec![SwatchGroup { name: "Host".to_owned(), entries }]);
    let ((library, state), library_state_at) = data.value_at("host-rows", (start, SwatchesState::default()));
    let out = SwatchesPanel::new(&library.groups, library_state_at.bind(state), Srgba::rgb(0.2, 0.4, 0.6))
        .thumb((), host_thumb)
        .row_icons(|key| match key {
            SwatchKey::Swatch(name) if name == "Sunset" => vec![(IconSpec::initial("G"), "Gradient".to_owned())],
            SwatchKey::Swatch(name) if name == "Dots" => vec![(IconSpec::initial("P"), "Pattern".to_owned())],
            _ => Vec::new(),
        })
        .tools(|ui| {
            Text::new("Host tools").show(ui);
        })
        .footer(None)
        .show(ui);
    for edit in &out.edits {
        library.apply(edit);
    }
    out.into()
}
