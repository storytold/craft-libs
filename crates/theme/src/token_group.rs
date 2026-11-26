//! Token groups: a crate's own namespaced semantic tokens, derived from the resolved tokens and overlaid by a
//! theme's values under the group's path.

use crate::color::Rgba;
use crate::tokens::Tokens;

/// The values a theme sets under token group paths: `(path, field)` to colour.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupValues {
    colors: Vec<(String, String, Rgba)>,
}

impl GroupValues {
    /// No values.
    pub const fn new() -> Self {
        GroupValues { colors: Vec::new() }
    }

    /// Sets the colour of `field` under `path`, replacing an earlier value.
    pub fn set_color(&mut self, path: &str, field: &str, color: Rgba) -> &mut Self {
        match self.colors.iter_mut().find(|(p, f, _)| p == path && f == field) {
            Some(entry) => entry.2 = color,
            None => self.colors.push((path.to_owned(), field.to_owned(), color)),
        }
        self
    }

    /// The theme's colour for `field` under `path`.
    pub fn color(&self, path: &str, field: &str) -> Option<Rgba> {
        self.colors.iter().find(|(p, f, _)| p == path && f == field).map(|(_, _, c)| *c)
    }

    /// Every `(path, field)` set, for the inspector's list of values no group reads.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.colors.iter().map(|(p, f, _)| (p.as_str(), f.as_str()))
    }
}

/// A crate's namespaced group of semantic tokens for its own concepts. `#[derive(TokenGroup)]` writes `PATH` and
/// `overlay`; the author writes `derive`.
pub trait TokenGroup: Clone + Send + Sync + 'static {
    /// Crate-prefixed path the theme addresses the group's fields under.
    const PATH: &'static str;

    /// The group's defaults from the resolved semantic tokens (mode, contrast, colours, shape). A literal
    /// colour is allowed here as a default a theme overrides.
    fn derive(tokens: &Tokens) -> Self;

    /// Lays the theme's values under `PATH` over the derived defaults. A group without theme-addressable
    /// fields keeps the default, which changes nothing.
    fn overlay(&mut self, _values: &GroupValues) {}
}
