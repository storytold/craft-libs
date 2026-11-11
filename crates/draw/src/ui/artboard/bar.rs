//! [`ArtboardBar`]: the members the Artboard tool adds to the app's options `Bar`.

use super::{ArtboardItem, ArtboardPreset, ArtboardWords};
use craft_behavior::kurbo::Size;
use craft_behavior::{EditEvent, EditEvents, Mixed};
use craft_ui::bars::{BarItem, BarItems, ItemDisplay};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::display::Divider;
use craft_ui::form::Field;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, UnitKind};
use craft_ui::text::NumberField;
use egui::{Response, Ui};
use std::ops::RangeInclusive;

/// The smallest and largest side of a board, in pixels (PhotoCraft's field range).
pub(super) const SIDE: RangeInclusive<f64> = 1.0..=300_000.0;

/// The commands of the Artboard bar. The app owns the boards and takes the command when it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui::contract::CommandSet)]
#[command(prefix = "artboardCommand")]
#[non_exhaustive]
pub enum ArtboardCommand {
    /// A board of the selected board's size, else the canvas's.
    #[command(label = "Add New Artboard")]
    AddNew,
}

/// What [`ArtboardBar`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ArtboardBarOutput {
    /// The members' response.
    pub response: Response,
    /// The selected board's new size: a preset pick, Make Portrait or Make Landscape, or a typed W or H is a `Set`; a
    /// scrub of a caption is a gesture. The app resizes the board (contents stay).
    pub size: EditEvents<Size>,
}

impl Outcome for ArtboardBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The members the Artboard tool adds to the options bar, in PhotoCraft's order: the Size preset (Custom, then the
/// app's presets), W and H (1 to 300,000 px), Make Portrait (shown only while the board is wider than tall), Make
/// Landscape (only while taller than wide), a rule and Add New Artboard ([`ArtboardCommand::AddNew`]). Without a
/// selected board the size members are off. Each member is a Tab stop in the bar's order.
#[must_use = "show the members with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct ArtboardBar<'a> {
    selected: Option<&'a ArtboardItem>,
    presets: &'a [ArtboardPreset],
}

impl<'a> ArtboardBar<'a> {
    /// The members for the `selected` board with the app's `presets`.
    pub fn new(selected: Option<&'a ArtboardItem>, presets: &'a [ArtboardPreset]) -> Self {
        ArtboardBar { selected, presets }
    }

    /// Adds the members to the bar's row.
    pub fn show(self, ui: &mut Ui) -> ArtboardBarOutput {
        let words = Env::of(ui).words::<ArtboardWords>();
        let size = self.selected.map(|board| Size::new(board.rect.width().abs(), board.rect.height().abs()));
        let shown = size.unwrap_or(Size::ZERO);
        let on = size.is_some();
        let mut out = EditEvents::new();
        // The preset the board's size matches, else Custom.
        let mut entries = vec![Entry::new(None, words.custom.clone())];
        entries.extend(self.presets.iter().enumerate().map(|(i, preset)| Entry::new(Some(i), preset.name.clone())));
        let current = self.presets.iter().position(|preset| preset.size == shown);
        Field::new(words.size.clone()).enabled(on).show(ui, |ui, _| {
            let pick = Select::new(words.size.clone(), Bind::given(current), &entries).enabled(on).show(ui);
            if let Some(Some(i)) = pick.value().copied()
                && let Some(preset) = self.presets.get(i)
            {
                out = pick.events.map(|_| preset.size);
            }
        });
        let side = |ui: &mut Ui, caption: &craft_ui::runtime::env::Label, name: &craft_ui::runtime::env::Label, value: f64, width: bool| {
            Field::new(caption.clone())
                .enabled(on)
                .show(ui, |ui, _| {
                    let field = NumberField::new(name.clone(), Bind::given(Some(value))).range(SIDE).unit_kind(UnitKind::Pixels).enabled(on).show(ui);
                    field.edit.events.map(|v| {
                        if width { Size::new(v.unwrap_or(shown.width), shown.height) } else { Size::new(shown.width, v.unwrap_or(shown.height)) }
                    })
                })
                .inner
                .unwrap_or_default()
        };
        let width = side(ui, &words.width, &words.width_name, shown.width, true);
        let height = side(ui, &words.height, &words.height_name, shown.height, false);
        let mut size_events = out;
        size_events.extend(width);
        size_events.extend(height);
        // Make Portrait is for a board wider than tall, Make Landscape for one taller than wide.
        let swapped = Size::new(shown.height, shown.width);
        let swap = |ui: &mut Ui, label: &craft_ui::runtime::env::Label| {
            clicked(&Button::new(label.clone()).show(ui)).then(|| EditEvent::set(Mixed::Same(shown), swapped, label.as_str().into(), None))
        };
        if shown.width > shown.height {
            size_events.extend(swap(ui, &words.make_portrait));
        }
        if shown.height > shown.width {
            size_events.extend(swap(ui, &words.make_landscape));
        }
        Divider::vertical().show(ui);
        let response = BarItems::new(vec![BarItem::new(ArtboardCommand::AddNew).display(ItemDisplay::Text)]).label(words.commands.clone()).show(ui);
        ArtboardBarOutput { response, size: size_events }
    }
}
