//! The words of the Paths panel ([`PathsWords`]).

craft_ui::words! {
    /// The words of [`PathsPanel`](super::PathsPanel), English by default (Photoshop's).
    pub struct PathsWords {
        /// The panel's accessible name.
        panel: "paths.panel" = "Paths";
        /// The list's name for a screen reader.
        list: "paths.list" = "Path list";
        /// What the empty list says.
        empty: "paths.empty" = "Draw with the Pen tool (P) or make a work path from a selection.";
        /// The default name of the work path.
        work_path: "paths.workPath" = "Work Path";
        /// The row menu's name.
        row_menu: "paths.rowMenu" = "Path";
        /// The footer's Fill Path button.
        fill_path: "paths.fillPath" = "Fill path with foreground color";
        /// The footer's Stroke Path button.
        stroke_path: "paths.strokePath" = "Stroke path with brush";
        /// The footer's Load button.
        load_selection: "paths.loadSelection" = "Load path as a selection";
        /// The footer's Make Work Path button.
        make_work_path: "paths.makeWorkPath" = "Make work path from selection";
        /// The footer's Vector Mask button.
        vector_mask: "paths.vectorMask" = "Add vector mask";
        /// The footer's New button.
        new_path: "paths.newPath" = "Create new path";
        /// The footer's Delete button.
        delete_path: "paths.deletePath" = "Delete current path";
        /// The footer's Delete button and the row menu's Delete while the selected row is a vector mask.
        delete_vector_mask: "paths.deleteVectorMask" = "Delete Vector Mask";
        /// The row menu's Make Selection.
        menu_make_selection: "paths.menuMakeSelection" = "Make Selection…";
        /// The row menu's Fill Path.
        menu_fill: "paths.menuFill" = "Fill Path…";
        /// The row menu's Stroke Path.
        menu_stroke: "paths.menuStroke" = "Stroke Path…";
        /// The row menu's Save Path.
        menu_save: "paths.menuSave" = "Save Path…";
        /// The row menu's Duplicate Path.
        menu_duplicate: "paths.menuDuplicate" = "Duplicate Path…";
        /// The row menu's Rename Path.
        menu_rename: "paths.menuRename" = "Rename Path…";
        /// The row menu's Delete Path.
        menu_delete: "paths.menuDelete" = "Delete Path";
    }
}
