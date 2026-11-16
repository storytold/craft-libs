//! The craft libraries in one dependency. Turn on one feature per crate and `ui` for their
//! egui side; import through `craft::<crate>`.
#![warn(missing_docs)]

pub mod egui_fork;

#[cfg(all(feature = "ui-system_fonts", target_arch = "wasm32"))]
compile_error!("`ui-system_fonts` is desktop-only: the system font index needs fontique, which has no web build");
#[cfg(feature = "aaf")]
pub use craft_aaf as aaf;
#[cfg(feature = "aep")]
pub use craft_aep as aep;
#[cfg(feature = "affinity")]
pub use craft_affinity as affinity;
#[cfg(feature = "ai-core")]
pub use craft_ai_core as ai_core;
#[cfg(feature = "assuan")]
pub use craft_assuan as assuan;
#[cfg(feature = "atomic")]
pub use craft_atomic as atomic;
#[cfg(feature = "audio")]
pub use craft_audio as audio;
#[cfg(feature = "automation")]
pub use craft_automation as automation;
pub use craft_behavior as behavior;
#[cfg(feature = "browse")]
pub use craft_browse as browse;
#[cfg(feature = "captions")]
pub use craft_captions as captions;
#[cfg(feature = "cfb")]
pub use craft_cfb as cfb;
#[cfg(feature = "cms")]
pub use craft_cms as cms;
#[cfg(feature = "codepage")]
pub use craft_codepage as codepage;
#[cfg(feature = "color")]
pub use craft_color as color;
#[cfg(feature = "crs")]
pub use craft_crs as crs;
#[cfg(feature = "crw")]
pub use craft_crw as crw;
#[cfg(feature = "csv")]
pub use craft_csv as csv;
#[cfg(feature = "dav")]
pub use craft_dav as dav;
#[cfg(feature = "designcraft-format")]
pub use craft_designcraft_format as designcraft_format;
#[cfg(feature = "dialogs")]
pub use craft_dialogs as dialogs;
#[cfg(feature = "doc")]
pub use craft_doc as doc;
#[cfg(feature = "doc-binary")]
pub use craft_doc_binary as doc_binary;
#[cfg(feature = "docpath")]
pub use craft_docpath as docpath;
#[cfg(feature = "document-bundle")]
pub use craft_document_bundle as document_bundle;
#[cfg(feature = "docx")]
pub use craft_docx as docx;
#[cfg(feature = "dpx")]
pub use craft_dpx as dpx;
#[cfg(feature = "draw")]
pub use craft_draw as draw;
#[cfg(feature = "drawingml")]
pub use craft_drawingml as drawingml;
#[cfg(feature = "dwg")]
pub use craft_dwg as dwg;
#[cfg(feature = "dxf")]
pub use craft_dxf as dxf;
#[cfg(feature = "ecproj-format")]
pub use craft_ecproj_format as ecproj_format;
#[cfg(feature = "eps")]
pub use craft_eps as eps;
#[cfg(feature = "epub")]
pub use craft_epub as epub;
#[cfg(feature = "ews")]
pub use craft_ews as ews;
#[cfg(feature = "exr")]
pub use craft_exr as exr;
#[cfg(feature = "fig")]
pub use craft_fig as fig;
#[cfg(feature = "figcraft-format")]
pub use craft_figcraft_format as figcraft_format;
#[cfg(feature = "figma-rest")]
pub use craft_figma_rest as figma_rest;
#[cfg(feature = "file-formats")]
pub use craft_file_formats as file_formats;
#[cfg(feature = "file-import")]
pub use craft_file_import as file_import;
#[cfg(feature = "flags")]
pub use craft_flags as flags;
#[cfg(feature = "flow")]
pub use craft_flow as flow;
#[cfg(feature = "font-catalog")]
pub use craft_font_catalog as font_catalog;
#[cfg(feature = "font-manager")]
pub use craft_font_manager as font_manager;
#[cfg(feature = "font-store")]
pub use craft_font_store as font_store;
#[cfg(feature = "font-subset")]
pub use craft_font_subset as font_subset;
#[cfg(feature = "gainmap")]
pub use craft_gainmap as gainmap;
#[cfg(feature = "gcode")]
pub use craft_gcode as gcode;
#[cfg(feature = "gif")]
pub use craft_gif as gif;
#[cfg(feature = "hdr")]
pub use craft_hdr as hdr;
#[cfg(feature = "heif")]
pub use craft_heif as heif;
pub use craft_host as host;
#[cfg(feature = "html")]
pub use craft_html as html;
pub use craft_i18n as i18n;
#[cfg(feature = "ical")]
pub use craft_ical as ical;
#[cfg(feature = "idml")]
pub use craft_idml as idml;
#[cfg(feature = "imaging")]
pub use craft_imaging as imaging;
#[cfg(feature = "indd")]
pub use craft_indd as indd;
#[cfg(feature = "jmap")]
pub use craft_jmap as jmap;
#[cfg(feature = "jpeg")]
pub use craft_jpeg as jpeg;
#[cfg(feature = "keymap")]
pub use craft_keymap as keymap;
#[cfg(feature = "kiwi")]
pub use craft_kiwi as kiwi;
#[cfg(feature = "kra")]
pub use craft_kra as kra;
#[cfg(feature = "latex")]
pub use craft_latex as latex;
#[cfg(feature = "layers")]
pub use craft_layers as layers;
#[cfg(feature = "lottie")]
pub use craft_lottie as lottie;
#[cfg(feature = "markdown")]
pub use craft_markdown as markdown;
#[cfg(feature = "materialx")]
pub use craft_materialx as materialx;
#[cfg(feature = "math")]
pub use craft_math as math;
#[cfg(feature = "media")]
pub use craft_media as media;
#[cfg(feature = "menu")]
pub use craft_menu as menu;
#[cfg(feature = "mesh")]
pub use craft_mesh as mesh;
#[cfg(feature = "metadata")]
pub use craft_metadata as metadata;
#[cfg(feature = "metafile")]
pub use craft_metafile as metafile;
#[cfg(feature = "midi")]
pub use craft_midi as midi;
#[cfg(feature = "mime")]
pub use craft_mime as mime;
#[cfg(feature = "odf")]
pub use craft_odf as odf;
#[cfg(feature = "ods")]
pub use craft_ods as ods;
#[cfg(feature = "offcrypto")]
pub use craft_offcrypto as offcrypto;
#[cfg(feature = "opc")]
pub use craft_opc as opc;
#[cfg(feature = "openpgp")]
pub use craft_openpgp as openpgp;
#[cfg(feature = "ora")]
pub use craft_ora as ora;
#[cfg(feature = "pages")]
pub use craft_pages as pages;
#[cfg(feature = "pcraft-bundle")]
pub use craft_pcraft_bundle as pcraft_bundle;
#[cfg(feature = "pcraft-format")]
pub use craft_pcraft_format as pcraft_format;
#[cfg(feature = "pdf")]
pub use craft_pdf as pdf;
#[cfg(feature = "pdn")]
pub use craft_pdn as pdn;
#[cfg(feature = "penpot")]
pub use craft_penpot as penpot;
#[cfg(feature = "pkcs7")]
pub use craft_pkcs7 as pkcs7;
#[cfg(feature = "platform")]
pub use craft_platform as platform;
#[cfg(feature = "plotter")]
pub use craft_plotter as plotter;
#[cfg(feature = "pptx")]
pub use craft_pptx as pptx;
#[cfg(feature = "prc")]
pub use craft_prc as prc;
#[cfg(feature = "prepress")]
pub use craft_prepress as prepress;
#[cfg(feature = "print")]
pub use craft_print as print;
#[cfg(feature = "proof")]
pub use craft_proof as proof;
#[cfg(feature = "psd")]
pub use craft_psd as psd;
#[cfg(feature = "pst")]
pub use craft_pst as pst;
#[cfg(feature = "quit")]
pub use craft_quit as quit;
#[cfg(feature = "raster-codecs")]
pub use craft_raster_codecs as raster_codecs;
#[cfg(feature = "raw")]
pub use craft_raw as raw;
#[cfg(feature = "review")]
pub use craft_review as review;
#[cfg(feature = "rtf")]
pub use craft_rtf as rtf;
#[cfg(feature = "scraft-format")]
pub use craft_scraft_format as scraft_format;
#[cfg(feature = "settings")]
pub use craft_settings as settings;
#[cfg(feature = "sf2")]
pub use craft_sf2 as sf2;
#[cfg(feature = "sheet")]
pub use craft_sheet as sheet;
#[cfg(feature = "silhouette-studio")]
pub use craft_silhouette_studio as silhouette_studio;
#[cfg(feature = "sketch")]
pub use craft_sketch as sketch;
#[cfg(feature = "stencilcraft-document")]
pub use craft_stencilcraft_document as stencilcraft_document;
#[cfg(feature = "stencilcraft-format")]
pub use craft_stencilcraft_format as stencilcraft_format;
#[cfg(feature = "stream-codecs")]
pub use craft_stream_codecs as stream_codecs;
#[cfg(feature = "svg")]
pub use craft_svg as svg;
#[cfg(feature = "table")]
pub use craft_table as table;
#[cfg(feature = "text")]
pub use craft_text as text;
#[cfg(feature = "tga")]
pub use craft_tga as tga;
pub use craft_theme as theme;
#[cfg(feature = "theme-store")]
pub use craft_theme_store as theme_store;
#[cfg(feature = "tiff")]
pub use craft_tiff as tiff;
#[cfg(feature = "time")]
pub use craft_time as time;
#[cfg(feature = "timeline")]
pub use craft_timeline as timeline;
#[cfg(feature = "ui")]
pub use craft_ui as ui;
#[cfg(feature = "update")]
pub use craft_update as update;
#[cfg(feature = "vcard")]
pub use craft_vcard as vcard;
#[cfg(feature = "vectorcraft-format")]
pub use craft_vectorcraft_format as vectorcraft_format;
#[cfg(feature = "vectordrawable")]
pub use craft_vectordrawable as vectordrawable;
#[cfg(feature = "video")]
pub use craft_video as video;
#[cfg(feature = "web-api")]
pub use craft_web_api as web_api;
#[cfg(feature = "web-host")]
pub use craft_web_host as web_host;
#[cfg(feature = "webp")]
pub use craft_webp as webp;
#[cfg(feature = "window")]
pub use craft_window as window;
#[cfg(feature = "wordmodel")]
pub use craft_wordmodel as wordmodel;
#[cfg(feature = "xcf")]
pub use craft_xcf as xcf;
#[cfg(feature = "xls")]
pub use craft_xls as xls;
#[cfg(feature = "xlsx")]
pub use craft_xlsx as xlsx;
#[cfg(feature = "xml")]
pub use craft_xml as xml;
#[cfg(feature = "y4m")]
pub use craft_y4m as y4m;
#[cfg(feature = "zip")]
pub use craft_zip as zip;
#[cfg(feature = "ui")]
pub use egui;

/// The types an app imports first: the host handle, a dialog stack, the dialog outcome and the theme
/// tokens.
pub mod prelude {
    pub use craft_host::{AppInfo, Host, Section};
    pub use craft_theme::Tokens;
    #[cfg(feature = "ui")]
    pub use craft_ui::{contract::Outcome, dialog::DialogStack};
}

#[cfg(test)]
extern crate self as craft;

#[cfg(test)]
#[path = "../tests/main.rs"]
mod integration;
