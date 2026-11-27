//! Form-dialog patterns on `Dialog` and `Field`: `FormDialog`, `ParamDialog` (a command form is `ParamDialog::command`) and the option, sizing and export dialogs.

mod words;

mod fields;
mod param;
pub(crate) mod rows;

pub use fields::{DialogFields, FieldValue};
pub use param::{CustomContext, CustomRow, Enable, NumberUnit, Param, ParamKind, encode, open_fields, set_color_field};
pub use rows::{ParamRows, ParamRowsOutput};
pub use words::ParamWords;

pub(crate) mod dialog;
pub(crate) mod form;
mod host_requests;
pub(crate) mod path_row;
pub(crate) mod preview;

pub use dialog::{
    BelowRows, DialogLayout, HostedParamDialog, ParamDialog, ParamDialogOutput, ParamDialogWords, ParamLayout, ParamPresets, ParamResult, ParamTab,
    ResetFooter, ResetPick, SharedOverlay, SharedRows, set_param_field,
};
pub use form::{FormButtons, FormCx, FormDialog, FormDialogWords, FormOutput};
pub use host_requests::{HostRequests, PreviewRequest};
pub use path_row::{PathRow, PathRowOutput, PathRowWords};
pub use preview::{
    PagerStep, PagerUnit, PaneOverlay, PanePager, PreviewOutput, PreviewPane, PreviewPaneStyle, PreviewState, PreviewView, PreviewWords, PreviewZoom,
    PreviewZoomOutput,
};

mod size_model;

pub use size_model::{LinkedSize, MAX_DOCUMENT_SIDE, SizeError, resize_from_scale, resize_from_side};

pub(crate) mod list_manager;
pub(crate) mod password;
pub(crate) mod report;

pub use list_manager::{
    ActionEnabled, HostedListManager, ListManager, ListManagerAction, ListManagerChoice, ListManagerEntry, ListManagerOutput, ListManagerWords,
};
pub use password::{
    PasswordAnswer, PasswordDialog, PasswordFields, PasswordFieldsOutput, PasswordFiles, PasswordOutput, PasswordPurpose, PasswordStrength,
    PasswordWords, Secret,
};
pub use report::{HostedReport, ReportDialog, ReportGroup, ReportNote, ReportOutput, ReportWords};

pub(crate) mod file_list;

pub use file_list::{FileList, FileListMode, FileListPicker, FileListPickerOutput, FileListWords};

pub(crate) mod sizing;

pub use sizing::{
    CanvasExtension, CanvasSize, CanvasSizeOutput, CanvasSizeResult, CanvasSizeValues, HostedCanvasSize, HostedImageSize, ImageSize, ImageSizeOutput,
    ImageSizeResult, ImageSizeValues, Resample, SizingOutput, SizingWords,
};

pub(crate) mod new_document;

// PLAN: decisions.md (files-2) asks for a `RasterMemory` re-export here; the type exists in no tree of this set, so files-w3 adds it with its definition.
pub use new_document::{
    BACKGROUND_COLOR_KEY, BackgroundContents, BitDepth, CardCx, CardSource, ColorMode, ColorProfiles, DetailsCx, DocumentCard, DocumentOptions,
    DocumentPreset, DocumentSettings, HostedNewDocument, NewDocumentDialog, NewDocumentOutput, NewDocumentPresetWords, NewDocumentResult,
    NewDocumentWords, Orientation, PIXEL_ASPECTS, PresetCardLook, PresetCategory, RasterMemory, RasterSetup, SQUARE_PIXELS,
};

pub(crate) mod about;

pub use about::{
    AboutAppTab, AboutDialog, AboutImage, AboutInfo, AboutLayout, AboutLicense, AboutLink, AboutOutput, AboutTab, AboutWords, CommitShare,
    Contributor, ContributorSort, Credits, HostedAbout, ModelCredit, NameMode,
};

mod color_choice;
pub(crate) mod guides;
pub(crate) mod properties;

pub use guides::{
    GUIDE_LAYOUT_COLOR_KEY, GuideDialogWords, GuideGroup, GuideLayoutDialog, GuideLayoutError, GuideLayoutOptions, GuideLayoutOutput,
    GuideLayoutPreset, GuideLayoutResult, GuideLayoutValues, GuideLength, GuideRect, GuideTarget, GuideUnits, HostedGuideLayoutDialog,
    HostedNewGuideDialog, NEW_GUIDE_COLOR_KEY, NewGuideDialog, NewGuideOutput, NewGuideValues,
};
pub use properties::{HostedPropertiesDialog, PropertiesDialog, PropertiesOutput, PropertiesTab, PropertiesWords, PropertyRow, TabLayout};

// canvas-navigator-a
pub(crate) mod navigator_options;

pub use navigator_options::{NAVIGATOR_PROXY_COLOR_KEY, NavigatorOptionsDialog, NavigatorOptionsDialogOutput, NavigatorOptionsWords};
// End of canvas-navigator-a.

pub(crate) mod export;

pub use export::{
    DdsCompression, DdsOptions, ExportAsDialog, ExportAsOutput, ExportAsResult, ExportFormat, ExportFormatDefaults, ExportMetadata, ExportOptions,
    ExportOptionsKind, ExportOutput, ExportPreview, ExportProfile, ExportSize, ExportWords, FormatOptions, FormatOptionsDialog, FormatOptionsOutput,
    FormatOptionsWords, HostedExportAs, HostedFormatOptions, JpegFormat, JpegOptions, JpegPreset, OutputPathOutput, OutputPathRow, OutputPathWords,
    ScaleRow, ScaleSize, SizeEstimate, TargaOptions, TiffCompression, TiffOptions, jpeg_quality_percent,
};

// Save PDF.
pub(crate) mod pdf_export;

pub use pdf_export::{
    ArtboardChoice, HostedPdfExport, PasswordSwitches, PdfArtboards, PdfExportAnswer, PdfExportDialog, PdfExportMode, PdfExportOutput,
    PdfExportResult, PdfExportWords, PdfPresetEvent, PdfSection, PresetDraft, RangeError, differences, summary_text,
};

pub(crate) mod save_for_web;
pub(crate) mod section_list;
pub(crate) mod watermark;

pub use save_for_web::{
    Animation, BrowserPreview, ColorTable, ColorTableEdit, ColorTableEvent, Dither, HostedSaveForWeb, MATTE_COLOR_KEY, OptimizeAnswer,
    OptimizeRequest, OptimizeScope, OptimizeStart, PALETTE_COLOR_KEY, SAVE_PATH_KEY, SaveForWebDialog, SaveForWebOptions, SaveForWebOutput,
    SaveForWebResult, SaveForWebWords, TableMarks, TableOrder, WEB_SPEEDS, WantedVariant, WebAntiAlias, WebDocument, WebFormat, WebMetadata, WebPane,
    WebPictures, WebPreset, WebPresetEvent, WebSlice, WebSlices, WebVariant, WebView,
};
pub use section_list::SectionList;
pub use watermark::{ROTATIONS, WATERMARK_IMAGE_KEY, Watermark, WatermarkOutput, WatermarkSettings, WatermarkStyle, WatermarkWords};
