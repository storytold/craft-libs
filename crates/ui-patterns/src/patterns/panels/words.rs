//! Built-in words of the panels family.

craft_ui::words! {
    /// The words of the panel patterns; each id is `<type>.<name>`.
    pub struct PanelWords {
        inspector_empty: "inspector.empty" = "No properties";
        history_list: "history.list" = "History";
        history_empty: "history.empty" = "No history";
        history_delete_tip: "history.deleteTip" = "Delete Current State";
        history_new_snapshot: "history.newSnapshot" = "Create New Snapshot";
        history_new_document: "history.newDocument" = "Create New Document from Current State";
        history_rename: "history.rename" = "Rename…";
        history_delete: "history.delete" = "Delete";
        history_source: "history.source" = "History Brush Source";
        history_source_cell: "history.sourceCell" = "Set {row} as History Brush Source";
        palette_search: "commandPalette.search" = "Search commands";
        palette_placeholder: "commandPalette.placeholder" = "Search commands, tools and panels…";
        palette_list: "commandPalette.list" = "Commands";
        palette_empty: "commandPalette.empty" = "No matching commands";
        palette_count: template "commandPalette.count" = "{shown} of {n} results. Keep typing to narrow them.";
        palette_results: count "commandPalette.results" = "{n} result" | "{n} results";
        tour_back: "tour.back" = "Back";
        tour_next: "tour.next" = "Next";
        tour_done: "tour.done" = "Done";
        tour_skip: "tour.skip" = "Skip Tour";
        tour_count: template "tour.count" = "{n} of {total}";
    }
}
