//! `SwatchesPanel`: the Swatches panel on a [`SwatchLibrary`]: groups of flat touching squares, click to use, the panel
//! menu for new swatches, groups and swatch files.

mod library;
mod menu;
mod panel;
#[cfg(feature = "screen-testing")]
mod stories;

pub use library::{SwatchAt, SwatchKey, SwatchLibrary, SwatchesState};
pub use menu::{SwatchesCommand, SwatchesFileOutput, SwatchesMenu, SwatchesMenuOutput};
pub use panel::{SwatchesPanel, SwatchesPanelOutput};

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
        /// The start of the toast after an import that changed something; the notes follow.
        notice_import: "swatches.noticeImport" = "Swatches imported with changes";
        /// The start of the toast after an export that changed something.
        notice_export: "swatches.noticeExport" = "Swatches exported with changes";
        /// The toast when a swatch file could not be imported; `{reason}` says why.
        import_failed: template "swatches.importFailed" = "Swatches could not be imported: {reason}";
        /// The toast when swatches could not be exported; `{reason}` says why.
        export_failed: template "swatches.exportFailed" = "Swatches could not be exported: {reason}";
        groups_flattened: "swatches.groupsFlattened" = "The format has no groups; the swatches were written as one list.";
        book_read: count "swatches.bookRead" = "{n} color-book color was read as black." | "{n} color-book colors were read as black.";
        book_left_out: count "swatches.bookLeftOut" = "{n} color-book color was left out." | "{n} color-book colors were left out.";
        skipped: count "swatches.skipped" = "{n} color in a model that cannot be read was skipped." | "{n} colors in a model that cannot be read were skipped.";
        converted: count "swatches.converted" = "{n} color was converted to RGB." | "{n} colors were converted to RGB.";
        kinds_dropped: count "swatches.kindsDropped" = "{n} spot or global color was written as a process color." | "{n} spot or global colors were written as process colors.";
        truncated: count "swatches.truncated" = "{n} color past the 256 the format holds was left out." | "{n} colors past the 256 the format holds were left out.";
        names_cut: count "swatches.namesCut" = "{n} name was cut to the length the format holds." | "{n} names were cut to the length the format holds.";
        /// The value read for a colour-book colour, which the panel cannot show.
        book_color: "swatches.bookColor" = "Color book color";
    }
}
