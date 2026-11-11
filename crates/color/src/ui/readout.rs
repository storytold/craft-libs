//! `ColorReadout`: a colour as a hex field and editable R, G and B fields (the readout of a Color panel).

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::{ColumnWidth, EditEvent, EditEvents, Mixed};
use craft_theme::Rgba;
use craft_ui::contract::{Bind, Outcome, Pattern, Props as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::{Grid, Stack};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::text::{NumberField, TextField};
use egui::{Response, Ui};

/// The hex field and the three channel fields share the room equally, so the row fits a narrow panel.
const FIELDS: &[ColumnWidth] = &[ColumnWidth::fill(1.0); 4];

/// The channels the fields edit, in field order.
const CHANNELS: [ReadoutChannel; 3] = [ReadoutChannel::Red, ReadoutChannel::Green, ReadoutChannel::Blue];

craft_ui::words! {
    /// The readout's words, English by default.
    #[non_exhaustive]
    pub struct ColorReadoutWords {
        /// The hex field's accessible name.
        hex: "colorReadout.hex" = "Hex color";
        /// The red field's accessible name.
        red: "colorReadout.red" = "Red";
        /// The green field's accessible name.
        green: "colorReadout.green" = "Green";
        /// The blue field's accessible name.
        blue: "colorReadout.blue" = "Blue";
    }
}

/// A channel of the readout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReadoutChannel {
    /// Red.
    Red,
    /// Green.
    Green,
    /// Blue.
    Blue,
}

impl ReadoutChannel {
    /// The channel's place in an [`Rgba`].
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// An edit of one channel of a mixed colour: the host sets that channel of every item and keeps their other channels
/// and their alpha. The value is `None` while the field holds text that is not a number.
#[derive(Debug)]
#[non_exhaustive]
pub struct ChannelEdit {
    /// The channel the field edits.
    pub channel: ReadoutChannel,
    /// The field's edit, as the field reports it.
    pub event: EditEvent<Option<u8>>,
}

/// What a [`ColorReadout`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the edit"]
pub struct ColorReadoutOutput {
    /// The readout's response.
    pub response: Response,
    /// The pass's edits of the whole colour, in order; empty when nothing was edited. A channel field edits the
    /// whole colour only while the value is the same for every item; the hex field always does.
    pub events: EditEvents<Rgba>,
    /// The pass's edits of one channel while the value is mixed, in order; empty when no channel field was edited or
    /// the value is the same for every item.
    pub channel: Vec<ChannelEdit>,
}

impl Outcome for ColorReadoutOutput {
    type Value = Rgba;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        let mut facts = craft_ui::testing::StoryOutput::default();
        facts.id = Some(self.response.id);
        facts.edits = self
            .events
            .iter()
            .filter_map(craft_ui::testing::EditKind::of)
            .chain(self.channel.iter().filter_map(|edit| craft_ui::testing::EditKind::of(&edit.event)))
            .collect();
        facts
    }
}

/// A colour as a hex field and three number fields (R, G, B, each 0 to 255), in one row. An edit in any field
/// reports the whole colour, with the other channels and the alpha as given. While the value is mixed, the hex field
/// still sets the whole colour and a channel field reports that channel alone ([`ColorReadoutOutput::channel`]), for
/// the host to set on each item. Each field's gesture is the readout's.
///
/// AccessKit: a group named by the label, the hex field "Hex color" and a spin button per channel named by its
/// colour.
#[must_use = "show the readout with `show`"]
#[derive(Pattern)]
pub struct ColorReadout<'a> {
    label: Label,
    value: Bind<'a, Rgba>,
    shared: [Mixed<u8>; 3],
}

impl<'a> ColorReadout<'a> {
    /// A readout of `value` named `label`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, Rgba>) -> Self {
        ColorReadout { label: label.into(), value, shared: [Mixed::Mixed; 3] }
    }

    /// The channels (red, green, blue, in [`Rgba`] order) that are the same in every item while the value is mixed,
    /// computed by the host: a channel field shows its shared number and the others show Mixed, as Photoshop's Info
    /// and Color panels do. Ignored while the value is the same for every item.
    pub fn shared_channels(mut self, shared: [Mixed<u8>; 3]) -> Self {
        self.shared = shared;
        self
    }

    /// Shows the readout.
    pub fn show(self, ui: &mut Ui) -> ColorReadoutOutput {
        let ColorReadout { label, mut value, shared } = self;
        let words = Env::of(ui).words::<ColorReadoutWords>();
        let shown = Stack::new().label(label).show(ui, |ui, body| {
            let held: Mixed<Rgba> = value.reborrow().read(&mut *body).unwrap_or(Mixed::Mixed);
            let mut events: Vec<EditEvent<Rgba>> = Vec::new();
            let mut channel_edits = Vec::new();
            Grid::new(FIELDS).show(ui, |ui, _| {
                events.clear();
                channel_edits.clear();
                events.extend(TextField::hex(words.hex.as_str().to_owned(), Bind::given(held)).show(ui).events);
                let names = [&words.red, &words.green, &words.blue];
                for (i, name) in names.into_iter().enumerate() {
                    let field = NumberField::new(
                        name.as_str().to_owned(),
                        Bind::given(match held {
                            Mixed::Same(color) => Mixed::Same(Some(f64::from(color.0[i]))),
                            Mixed::Mixed => match shared[i] {
                                Mixed::Same(byte) => Mixed::Same(Some(f64::from(byte))),
                                Mixed::Mixed => Mixed::Mixed,
                            },
                        }),
                    )
                    .range(0.0..=255.0)
                    .step(1.0)
                    .snap_typed(true)
                    .commit_decimals(0);
                    let byte = |number: Option<f64>| number.map(|number| number.round().clamp(0.0, 255.0) as u8);
                    let out = field.show(ui);
                    for event in &out.events {
                        match held {
                            Mixed::Same(base) => events.push(event.clone().map(move |number| {
                                let mut mine = base;
                                if let Some(byte) = byte(number) {
                                    mine.0[i] = byte;
                                }
                                mine
                            })),
                            Mixed::Mixed => channel_edits.push(ChannelEdit { channel: CHANNELS[i], event: event.clone().map(byte) }),
                        }
                    }
                }
            });
            for event in &events {
                let _ = value.reborrow().write(&mut *body, event);
            }
            (events.into_iter().collect::<EditEvents<_>>(), channel_edits)
        });
        let (events, channel) = shown.inner.unwrap_or_default();
        ColorReadoutOutput { response: shown.output, events, channel }
    }
}
