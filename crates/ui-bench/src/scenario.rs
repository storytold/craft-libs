//! The scenario list of spec 22.7.

/// One scenario: a name that matches the spec and whether it must run no discard pass.
#[derive(Clone, Copy, Debug)]
pub struct Scenario {
    /// The name in the report and in each library's scenario table.
    pub name: &'static str,
    /// What the spec's text calls it.
    pub description: &'static str,
    /// One of the four scenarios that assert steady animation, scrolling or changing measured
    /// content run no discard pass.
    pub no_discard: bool,
}

/// Every scenario, in report order.
pub const SCENARIOS: &[Scenario] = &[
    Scenario { name: "inspector-500-fields", description: "an inspector of 500 Field rows", no_discard: false },
    Scenario { name: "virtual-list-10000", description: "a 10,000-item virtual list", no_discard: false },
    Scenario { name: "sheet-50x200", description: "a sheet of 50 x 200 visible cells as row parts", no_discard: false },
    Scenario { name: "canvas-50-controls", description: "a canvas with 50 canvas controls", no_discard: false },
    Scenario { name: "menu-40-rows", description: "an open menu with 40 rows", no_discard: false },
    Scenario { name: "table-1000x5", description: "a table of 1,000 rows and 5 columns", no_discard: false },
    Scenario { name: "tile-grid-500", description: "a tile grid of 500 tiles", no_discard: false },
    Scenario { name: "text-field-100kb", description: "a 100 KB multi-line text field, bound", no_discard: false },
    Scenario { name: "list-10000-selected", description: "a list with 10,000 selected keys", no_discard: false },
    Scenario { name: "collapse-in-content-sized-panel", description: "a collapse animating inside a content-sized panel", no_discard: true },
    Scenario { name: "auto-height-list-scroll-10000", description: "scrolling a 10,000-row auto-height list", no_discard: true },
    Scenario { name: "second-viewport", description: "a second viewport open", no_discard: true },
    Scenario { name: "dialog-20-fields", description: "a modal dialog open over 20 fields", no_discard: false },
    Scenario { name: "tab-strip-60", description: "a tab strip of 60 document tabs with overflow, one selected", no_discard: false },
    Scenario {
        name: "canvas-view-4k",
        description: "a rotated canvas view over a 4096 x 4096 raster with rulers, 40 guides and the Hand tool, panning each frame",
        no_discard: true,
    },
    Scenario { name: "param-dialog-export", description: "the Export As dialog open with its format form and a ready preview", no_discard: false },
    Scenario {
        name: "readout-and-content-column",
        description: "an end-aligned readout and a Content grid column changing every frame",
        no_discard: true,
    },
];
