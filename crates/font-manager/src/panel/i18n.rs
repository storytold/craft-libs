//! The interface languages the panel offers, and its words in them.

use craft_i18n::Embedded;

/// The interface languages offered, in menu order.
pub const LANGUAGES: &[&str] = &["en", "ja", "ar"];

/// The panel's words in Japanese and Arabic: the shared catalogs an app gives the `I18n` of its Host, alone or after its
/// own shared rows. A word without a row shows in English.
pub const CATALOGS: &Embedded = &[("ja", include_str!("catalogs/ja.tsv"), ""), ("ar", include_str!("catalogs/ar.tsv"), "")];
