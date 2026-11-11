#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Shared text processing for the Craft apps. It sits below the composer: layout code calls it on
//! plain text and a language before it shapes and places glyphs.
//!
//! Modules, each behind the cargo feature of the same name where they need a dependency of their
//! own (`quotes`, `styles` and `lists` are always built):
//! - [`hyphen`] (default): hyphenation points, with bundled US English data and optional
//!   hypher patterns for 47 more languages (Slovak, under the GPL, only with `slovak-gpl`).
//! - [`segment`] (default): line-break opportunities (UAX #14 with kinsoku), and word breaks for
//!   Thai, Lao, Khmer and Burmese with their language features.
//! - [`bidi`] (default): bidirectional text (UAX #9): embedding levels, line reordering and
//!   mirroring.
//! - [`linebreak`]: line breaking on a font-independent box/glue/penalty model.
//! - [`spell`]: spelling, with a bundled US English word list and suggestions.
//! - [`quotes`]: smart quotes, the straight quotes typed into text turned into the language's own.
//! - [`find`]: find and replace patterns (Acrobat's and Word's options, Word's codes and
//!   wildcards) and the matches of a search across pages; pulls regex.
//! - [`styles`]: the rules named style lists share (folders in names, the next free name).
//! - [`lists`]: bullet and number markers per list level, and the number styles page numbers use
//!   too.
//! - [`ui`] (egui, on craft-ui): the canvas text editor, the Character and Paragraph controls and
//!   the Paragraph dialog, the text style panels and the style picker, the Check Spelling dialog,
//!   and the paged-text controls: the text ruler, the Tabs dialog, the notes editor, the Columns
//!   and Hyperlink dialogs, the find bar and the Find and Replace dialog. With `font-picker`, the
//!   Character controls' family field is craft-font-manager's font picker.
//!
//! Language features (`spanish`, `german`, …, `all-languages`) add data for that language and
//! turn on the modules that use it.

#[cfg(feature = "bidi")]
pub mod bidi;
#[cfg(feature = "find")]
pub mod find;
#[cfg(all(test, feature = "find"))]
mod find_tests;
#[cfg(feature = "hyphen")]
pub mod hyphen;
pub mod linebreak;
pub mod lists;
#[cfg(test)]
mod lists_tests;
pub mod quotes;
#[cfg(feature = "segment")]
pub mod segment;
#[cfg(feature = "spell")]
pub mod spell;
#[cfg(all(test, feature = "spell"))]
mod spell_tests;
pub mod styles;
#[cfg(feature = "ui")]
pub mod ui;

/// The stories and trials of the crate's UI, registered in craft-ui's registry.
#[cfg(feature = "screen-testing")]
pub mod testing {
    craft_ui::testing_registered!();
}

#[cfg(all(test, feature = "screen-testing"))]
extern crate self as craft_text;
#[cfg(all(test, feature = "screen-testing"))]
#[path = "../tests/it/main.rs"]
mod integration;

#[cfg(test)]
fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::registered_in(env!("CARGO_PKG_NAME")));
}
