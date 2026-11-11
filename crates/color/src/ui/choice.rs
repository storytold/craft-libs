//! `ColorChoice`: a choice among named colours with a custom colour, as Photoshop's pasteboard colour
//! (Preferences, Interface), the Crop shield colour and the Note colour.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_theme::Srgba;
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Props as _, Show as _};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::requests::ColorRequest;
use egui::{Response, Ui};

craft_ui::words! {
    /// The choice's own rows, English by default.
    #[non_exhaustive]
    pub struct ColorChoiceWords {
        /// The row of the custom colour last picked.
        custom: "colorChoice.custom" = "Custom";
        /// The row that opens the Color Picker.
        select_custom: "colorChoice.selectCustom" = "Select Custom Color…";
    }
}

/// One named colour of a [`ColorChoice`]: its id (what the app stores), its label and the colour its swatch shows,
/// `None` for one whose colour the theme decides (Photoshop's "Default" pasteboard).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NamedColor {
    /// What the app stores for this colour.
    pub id: String,
    /// The name shown.
    pub label: Label,
    /// The swatch colour, or `None` to let the theme decide.
    pub color: Option<Srgba>,
}

impl NamedColor {
    /// A named colour with the given id, label and swatch colour.
    pub fn new(id: impl Into<String>, label: impl Into<Label>, color: Option<Srgba>) -> NamedColor {
        NamedColor { id: id.into(), label: label.into(), color }
    }
}

/// What a [`ColorChoice`] holds: one of the named colours by id, or the custom colour.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ChosenColor {
    /// The named colour with this id.
    Named(String),
    /// The custom colour set by [`ColorChoice::custom`].
    Custom,
}

/// What [`ColorChoice::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the choice and opens the picker"]
pub struct ColorChoiceOutput {
    /// The choice's response.
    pub response: Response,
    /// The row chosen this pass; the host stores it.
    pub chosen: Option<ChosenColor>,
    /// "Select Custom Color…" was chosen, for this pass only: the app opens its colour picker from the request (key
    /// [`ColorChoice::CUSTOM_KEY`], the custom colour) and, on OK, stores the colour and chooses
    /// [`ChosenColor::Custom`].
    pub pick_color: Option<ColorRequest>,
}

impl Outcome for ColorChoiceOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What a row of the list stands for.
#[derive(Clone, Debug, PartialEq)]
enum Row {
    Named(String),
    Custom,
    SelectCustom,
}

/// A `Select` of named colours, each row with a swatch before its label, then "Custom" (the custom colour, shown once
/// there is one), a separator and "Select Custom Color…". The closed control shows the chosen row's swatch and label.
/// Keys and AccessKit are the `Select`'s; the swatch is decoration (the label names the row).
#[derive(craft_ui::contract::Pattern)]
pub struct ColorChoice<'a> {
    label: Label,
    value: &'a ChosenColor,
    named: Vec<NamedColor>,
    custom: Option<Srgba>,
    enabled: bool,
}

impl<'a> ColorChoice<'a> {
    /// The key of the [`ColorRequest`] that "Select Custom Color…" reports.
    pub const CUSTOM_KEY: &'static str = "colorChoice.custom";

    /// A choice showing `value`, named `label`.
    pub fn new(label: impl Into<Label>, value: &'a ChosenColor) -> ColorChoice<'a> {
        ColorChoice { label: label.into(), value, named: Vec::new(), custom: None, enabled: true }
    }

    /// The named colours, in order.
    #[must_use]
    pub fn named(mut self, named: impl IntoIterator<Item = NamedColor>) -> Self {
        self.named = named.into_iter().collect();
        self
    }

    /// The custom colour; without it the "Custom" row is not shown.
    #[must_use]
    pub fn custom(mut self, color: Option<Srgba>) -> Self {
        self.custom = color;
        self
    }

    /// Turns input on or off; on by default.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Shows the choice.
    pub fn show(self, ui: &mut Ui) -> ColorChoiceOutput {
        let words = Env::of(ui).words::<ColorChoiceWords>();
        let row = match self.value {
            ChosenColor::Named(id) => Row::Named(id.clone()),
            ChosenColor::Custom => Row::Custom,
        };
        // The picker opens on the custom colour, else on the colour of the chosen row.
        let chosen_color = match self.value {
            ChosenColor::Named(id) => self.named.iter().find(|named| named.id == *id).and_then(|named| named.color),
            ChosenColor::Custom => None,
        };
        let start = self.custom.or(chosen_color).unwrap_or(Srgba::rgb(0.0, 0.0, 0.0));
        let swatch = |entry: Entry<Row>, color: Option<Srgba>| match color {
            Some(color) => entry.swatch(color),
            None => entry,
        };
        let mut entries: Vec<Entry<Row>> =
            self.named.iter().map(|named| swatch(Entry::new(Row::Named(named.id.clone()), named.label.clone()), named.color)).collect();
        if self.custom.is_some() {
            entries.push(swatch(Entry::new(Row::Custom, words.custom.as_str().to_owned()), self.custom));
        }
        entries.push(Entry::separator());
        entries.push(Entry::new(Row::SelectCustom, words.select_custom.as_str().to_owned()));
        let shown = Select::new(self.label.clone(), Bind::given(row), &entries).enabled(self.enabled).show(ui);
        let picked = shown.value().cloned();
        let mut pick_color = None;
        let chosen = match picked {
            Some(Row::Named(id)) => Some(ChosenColor::Named(id)),
            Some(Row::Custom) => Some(ChosenColor::Custom),
            Some(Row::SelectCustom) => {
                let title = words.select_custom.as_str().trim_end_matches('…').to_owned();
                pick_color = Some(ColorRequest::new(Self::CUSTOM_KEY, start).title(title));
                None
            }
            None => None,
        };
        ColorChoiceOutput { response: shown.response, chosen, pick_color }
    }
}
