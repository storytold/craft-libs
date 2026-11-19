//! The app layer editor: the base overrides and component pins of an app's `sizes.json`, edited from the size rows of
//! the browser and applied to every theme the gallery shows.
//!
//! The file moves as text: it is loaded by pasting it into the field and exported by writing it to the field.

use super::subject::Pick;
use craft_theme::variant::Tone;
use craft_theme::{AppSizes, Explained, SizeMap, SizeOrigin, SizeValue};
use craft_ui::buttons::Button;
use craft_ui::choice::Segmented;
use craft_ui::contract::{Bind, Entry, Gap, Held, Pattern, Props, Scroll, Show as _, ShowContainer as _, clicked};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::notices::Notice;
use craft_ui::runtime::memory::Remember as _;
use craft_ui::sections::Section;
use craft_ui::text::{NumberField, TextField};
use egui::Ui;

/// The rows of the JSON field.
const TEXT_ROWS: usize = 10;

/// The part of `sizes.json` an edit goes to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Part {
    #[default]
    All,
    Compact,
    Comfortable,
}

/// What the editor keeps between passes: the part edited, the text of the file and what the last action reported.
#[derive(Clone, Default)]
struct Session {
    part: Part,
    text: String,
    /// The outcome of the last load or export, and whether it is an error.
    status: Option<(String, bool)>,
}

/// Parses the file `bytes` named `name` as an app's `sizes.json`; the error names `name` and the offending paths.
pub fn parse_sizes(name: &str, bytes: &[u8]) -> Result<AppSizes, String> {
    AppSizes::parse(bytes).map_err(|errors| format!("{name}: {errors}"))
}

fn part_map(sizes: &AppSizes, part: Part) -> &SizeMap {
    match part {
        Part::All => &sizes.all,
        Part::Compact => &sizes.compact,
        Part::Comfortable => &sizes.comfortable,
    }
}

fn part_map_mut(sizes: &mut AppSizes, part: Part) -> &mut SizeMap {
    match part {
        Part::All => &mut sizes.all,
        Part::Compact => &mut sizes.compact,
        Part::Comfortable => &mut sizes.comfortable,
    }
}

fn number(value: SizeValue) -> f32 {
    match value {
        SizeValue::Number(n) => n,
    }
}

/// The app's size overrides as an editor: for a picked size, its override in the chosen part of the file; always the
/// file's text with Load, Export and Clear. Edits write to the bound layer.
#[derive(Pattern, Props)]
pub struct AppLayerEditor<'a> {
    sizes: Held<'a, AppSizes>,
    explained: &'a Explained,
    /// The row the host has picked; a size row shows its override editor.
    #[prop]
    pick: Option<&'a Pick>,
}

impl<'a> AppLayerEditor<'a> {
    /// An editor over the layer `sizes`, with the sizes of `explained` to pick from.
    pub fn new(sizes: Bind<'a, AppSizes>, explained: &'a Explained) -> Self {
        AppLayerEditor { sizes: Held::new(sizes), explained, pick: None }
    }

    /// Shows the editor across the room offered, scrolling. True when an edit wrote the layer this pass.
    pub fn show(self, ui: &mut Ui) -> bool {
        let AppLayerEditor { mut sizes, explained, pick } = self;
        let size = match pick {
            Some(Pick::Size(path)) => explained.sizes.iter().find(|origin| origin.path == *path),
            _ => None,
        };
        let mut edited = false;
        Stack::new().fill().scroll(Scroll::vertical()).gap(Gap::Section).show(ui, |ui, cx| {
            let Some(before) = sizes.read(cx) else { return };
            let mut layer = before.clone();
            let mut session = cx.memory::<Session>().clone();
            if let Some(origin) = size {
                override_section(ui, origin, &mut session, &mut layer);
            }
            file_section(ui, &mut session, &mut layer);
            *cx.memory::<Session>() = session;
            if layer != before {
                sizes.write(cx, &before, layer, "Edit app sizes");
                edited = true;
            }
        });
        edited
    }
}

/// The value of the picked size in the chosen part of the file, and the control to remove it.
fn override_section(ui: &mut Ui, origin: &SizeOrigin, session: &mut Session, layer: &mut AppSizes) {
    Section::fixed(format!("Override {}", origin.path)).show(ui, |ui, _| {
        let parts = [Entry::new(Part::All, "All"), Entry::new(Part::Compact, "Compact"), Entry::new(Part::Comfortable, "Comfortable")];
        Segmented::new("Part of sizes.json", Bind::bound(&mut session.part), &parts).show(ui);
        let set = part_map(layer, session.part).get(&origin.path).copied();
        let before = Some(f64::from(number(set.unwrap_or(origin.value))));
        let mut value = before;
        NumberField::new("Override", Bind::bound(&mut value)).range(f64::from(origin.range.0)..=f64::from(origin.range.1)).show(ui);
        if value != before
            && let Some(edited) = value
        {
            part_map_mut(layer, session.part).insert(origin.path.clone(), SizeValue::Number(edited as f32));
        }
        if clicked(&Button::new("Remove override").enabled(set.is_some()).show(ui)) {
            part_map_mut(layer, session.part).remove(&origin.path);
        }
    });
}

/// The file's text, the buttons that move it in and out of the layer and the outcome of the last one.
fn file_section(ui: &mut Ui, session: &mut Session, layer: &mut AppSizes) {
    Section::fixed("sizes.json").show(ui, |ui, _| {
        let counts = (layer.all.len(), layer.compact.len(), layer.comfortable.len());
        Text::new(format!("{} overrides: {} for all, {} Compact, {} Comfortable", counts.0 + counts.1 + counts.2, counts.0, counts.1, counts.2))
            .show(ui);
        TextField::new("sizes.json", Bind::bound(&mut session.text)).placeholder("Paste an app's sizes.json").lines(TEXT_ROWS).show(ui);
        Row::new().wrap(true).show(ui, |ui, _| {
            if clicked(&Button::new("Load").show(ui)) {
                match parse_sizes("sizes.json", session.text.as_bytes()) {
                    Ok(loaded) => {
                        *layer = loaded;
                        session.status = Some(("Loaded sizes.json".to_owned(), false));
                    }
                    Err(message) => session.status = Some((message, true)),
                }
            }
            if clicked(&Button::new("Export").show(ui)) {
                session.text = layer.to_json();
                session.status = Some(match parse_sizes("sizes.json", session.text.as_bytes()) {
                    Ok(_) => ("Wrote sizes.json to the field; copy it from there".to_owned(), false),
                    Err(errors) => (format!("sizes.json would not load: {errors}"), true),
                });
            }
            if clicked(&Button::new("Clear").enabled(!layer.is_empty()).show(ui)) {
                *layer = AppSizes::default();
                session.text.clear();
                session.status = None;
            }
        });
        if let Some((message, error)) = &session.status {
            Notice::new(if *error { Tone::Bad } else { Tone::Good }, message.clone()).show(ui);
        }
    });
}
