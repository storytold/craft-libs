//! The channel-mixing adjustments: Black & White, Channel Mixer and Selective Color.

mod black_white;
mod channel_mixer;
mod selective_color;
#[cfg(feature = "screen-testing")]
mod stories;

pub use black_white::{BlackWhite, BlackWhiteEditor, BlackWhiteOutput};
pub use channel_mixer::{ChannelMixer, ChannelMixerEditor};
pub use selective_color::{SelectiveColor, SelectiveColorEditor, SelectiveColorOutput, SelectiveMethod};

use craft_behavior::Value;
use craft_ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft_ui::form::Field;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::Label;
use egui::Ui;

use super::AdjustField;
use super::sliders::Run;

/// A select that picks one of several tables of slider rows, and the rows of the picked table.
struct Tables<'t> {
    name: Label,
    entries: &'t [Entry<usize>],
    picked: usize,
    fields: &'t [AdjustField],
    values: &'t [f64],
}

impl Tables<'_> {
    /// Shows the select and the rows. A row `i` edits through `set(value, i, x)`. Returns the table the user picked.
    fn show<V: Value>(
        &self,
        ui: &mut Ui,
        cx: &mut craft_ui::contract::BodyCx<'_>,
        run: &mut Run<'_, '_, V>,
        set: impl Fn(&V, usize, f64) -> V + Clone,
    ) -> Option<usize> {
        let mut chosen = None;
        Field::new(self.name.clone()).id_salt("select").show(ui, |ui, _| {
            chosen = Select::new(self.name.clone(), Bind::given(self.picked), self.entries).show(ui).value().copied();
        });
        for (i, field) in self.fields.iter().enumerate() {
            let set = set.clone();
            run.slider_row(ui, cx, field, self.values[i], move |now, x| set(now, i, x));
        }
        chosen
    }
}
