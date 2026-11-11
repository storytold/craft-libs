//! The page widgets, on craft-ui (feature `ui`). Every one reads the document through a small trait or plain values
//! the app passes, and reports what the user did; none owns pages.
//!
//! - [`view`]: [`PageView`] over the app's [`PageSource`]; overlays draw on its [`PlacedPage`]s.
//! - [`thumbnails`]: [`PageThumbnails`], the Pages panel and slide strip.
//! - [`access`]: the AccessKit description of the text on the pages.
//! - [`setup`]: [`PageSetupDialog`] and [`PageLayoutMenus`].
//! - [`margins`]: [`MarginHandles`] on the rulers.
//! - [`header_footer`](self::header_footer): [`HeaderFooterZones`].
//! - [`range`]: [`PageRangeField`] and [`PageRangeOptions`].
//! - [`text_selection`]: [`TextSelectTool`], a canvas tool over the app's [`PageTexts`].
//! - [`preview`](self::preview): [`PagePreview`].
//! - [`marks`]: [`TextFrameMarks`], hidden characters and frame extras.
//! - [`column_bar`](self::column_bar): [`ColumnBar`].
//! - [`reading`](self::reading): [`ReadingView`].
//! - [`list`]: [`PageList`], pages and artboards.
//! - [`sync_views`](self::sync_views): [`SyncedViews`].
//! - [`grid`]: [`PageGrids`], margins, columns and baseline grid.
//! - [`table`]: [`EdgePicker`], [`TableProperties`], [`TableSizePicker`], [`CellSelectionOverlay`].

pub mod access;
pub mod column_bar;
#[cfg(feature = "screen-testing")]
pub mod fixtures;
pub mod grid;
pub mod header_footer;
pub mod list;
pub mod margins;
pub mod marks;
pub mod preview;
pub mod range;
pub mod reading;
pub mod setup;
pub mod sync_views;
pub mod table;
pub mod text_selection;
pub mod thumbnails;
pub mod view;

pub use access::{AccessBuilder, AccessDescriber, AccessNode, AccessPosition, BlockRole};
pub use column_bar::{ColumnBar, ColumnBarOutput};
pub use grid::{PageGrid, PageGridColors, PageGrids, baseline_lines};
pub use header_footer::{HeaderFooterPage, HeaderFooterZones, HeaderFooterZonesOutput, Zone, ZoneAt, ZoneBands, ZoneText};
pub use list::{PageList, PageListEvent, PageListOutput, PageListRow, PageListWords};
pub use margins::{MarginChange, MarginEdge, MarginHandles, MarginHandlesOutput, MarginRules, PageEdges, clamp_margin, dragged_margin, margin_range};
pub use marks::{FrameExtras, HiddenMark, PlacedMark, TextFrameMarks};
pub use preview::{PagePreview, PagePreviewOutput, PagePreviewWords};
pub use range::{PageRangeCodec, PageRangeField, PageRangeFieldOutput, PageRangeOptions, PageRangeOptionsOutput, PageSelection};
pub use reading::{ReadingNav, ReadingNavOutput, ReadingState, ReadingView, ReadingViewOutput};
pub use setup::{PageLayoutChoice, PageLayoutMenus, PageSetupDialog};
pub use sync_views::{Lead, SyncedViews, SyncedViewsOutput, SyncedViewsState};
pub use table::{
    BorderEdges, BorderPreset, CellAlign, CellSelectionOverlay, EdgeKey, EdgePicker, RowHeight, TableChange, TableGeometry, TableProperties,
    TablePropertiesOutput, TableSizePicker, TableSizePickerOutput, TableValues, TextDirection,
};
pub use text_selection::{PageTextSource, PageTexts, TextMark, TextMarks, TextSelectState, TextSelectTool, TextSelection, TextSpan};
pub use thumbnails::{
    PageThumbnails, PageThumbnailsOutput, THUMBNAIL_ZOOM, ThumbnailEvent, ThumbnailMarks, ThumbnailSection, ThumbnailWords, moved_page,
};
pub use view::{PageBody, PageSource, PageView, PageViewOutput, PageViewState, PageViewWords, PlacedPage};
