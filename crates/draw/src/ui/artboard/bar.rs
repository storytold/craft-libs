//! [`ArtboardBar`]: the members the Artboard tool adds to the app's options `Bar`.

use super::background::background_cells;
use super::{ArtboardBackground, ArtboardItem, ArtboardPreset, ArtboardWords};
use craft_behavior::IconId;
use craft_behavior::kurbo::Size;
use craft_behavior::{EditEvent, EditEvents, Mixed};
use craft_ui::bars::{AlignAction, BarItem, BarItems, ItemDisplay};
use craft_ui::buttons::{Button, IconButton};
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, PopupKind, Props as _, Show as _, ShowContainer as _, Trigger, clicked};
use craft_ui::display::Divider;
use craft_ui::form::Field;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, UnitKind};
use craft_ui::runtime::requests::ColorRequest;
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
    /// The target's new size: a preset pick, Make Portrait or Make Landscape, or a typed W or H is a `Set`; a
    /// scrub of a caption is a gesture. The app resizes the board (contents stay) or the next board's size.
    pub size: EditEvents<Size>,
    /// The background picked.
    pub background: Option<ArtboardBackground>,
    /// The request for the app's colour picker.
    pub pick_color: Option<ColorRequest>,
}

impl Outcome for ArtboardBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The members the Artboard tool adds to the options bar, in PhotoCraft's order: the Size preset (Custom, then the
/// app's presets), W and H (1 to 300,000 px), Make Portrait (shown only while the size is wider than tall), Make
/// Landscape (only while taller than wide; both are off for the next board), the Background cells, a rule, Add New
/// Artboard ([`ArtboardCommand::AddNew`]), a rule and the Align menu button. Each member is a Tab stop in the bar's
/// order.
#[must_use = "show the members with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct ArtboardBar<'a> {
    target: ArtboardBarTarget<'a>,
    presets: &'a [ArtboardPreset],
}

/// What the bar edits: a selected board, or the next board to place.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum ArtboardBarTarget<'a> {
    /// The selected board.
    Board(&'a ArtboardItem),
    /// No board is selected: the size and background the next board gets.
    #[non_exhaustive]
    Next {
        /// The size of the next board.
        size: Size,
        /// Its background.
        background: ArtboardBackground,
    },
}

impl ArtboardBarTarget<'_> {
    /// No board is selected: the `size` and `background` the next board gets.
    pub const fn next(size: Size, background: ArtboardBackground) -> Self {
        ArtboardBarTarget::Next { size, background }
    }
}

impl<'a> ArtboardBar<'a> {
    /// The members for `target` with the app's `presets`.
    pub fn new(target: ArtboardBarTarget<'a>, presets: &'a [ArtboardPreset]) -> Self {
        ArtboardBar { target, presets }
    }

    /// Adds the members to the bar's row.
    pub fn show(self, ui: &mut Ui) -> ArtboardBarOutput {
        let words = Env::of(ui).words::<ArtboardWords>();
        let (shown, background, has_board) = match self.target {
            ArtboardBarTarget::Board(board) => (Size::new(board.rect.width().abs(), board.rect.height().abs()), board.background, true),
            ArtboardBarTarget::Next { size, background } => (size, background, false),
        };
        let mut out = EditEvents::new();
        // The preset the size matches, else Custom.
        let mut entries = vec![Entry::new(None, words.custom.clone())];
        entries.extend(self.presets.iter().enumerate().map(|(i, preset)| Entry::new(Some(i), preset.name.clone())));
        let current = self.presets.iter().position(|preset| preset.size == shown);
        Field::new(words.size.clone()).show(ui, |ui, _| {
            let pick = Select::new(words.size.clone(), Bind::given(current), &entries).show(ui);
            if let Some(Some(i)) = pick.value().copied()
                && let Some(preset) = self.presets.get(i)
            {
                out = pick.events.map(|_| preset.size);
            }
        });
        let side = |ui: &mut Ui, caption: &craft_ui::runtime::env::Label, name: &craft_ui::runtime::env::Label, value: f64, width: bool| {
            Field::new(caption.clone())
                .show(ui, |ui, _| {
                    let field = NumberField::new(name.clone(), Bind::given(Some(value))).range(SIDE).unit_kind(UnitKind::Pixels).show(ui);
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
        // Make Portrait is for a size wider than tall, Make Landscape for one taller than wide.
        let swapped = Size::new(shown.height, shown.width);
        let swap = |ui: &mut Ui, label: &craft_ui::runtime::env::Label| {
            clicked(&Button::new(label.clone()).enabled(has_board).show(ui))
                .then(|| EditEvent::set(Mixed::Same(shown), swapped, label.as_str().into(), None))
        };
        if shown.width > shown.height {
            size_events.extend(swap(ui, &words.make_portrait));
        }
        if shown.height > shown.width {
            size_events.extend(swap(ui, &words.make_landscape));
        }
        let mut cells = None;
        Field::new(words.background.clone()).show(ui, |ui, _| {
            cells = Some(background_cells(ui, &words, background));
        });
        Divider::vertical().show(ui);
        let response = BarItems::new(vec![BarItem::new(ArtboardCommand::AddNew).display(ItemDisplay::Text)]).label(words.commands.clone()).show(ui);
        Divider::vertical().show(ui);
        let button = IconButton::new(IconId::new("align_edge_left")).label(words.align.clone()).popup(PopupKind::Menu).show(ui);
        Menu::new(words.align.clone()).opened_by(&button.response, Trigger::Press).show(ui, |ui, _| {
            for action in AlignAction::ALIGN {
                MenuItem::command(action).show(ui);
            }
            Divider::horizontal().show(ui);
            MenuItem::new(words.distribute_widths.clone()).enabled(false).show(ui);
            MenuItem::new(words.distribute_heights.clone()).enabled(false).show(ui);
        });
        let cells = cells.unwrap_or_default();
        ArtboardBarOutput { response, size: size_events, background: cells.picked, pick_color: cells.pick_color }
    }
}
