# craft-table

Spreadsheet widgets for Craft apps, from GridCraft (Excel), with LightCraft's condition builder. A UI-only family: everything is in `craft_table::ui` behind the `ui` feature.

- `grid`: `CellAt`, `CellRange`, `CellSelection`, `LineIndex` (sizes with prefix sums), `SheetGeometry` (zoom, headers, frozen panes, right-to-left) and the `CellGrid` widget over a `GridModel` the app implements, with Excel's keys.
- `conditions`: `ConditionBuilder`, rows of field, operator and value with nested groups.
- `outline`: `OutlineBand`, the group brackets and level buttons of grouped rows and columns.
- `fields`: `FieldListPane`, the PivotTable Fields pane.
- `chart`: `ChartPrimitive` and `paint_chart`, a chart drawn from a primitive list.
- `slicer`: `Slicer`, toggle tiles that filter a table.
- `refpick`: `ReferencePicker`, point mode for a formula field.
- `stats`: `SelectionStats` and the status bar's `SelectionStatsReadout`.
- `error_tag`: `ErrorTag`, the warning tag beside a cell with its fixes.
- `filter`: `FilterButton`, the filter arrow in a header cell.

Edits go out as events; the app applies them to its workbook.

Stories are `#[story]` functions in `src/ui/*/stories.rs`; they register in `craft_ui::testing::stories()` and the `craft-ui-gallery` viewer lists them by the type they show.
