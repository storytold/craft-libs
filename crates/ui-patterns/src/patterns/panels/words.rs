//! Built-in words of the panels family.

craft_ui::words! {
    /// The words of the panel patterns; each id is `<type>.<name>`.
    pub struct PanelWords {
        inspector_empty: "inspector.empty" = "No properties";
        palette_search: "commandPalette.search" = "Search commands";
        palette_placeholder: "commandPalette.placeholder" = "Search commands, tools and panels…";
        palette_list: "commandPalette.list" = "Commands";
        palette_empty: "commandPalette.empty" = "No matching commands";
        palette_hint: "commandPalette.hint" = "↑↓ navigate  Enter run  Esc close";
        palette_results: count "commandPalette.results" = "{n} result" | "{n} results";
        tour_back: "tour.back" = "Back";
        tour_next: "tour.next" = "Next";
        tour_done: "tour.done" = "Done";
        tour_skip: "tour.skip" = "Skip";
        tour_count: template "tour.count" = "{n} of {total}";
    }
}
