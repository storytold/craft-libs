//! `Colours`: every semantic colour role of a token set as a swatch row, by group.

use craft_behavior::ColumnWidth;
use craft_theme::{ColorToken, Rgba, Srgba, Tokens};
use craft_ui::choice::Swatch;
use craft_ui::contract::{Gap, Pattern, Props, Show as _, ShowContainer as _};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Grid, Stack};
use craft_ui::style::TypeRole;
use egui::Ui;

/// The swatch, the role's path and its value.
const COLUMNS: &[ColumnWidth] = &[ColumnWidth::Content, ColumnWidth::fill(1.0), ColumnWidth::Content];

/// The semantic colour roles of `tokens` in their order, a group of rows under each first segment of their paths
/// (surface, text, line…): each row the role's colour, its path and its value as hex.
#[derive(Pattern, Props)]
pub struct Colours<'a> {
    tokens: &'a Tokens,
}

impl<'a> Colours<'a> {
    /// The colour roles of `tokens`.
    pub fn new(tokens: &'a Tokens) -> Self {
        Colours { tokens }
    }

    /// Shows the groups one under the other.
    pub fn show(self, ui: &mut Ui) {
        let tokens = self.tokens;
        let mut groups: Vec<(String, Vec<ColorToken>)> = Vec::new();
        for &token in ColorToken::ALL {
            let path = token.path();
            let group = path.split('.').next().unwrap_or_default();
            match groups.last_mut() {
                Some((last, roles)) if last == group => roles.push(token),
                _ => groups.push((group.to_owned(), vec![token])),
            }
        }
        Stack::new().fill_width().gap(Gap::Section).show(ui, |ui, _| {
            for (group, roles) in &groups {
                Stack::new().fill_width().gap(Gap::Tight).show(ui, |ui, _| {
                    Text::new(group.clone()).role(TypeRole::BodyStrong).show(ui);
                    Grid::new(COLUMNS).gap(Gap::Tight).fill_width().show(ui, |ui, _| {
                        for &token in roles {
                            let colour = tokens.color(token);
                            Swatch::new(token.path(), Srgba::from(colour)).show(ui);
                            Text::new(token.path()).role(TypeRole::Label).show(ui);
                            Text::new(hex(colour)).role(TypeRole::Mono).shade(Shade::Muted).show(ui);
                        }
                    });
                });
            }
        });
    }
}

/// `#rrggbb`, or `#rrggbbaa` when the colour is translucent.
fn hex(colour: Rgba) -> String {
    let [r, g, b, a] = colour.0;
    if a == u8::MAX { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
}
