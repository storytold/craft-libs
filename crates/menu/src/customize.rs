//! The user's menu customization: rows they hid and colours they gave rows.

use std::collections::{BTreeMap, BTreeSet};

use craft_theme::Srgba;

use crate::model::Row;

/// Edit › Menus: the command ids the user hid and the colour each coloured command has.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Customization {
    /// The ids of the commands hidden from the menus.
    pub hidden: BTreeSet<String>,
    /// The colour of each coloured command, by id.
    pub colors: BTreeMap<String, Srgba>,
}

impl Customization {
    /// A customization hiding the `hidden` ids and colouring each `colors` id.
    pub fn new(hidden: impl IntoIterator<Item = String>, colors: impl IntoIterator<Item = (String, Srgba)>) -> Self {
        Customization { hidden: hidden.into_iter().collect(), colors: colors.into_iter().collect() }
    }

    /// Drops the rows whose command id is hidden and sets the tint of the rows whose id has a
    /// colour. Separators stay; `from_items` tidies them.
    pub fn apply(&self, rows: &mut Vec<Row>) {
        rows.retain(|row| !self.hidden.contains(&row.command.id));
        for row in rows.iter_mut() {
            if let Some(color) = self.colors.get(&row.command.id) {
                row.tint = Some(*color);
            }
        }
    }
}
