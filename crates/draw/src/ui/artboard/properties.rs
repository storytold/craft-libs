//! [`ArtboardProperties`]: an artboard's section in the Properties panel.

use super::bar::SIDE;
use super::{ArtboardBackground, ArtboardItem, ArtboardPreset, ArtboardWords};
use craft_behavior::EditEvents;
use craft_behavior::kurbo::Rect;
use craft_theme::Srgba;
use craft_ui::choice::Swatch;
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Show as _, ShowContainer as _};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, UnitKind};
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::sections::Section;
use craft_ui::text::NumberField;
use egui::{Response, Ui};
use std::ops::RangeInclusive;

/// The range of X and Y, in pixels.
const OFFSET: RangeInclusive<f64> = -300_000.0..=300_000.0;

/// The key of the [`ColorRequest`] Other… files.
const BACKGROUND_KEY: &str = "artboard.background";

/// The background select's rows.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Choice {
    White,
    Black,
    Transparent,
    Other,
}

/// What [`ArtboardProperties::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ArtboardPropertiesOutput {
    /// The section's response.
    pub response: Response,
    /// The board's new rects, in order, from the preset, W, H, X or Y (a `Set`, or a caption scrub's gesture).
    pub rect: EditEvents<Rect>,
    /// A background picked from the select (White, Black or Transparent).
    pub background: Option<ArtboardBackground>,
    /// Other… was chosen, or the swatch of the custom colour was clicked: the app opens its colour picker with this
    /// request (titled "Artboard Background Color") and sets `Custom(color)` from the answer.
    pub pick_color: Option<ColorRequest>,
}

impl Outcome for ArtboardPropertiesOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Artboard section: Size preset, W and H (1 to 300,000 px), X and Y (±300,000 px, all four in pixels) and
/// Background (White, Black, Transparent, Other… with a swatch of the custom colour). Every field is a Tab stop in
/// reading order.
#[must_use = "show the section with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct ArtboardProperties<'a> {
    board: &'a ArtboardItem,
    presets: &'a [ArtboardPreset],
}

impl<'a> ArtboardProperties<'a> {
    /// The section of `board`, with the app's `presets`.
    pub fn new(board: &'a ArtboardItem, presets: &'a [ArtboardPreset]) -> Self {
        ArtboardProperties { board, presets }
    }

    /// Shows the section.
    pub fn show(self, ui: &mut Ui) -> ArtboardPropertiesOutput {
        let words = Env::of(ui).words::<ArtboardWords>();
        let rect = self.board.rect.abs();
        let (width, height) = (rect.width(), rect.height());
        let background = self.board.background;
        let custom = match background {
            ArtboardBackground::Custom(color) => Some(color),
            _ => None,
        };
        let mut rect_events = EditEvents::new();
        let mut picked = None;
        let mut pick_color = None;
        let request = |color: Option<Srgba>| {
            ColorRequest::new(BACKGROUND_KEY, color.unwrap_or(Srgba::rgb(1.0, 1.0, 1.0))).title(words.background_color.clone())
        };
        let mut presets = vec![Entry::new(None, words.custom.clone())];
        presets.extend(self.presets.iter().enumerate().map(|(i, preset)| Entry::new(Some(i), preset.name.clone())));
        let backgrounds = [
            Entry::new(Choice::White, words.white.clone()),
            Entry::new(Choice::Black, words.black.clone()),
            Entry::new(Choice::Transparent, words.transparent.clone()),
            Entry::new(Choice::Other, words.other.clone()),
        ];
        let section = Section::new(words.artboard.clone()).show(ui, |ui, _| {
            FieldGroup::new(words.artboard.clone()).show(ui, |ui, _| {
                let current = self.presets.iter().position(|preset| preset.size.width == width && preset.size.height == height);
                Field::new(words.size.clone()).show(ui, |ui, _| {
                    let pick = Select::new(words.size.clone(), Bind::given(current), &presets).show(ui);
                    if let Some(Some(i)) = pick.value().copied()
                        && let Some(preset) = self.presets.get(i)
                    {
                        let moved = Rect::new(rect.x0, rect.y0, rect.x0 + preset.size.width, rect.y0 + preset.size.height);
                        rect_events.extend(pick.events.map(|_| moved));
                    }
                });
                let field = |ui: &mut Ui,
                             caption: &craft_ui::runtime::env::Label,
                             name: &craft_ui::runtime::env::Label,
                             value: f64,
                             range: &RangeInclusive<f64>| {
                    Field::new(caption.clone())
                        .show(ui, |ui, _| {
                            NumberField::new(name.clone(), Bind::given(Some(value)))
                                .range(range.clone())
                                .unit_kind(UnitKind::Pixels)
                                .show(ui)
                                .edit
                                .events
                        })
                        .inner
                        .unwrap_or_default()
                };
                let resized = |size: (f64, f64)| Rect::new(rect.x0, rect.y0, rect.x0 + size.0, rect.y0 + size.1);
                let moved = |at: (f64, f64)| Rect::new(at.0, at.1, at.0 + width, at.1 + height);
                rect_events.extend(field(ui, &words.width, &words.width_name, width, &SIDE).map(|v| resized((v.unwrap_or(width), height))));
                rect_events.extend(field(ui, &words.height, &words.height_name, height, &SIDE).map(|v| resized((width, v.unwrap_or(height)))));
                rect_events.extend(field(ui, &words.x, &words.x, rect.x0, &OFFSET).map(|v| moved((v.unwrap_or(rect.x0), rect.y0))));
                rect_events.extend(field(ui, &words.y, &words.y, rect.y0, &OFFSET).map(|v| moved((rect.x0, v.unwrap_or(rect.y0)))));
                Field::new(words.background.clone()).show(ui, |ui, _| {
                    Row::new().show(ui, |ui, _| {
                        let now = match background {
                            ArtboardBackground::White => Choice::White,
                            ArtboardBackground::Black => Choice::Black,
                            ArtboardBackground::Transparent => Choice::Transparent,
                            _ => Choice::Other,
                        };
                        match Select::new(words.background.clone(), Bind::given(now), &backgrounds).show(ui).value().copied() {
                            Some(Choice::White) => picked = Some(ArtboardBackground::White),
                            Some(Choice::Black) => picked = Some(ArtboardBackground::Black),
                            Some(Choice::Transparent) => picked = Some(ArtboardBackground::Transparent),
                            Some(Choice::Other) => pick_color = Some(request(custom)),
                            None => {}
                        }
                        if let Some(color) = custom
                            && let Some(asked) = Swatch::new(words.background_color.clone(), color).picker(BACKGROUND_KEY).show(ui).pick_color
                        {
                            pick_color = Some(asked);
                        }
                    });
                });
            });
        });
        ArtboardPropertiesOutput { response: section.response.clone(), rect: rect_events, background: picked, pick_color }
    }
}
