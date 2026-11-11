//! Pages for the Craft document apps: where pages sit on screen, their paper and margins, page
//! ranges and page numbers. The document, its layout and its rendering stay in each app.
//!
//! Core modules (no egui):
//! - [`layout`] (N3.06): page rectangles for one page across, two-page spreads (with or without a
//!   cover page) or as many as fit, continuous or one row at a time; fit width, fit page and fit
//!   height; Word's Hide White Space; the view defaults a document opens with.
//! - [`setup`] (N3.08): paper sizes, orientation, margins and their presets, and the page setup a
//!   Page Setup or Document Setup dialog edits.
//! - [`range`] (N3.13): page ranges typed as "1-3, 5", and Acrobat's From/To/Subset ranges.
//! - [`numbering`] (N3.29): page numbers in a number style (1, i, A, 01…) with a section's start
//!   and prefix.
//!
//! With `ui` (on craft-ui), [`ui`] has the widgets: the page view, the thumbnails strip, the page
//! setup dialog and page layout menus, margin handles, header and footer zones, the page range
//! field, the text selection layer, the page preview, text frame marks, the column bar, the
//! reading view, the page list, synced views, page grids, the accessibility bridge and document
//! tables.
//!
//! Lengths are PDF points (1/72 inch) throughout; screen lengths are egui points.

pub mod layout;
#[cfg(test)]
mod layout_tests;
pub mod numbering;
pub mod range;
#[cfg(test)]
mod range_tests;
pub mod setup;
#[cfg(feature = "ui")]
pub mod ui;

pub use layout::{
    Fit, LayoutOptions, PageDisplay, PageSpacing, PagedLayout, SCREEN_POINTS_PER_POINT, ViewDefaults, ZOOM_RANGE, display_token, fit_scale, page_at,
    parse_display_token, place_pages, shown_row, spread_first,
};
pub use numbering::{NumberStyle, PAGE_NUMBER_STYLES, PageNumbering};
pub use range::{PageRange, PageRangeError, Subset, format_pages, parse_pages};
pub use setup::{MARGIN_PRESETS, MarginPreset, Margins, Orientation, PAPER_SIZES, PageSetup, PageVerticalAlign, PaperSize, paper_named};

/// The stories and trials this crate registers.
#[cfg(feature = "screen-testing")]
pub mod testing {
    craft_ui::testing_registered!();
}

#[cfg(all(test, feature = "screen-testing"))]
extern crate self as craft_pages;
#[cfg(all(test, feature = "screen-testing"))]
#[path = "../tests/it/main.rs"]
mod integration;

#[cfg(test)]
fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::registered_in(env!("CARGO_PKG_NAME")));
}
