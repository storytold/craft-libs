//! Form-dialog patterns on `Dialog` and `Field`: `FormDialog`, `ParamDialog` (a command form is `ParamDialog::command`) and the option, sizing and export dialogs.

mod words;

mod fields;
mod param;
mod rows;

pub use fields::{DialogFields, FieldValue};
pub use param::{CustomContext, CustomRow, NumberUnit, Param, ParamKind, encode, open_fields, set_color_field};
pub use rows::{ParamRows, ParamRowsOutput};
pub use words::ParamWords;

mod dialog;
mod form;
mod host_requests;
mod preview;

pub use dialog::{
    BelowRows, FooterExtra, HostedParamDialog, ParamDialog, ParamDialogOutput, ParamDialogWords, ParamLayout, ParamResult, ResetFooter, ResetPick,
};
pub use form::{FormCx, FormDialog, FormOutput};
pub use host_requests::{HostRequests, PreviewRequest};
pub use preview::{
    PaneOverlay, PreviewOutput, PreviewPane, PreviewPaneStyle, PreviewState, PreviewView, PreviewWords, PreviewZoom, PreviewZoomOutput,
};

mod size_model;

pub use size_model::{LinkedSize, MAX_DOCUMENT_SIDE, SizeError, resize_from_scale, resize_from_side};

mod list_manager;
mod password;
mod report;

pub use list_manager::{ActionEnabled, ListManager, ListManagerAction, ListManagerChoice, ListManagerEntry, ListManagerOutput, ListManagerWords};
pub use password::{PasswordAnswer, PasswordDialog, PasswordOutput, PasswordPurpose, PasswordStrength, PasswordWords, Secret};
pub use report::{ReportDialog, ReportNote, ReportOutput, ReportWords};

mod sizing;

pub use sizing::{
    CanvasExtension, CanvasSize, CanvasSizeOutput, CanvasSizeResult, CanvasSizeValues, HostedImageSize, ImageSize, ImageSizeOutput, ImageSizeResult,
    ImageSizeValues, Resample, SizingOutput, SizingWords,
};

mod new_document;

pub use new_document::{
    BACKGROUND_COLOR_KEY, BackgroundContents, BitDepth, ColorMode, ColorProfiles, DocumentOptions, DocumentPreset, HostedNewDocument,
    NewDocumentDialog, NewDocumentOutput, NewDocumentPresetWords, NewDocumentResult, NewDocumentWords, Orientation, PresetCategory,
};

mod about;

pub use about::{
    AboutDialog, AboutInfo, AboutLink, AboutOutput, AboutTab, AboutWords, CommitShare, Contributor, ContributorSort, Credits, ModelCredit, NameMode,
};

mod color_choice;
mod guides;
mod properties;

pub use guides::{
    GUIDE_LAYOUT_COLOR_KEY, GuideDialogWords, GuideGroup, GuideLayoutDialog, GuideLayoutError, GuideLayoutOptions, GuideLayoutOutput,
    GuideLayoutPreset, GuideLayoutResult, GuideLayoutValues, GuideLength, GuideRect, GuideTarget, GuideUnits, HostedGuideLayoutDialog,
    HostedNewGuideDialog, NEW_GUIDE_COLOR_KEY, NewGuideDialog, NewGuideOutput, NewGuideValues,
};
pub use properties::{HostedPropertiesDialog, PropertiesDialog, PropertiesOutput, PropertiesTab, PropertyRow};

// canvas-navigator-a
mod navigator_options;

pub use navigator_options::{NAVIGATOR_PROXY_COLOR_KEY, NavigatorOptionsDialog, NavigatorOptionsDialogOutput, NavigatorOptionsWords};
// End of canvas-navigator-a.

mod export;

pub use export::{
    DdsCompression, DdsOptions, ExportAsDialog, ExportAsOutput, ExportAsResult, ExportFormat, ExportFormatDefaults, ExportMetadata, ExportOptions,
    ExportOptionsKind, ExportPreview, ExportProfile, ExportSize, ExportWords, FormatOptions, FormatOptionsDialog, FormatOptionsOutput,
    FormatOptionsWords, JpegFormat, JpegOptions, JpegPreset, OutputPathOutput, OutputPathRow, OutputPathWords, SizeEstimate, TargaOptions,
    TiffOptions, jpeg_quality_percent,
};

// Save PDF.
mod pdf_export;

pub use pdf_export::{PdfArtboards, PdfExportDialog, PdfExportOutput, PdfExportResult, PdfExportWords, PdfPresetEvent, PdfSection, RangeError};

mod save_for_web;
mod watermark;

pub use save_for_web::{
    Animation, ColorTable, Dither, HostedSaveForWeb, MATTE_COLOR_KEY, SAVE_PATH_KEY, SaveForWebDialog, SaveForWebOptions, SaveForWebOutput,
    SaveForWebResult, SaveForWebWords, WantedVariant, WebFormat, WebMetadata, WebPane, WebPictures, WebPreset, WebVariant, WebView,
};
pub use watermark::{ROTATIONS, WATERMARK_IMAGE_KEY, Watermark, WatermarkOutput, WatermarkSettings, WatermarkStyle, WatermarkWords};
