# craft-pages

Pages for the Craft document apps (PdfCraft, WordCraft, DeckCraft, DesignCraft, VectorCraft,
GridCraft). The document, its layout and its rendering stay in each app; this crate has the
geometry and the widgets around pages.

| Module | Feature | Provides |
| --- | --- | --- |
| `layout` | | Page rectangles (single, two-up with a cover, as many as fit; continuous or one row), fit width/page/height, Hide White Space, view defaults |
| `setup` | | Paper sizes, orientation, margins and Word's presets, the page setup a dialog edits |
| `range` | | "1-3, 5" page ranges and Acrobat's From/To/Subset ranges |
| `numbering` | | Page labels in a number style with a section start and prefix |
| `ui` | `ui` | On craft-ui: `PageView`, `PageThumbnails`, `PagesAccess`, `PageSetupDialog` and `PageLayoutMenus`, `MarginHandles`, `HeaderFooterZones`, `PageRangeField`, `TextSelectionLayer`, `PagePreview`, `TextFrameMarks`, `ColumnBar`, `ReadingView`, `PageList`, `SyncedViews`, `PageGrids`, and `ui::table` (`EdgePicker`, `TableProperties`, `TableSizePicker`, `CellSelectionOverlay`) |
| | `serde` | Serde impls of the core types |

The widgets read the document through small traits (`PageSource`, `PageTextSource`,
`AccessDescriber`) or plain values, and report what the user did as actions or edit gestures.

Page numbers use `craft-text`'s list number styles. The spreadsheet grid is `craft-table`'s.


Stories are `#[story]` functions in `src/ui/*/stories.rs`; they register in `craft_ui::testing::stories()` and the `craft-ui-gallery` viewer lists them by the type they show.
