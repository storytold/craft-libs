//! `SwatchesPanel`: the Swatches panel on the groups of a [`SwatchLibrary`]: groups of flat touching squares, click to
//! use, the panel menu for new swatches, groups and swatch files. The panel never writes the library: it reports each
//! change as a resolved [`SwatchEdit`] (new, rename, delete, move, group edits, import) that the host runs, with
//! [`SwatchLibrary::apply`] for an in-memory host.

use craft_ui::tile_grid::TileZoom;

pub(crate) mod library;
mod menu;
pub(crate) mod panel;
#[cfg(feature = "suite")]
pub(crate) mod stories;

pub use library::{SwatchAt, SwatchEdit, SwatchKey, SwatchLibrary, SwatchesState, swatch_rows, swatch_source};
pub use menu::{SwatchesCommand, SwatchesFileOutput, SwatchesMenu, SwatchesMenuOutput};
pub use panel::{SwatchesPanel, SwatchesPanelOutput};

/// Compact colour tiles at the default zoom, with the same upper limit as the preset browser.
const SWATCH_MIN_ZOOM: f64 = 1.0 / 3.0;
const SWATCH_ZOOM: TileZoom = TileZoom::bounded(SWATCH_MIN_ZOOM, 5.0, 1.0 / 12.0, SWATCH_MIN_ZOOM);

/// The most swatches a library holds (PhotoCraft's `MAX_SWATCHES`).
pub const MAX_SWATCHES: usize = 100_000;

/// The most groups a library holds (PhotoCraft's `MAX_GROUPS`).
pub const MAX_GROUPS: usize = 10_000;

craft_ui::words! {
    /// The Swatches panel's words.
    #[non_exhaustive]
    pub struct SwatchesWords {
        title: "swatches.title" = "Swatches";
        /// The name of a new group.
        group: "swatches.group" = "Group";
        /// A new swatch's name; `{n}` numbers it.
        swatch_n: template "swatches.swatchN" = "Color Swatch {n}";
        /// The name of the file filter of the swatch pickers, and of an exported file.
        file: "swatches.file" = "Swatches";
        /// The start of the status message after an import that changed something; the notes follow.
        notice_import: "swatches.noticeImport" = "Swatches imported with changes";
        /// The start of the status message after an export that changed something.
        notice_export: "swatches.noticeExport" = "Swatches exported with changes";
        /// The status message when a swatch file could not be imported; `{reason}` says why.
        import_failed: template "swatches.importFailed" = "Swatches could not be imported: {reason}";
        /// The status message when swatches could not be exported; `{reason}` says why.
        export_failed: template "swatches.exportFailed" = "Swatches could not be exported: {reason}";
        groups_flattened: "swatches.groupsFlattened" = "The format has no groups; the swatches were written as one list.";
        book_read: count "swatches.bookRead" = "{n} color-book color was read as black." | "{n} color-book colors were read as black.";
        book_left_out: count "swatches.bookLeftOut" = "{n} color-book color was left out." | "{n} color-book colors were left out.";
        skipped: count "swatches.skipped" = "{n} color in a model that cannot be read was skipped." | "{n} colors in a model that cannot be read were skipped.";
        converted: count "swatches.converted" = "{n} color was converted to RGB." | "{n} colors were converted to RGB.";
        kinds_dropped: count "swatches.kindsDropped" = "{n} spot or global color was written as a process color." | "{n} spot or global colors were written as process colors.";
        paints_skipped: count "swatches.paintsSkipped" = "{n} gradient, pattern or None swatch was not written." | "{n} gradient, pattern or None swatches were not written.";
        truncated: count "swatches.truncated" = "{n} color past the 256 the format holds was left out." | "{n} colors past the 256 the format holds were left out.";
        transparent_kept: count "swatches.transparentKept" = "{n} color was kept transparent." | "{n} colors were kept transparent.";
        names_cut: count "swatches.namesCut" = "{n} name was cut to the length the format holds." | "{n} names were cut to the length the format holds.";
        /// The value read for a colour-book colour, which the panel cannot show.
        book_color: "swatches.bookColor" = "Color book color";
    }
}
