//! The data of New Document: the document settings, the built-in presets and categories, the lists the dialog
//! remembers and the rules on them.

use crate::patterns::form_dialogs::{LinkedSize, MAX_DOCUMENT_SIDE};
use crate::runtime::env::{Label, UnitKind, Units};
use craft_behavior::PixelSize;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

use super::{DocumentSettings, NewDocumentWords};

/// The most documents Recent keeps.
pub(super) const MAX_RECENT: usize = 20;
/// The most presets a user can save.
pub(super) const MAX_SAVED: usize = 256;
/// The most characters of a document or preset name.
pub(super) const MAX_NAME: usize = 255;
/// The id of the Recent card that stands for the application's own default size.
pub(super) const DEFAULT_ID: &str = "default";
/// The resolution a document without one opens at, in pixels per inch.
const DEFAULT_RESOLUTION: f64 = 300.0;
/// The size a document without one opens at: PhotoCraft's default, 7 x 5 in at 300 ppi.
const DEFAULT_SIZE: PixelSize = PixelSize::new(2100, 1500);
/// The least and greatest resolution, in pixels per inch.
const RESOLUTION_LIMIT: f64 = 30_000.0;

/// The units the Width and Height lists offer (Photoshop's New Document has no percent).
pub(super) const SIZE_UNITS: [UnitKind; 6] =
    [UnitKind::Pixels, UnitKind::Inches, UnitKind::Centimetres, UnitKind::Millimetres, UnitKind::Points, UnitKind::Picas];
/// The units the Resolution list offers.
pub(super) const RESOLUTION_UNITS: [UnitKind; 2] = [UnitKind::PixelsPerInch, UnitKind::PixelsPerCentimetre];

/// The document's colour mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ColorMode {
    /// One bit per pixel, black or white.
    Bitmap,
    /// One grey channel.
    Grayscale,
    /// Red, green and blue.
    #[default]
    Rgb,
    /// Cyan, magenta, yellow and black.
    Cmyk,
    /// Lightness and the two colour-opponent channels.
    Lab,
}

/// Bits per channel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BitDepth {
    /// 1 bit: a Bitmap document's only depth.
    One,
    /// 8 bits per channel.
    #[default]
    Eight,
    /// 16 bits per channel.
    Sixteen,
    /// 32-bit floating point.
    ThirtyTwo,
}

impl ColorMode {
    /// Every mode in the Color Mode list's order (Photoshop's).
    pub const ALL: [ColorMode; 5] = [ColorMode::Bitmap, ColorMode::Grayscale, ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab];

    /// The depths the mode takes, the one a mode switch lands on first: Bitmap is 1 bit only, every other mode 8,
    /// 16 or 32.
    pub fn depths(self) -> &'static [BitDepth] {
        match self {
            ColorMode::Bitmap => &[BitDepth::One],
            _ => &[BitDepth::Eight, BitDepth::Sixteen, BitDepth::ThirtyTwo],
        }
    }

    /// Whether the background may be transparent: a Bitmap pixel is black or white.
    pub fn allows_transparent(self) -> bool {
        self != ColorMode::Bitmap
    }
}

/// The pixel aspect ratios New Document lists before the app's own, as `(id, ratio)` in Photoshop's order. The ids
/// are the ones PhotoCraft's View › Pixel Aspect Ratio uses; `square` is the default.
pub const PIXEL_ASPECTS: [(&str, f64); 8] = [
    (SQUARE_PIXELS, 1.0),
    ("d1DvNtsc", 0.91),
    ("d1DvPal", 1.09),
    ("d1DvNtscWidescreen", 1.21),
    ("hdv1080", 1.33),
    ("d1DvPalWidescreen", 1.46),
    ("anamorphic2To1", 2.0),
    ("dvcproHd1080", 1.5),
];

/// The id of square pixels, the default pixel aspect ratio.
pub const SQUARE_PIXELS: &str = "square";

fn square_pixels() -> String {
    SQUARE_PIXELS.to_owned()
}

/// What fills the new document's background layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BackgroundContents {
    /// White.
    #[default]
    White,
    /// Black.
    Black,
    /// The application's current background colour (`NewDocumentDialog::background_color`).
    BackgroundColor,
    /// No fill.
    Transparent,
    /// A colour the user picked with Custom, sRGB, 0 to 1 per channel.
    Custom([f32; 3]),
}

/// Which way the page stands. The orientation control swaps width and height, and the built-in cards turn with it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Orientation {
    /// Taller than wide, or square.
    #[default]
    Portrait,
    /// Wider than tall.
    Landscape,
}

impl Orientation {
    /// Landscape when wider than tall; a square is portrait.
    pub fn of(size: PixelSize) -> Self {
        if size.width > size.height { Orientation::Landscape } else { Orientation::Portrait }
    }
}

/// Everything New Document creates a document from: the dialog's result, a saved preset, a recent card and the
/// remembered last values are all this.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct DocumentOptions {
    /// The document name ("Untitled-1"); a preset's own name when saved as one.
    pub name: String,
    /// Width and height in pixels.
    pub size: PixelSize,
    /// The resolution in pixels per inch.
    pub pixels_per_inch: f64,
    /// The unit the size fields show; the size itself is always in pixels.
    pub unit: UnitKind,
    /// The unit the resolution field shows; the resolution itself is stored in pixels per inch.
    pub resolution_unit: UnitKind,
    /// The colour mode.
    pub color_mode: ColorMode,
    /// Bits per channel.
    pub depth: BitDepth,
    /// The colour profile's name; `None` is the working profile of the mode.
    pub profile: Option<String>,
    /// What fills the background layer.
    pub background: BackgroundContents,
    /// Artboards: the document starts with one artboard of this size.
    pub artboard: bool,
    /// The pixel aspect ratio's id: one of [`PIXEL_ASPECTS`] or an id the app listed with
    /// `NewDocumentDialog::custom_pixel_aspects`.
    #[serde(default = "square_pixels")]
    pub pixel_aspect: String,
}

impl Default for DocumentOptions {
    /// 2100 x 1500 px at 300 ppi (7 x 5 in), RGB, 8 bit, white.
    fn default() -> Self {
        Self {
            name: String::new(),
            size: DEFAULT_SIZE,
            pixels_per_inch: DEFAULT_RESOLUTION,
            unit: UnitKind::Pixels,
            resolution_unit: UnitKind::PixelsPerInch,
            color_mode: ColorMode::Rgb,
            depth: BitDepth::Eight,
            profile: None,
            background: BackgroundContents::White,
            artboard: false,
            pixel_aspect: square_pixels(),
        }
    }
}

impl DocumentOptions {
    /// Whether two settings make the same document (the name and the display units apart).
    pub fn same_document(&self, other: &DocumentOptions) -> bool {
        self.size == other.size
            && self.pixels_per_inch == other.pixels_per_inch
            && self.color_mode == other.color_mode
            && self.depth == other.depth
            && self.background == other.background
            && self.profile == other.profile
    }

    /// The settings with the depth and background the colour mode takes: a depth the mode lacks becomes the mode's
    /// first (switching to Bitmap gives 1 bit, leaving it 8 bit), and a transparent background under Bitmap becomes
    /// white.
    #[must_use]
    pub fn fitted(mut self) -> Self {
        let depths = self.color_mode.depths();
        if !depths.contains(&self.depth) {
            self.depth = depths[0];
        }
        if !self.color_mode.allows_transparent() && self.background == BackgroundContents::Transparent {
            self.background = BackgroundContents::White;
        }
        self
    }

    /// Whether the settings fit what the dialog can show: a damaged remembered value does not.
    pub fn fits(&self) -> bool {
        let side = |n: u32| (1..=MAX_DOCUMENT_SIDE as u32).contains(&n);
        side(self.size.width)
            && side(self.size.height)
            && self.pixels_per_inch.is_finite()
            && self.pixels_per_inch > 0.0
            && self.pixels_per_inch <= RESOLUTION_LIMIT
            && SIZE_UNITS.contains(&self.unit)
            && RESOLUTION_UNITS.contains(&self.resolution_unit)
    }

    /// The fields that show these settings: the size, resolution and units, with `points_per_inch` points in an inch.
    pub(super) fn linked(&self, points_per_inch: f64) -> LinkedSize {
        let mut linked = LinkedSize::new(self.size).opening_resolution(self.pixels_per_inch).points_per_inch(points_per_inch).chain(false);
        linked.set_unit(self.unit);
        linked.set_resolution_unit(self.resolution_unit);
        linked
    }

    /// Whether `linked` shows these settings' size, resolution and units.
    pub(super) fn shown_by(&self, linked: &LinkedSize) -> bool {
        linked.pixels() == self.size
            && linked.pixels_per_inch() == self.pixels_per_inch
            && linked.unit() == self.unit
            && linked.resolution_unit() == self.resolution_unit
    }

    /// Takes the size, resolution and units from `linked`.
    pub(super) fn take_linked(&mut self, linked: &LinkedSize) {
        self.size = linked.pixels();
        self.pixels_per_inch = linked.pixels_per_inch();
        self.unit = linked.unit();
        self.resolution_unit = linked.resolution_unit();
    }
}

/// One built-in card: a name and a page size.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DocumentPreset {
    /// Stable id. The name shows through the word set under it for the built-in presets.
    pub id: &'static str,
    /// The English name, shown when no word set names the id.
    pub name: &'static str,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Resolution in pixels per inch.
    pub pixels_per_inch: f64,
    /// Picking the card turns Artboards on (Photoshop's Web and Mobile presets).
    pub artboard: bool,
    /// A print preset sized in pixels (a pixel grid): it shows and applies its pixels, at the app's print resolution.
    pub pixel_sized: bool,
}

impl DocumentPreset {
    /// The card `id`, named `name` in English, for a page of `width` x `height` pixels at `pixels_per_inch`.
    pub const fn new(id: &'static str, name: &'static str, width: u32, height: u32, pixels_per_inch: f64) -> Self {
        Self { id, name, width, height, pixels_per_inch, artboard: false, pixel_sized: false }
    }

    /// The card turns Artboards on when picked.
    #[must_use]
    pub const fn artboard(mut self) -> Self {
        self.artboard = true;
        self
    }

    /// The card keeps its pixels: it shows and applies them at the app's print resolution instead of keeping a page
    /// size in inches.
    #[must_use]
    pub const fn pixel_sized(mut self) -> Self {
        self.pixel_sized = true;
        self
    }

    /// Whether the preset is for print (300 ppi and up): it takes the app's print resolution; screen presets take the
    /// screen resolution.
    pub fn is_print(&self) -> bool {
        self.pixels_per_inch >= 300.0
    }

    /// Whether the card and fields show the preset in inches: a print preset that is not [`pixel_sized`](Self::pixel_sized).
    pub fn in_inches(&self) -> bool {
        self.is_print() && !self.pixel_sized
    }

    /// The preset turned to `orientation`; a square preset stays as it is.
    #[must_use]
    pub fn turned(&self, orientation: Orientation) -> DocumentPreset {
        let portrait = orientation == Orientation::Portrait;
        if self.width == self.height || (self.height > self.width) == portrait {
            return self.clone();
        }
        DocumentPreset { width: self.height, height: self.width, ..self.clone() }
    }

    /// The preset as settings: the unit (inches for print, pixels for screen and pixel-sized presets) and the
    /// resolution moved to the app's print or screen resolution, keeping a print preset's physical size and any other
    /// preset's pixels.
    pub fn options(&self, print_ppi: f64, screen_ppi: f64) -> DocumentOptions {
        let inches = self.in_inches();
        let wanted = if self.is_print() { print_ppi } else { screen_ppi };
        let (width, height, pixels_per_inch) = if !(wanted.is_finite() && wanted > 0.0) {
            (self.width, self.height, self.pixels_per_inch)
        } else if inches {
            let scale = wanted / self.pixels_per_inch;
            let pixels = |side: u32| (f64::from(side) * scale).round().clamp(1.0, MAX_DOCUMENT_SIDE) as u32;
            (pixels(self.width), pixels(self.height), wanted)
        } else {
            (self.width, self.height, wanted)
        };
        DocumentOptions {
            size: PixelSize::new(width, height),
            pixels_per_inch,
            unit: if inches { UnitKind::Inches } else { UnitKind::Pixels },
            ..DocumentOptions::default()
        }
    }
}

/// A category tab and its cards.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PresetCategory<P = DocumentPreset> {
    /// Stable id: `"photo"`.
    pub id: Cow<'static, str>,
    /// The title, shown when no word names the id.
    pub title: Label,
    /// The category's cards in display order.
    pub presets: Vec<P>,
}

impl<P> PresetCategory<P> {
    /// The category `id` titled `title`, with its `presets`.
    pub fn new(id: impl Into<Cow<'static, str>>, title: impl Into<Label>, presets: Vec<P>) -> Self {
        Self { id: id.into(), title: title.into(), presets }
    }
}

impl PresetCategory<DocumentPreset> {
    /// PhotoCraft's categories in tab order: Photo, Print, Art & Illustration, Web, Mobile, Film & Video, with their
    /// presets. Recent and Saved are not in this list: the dialog adds them first and last.
    pub fn standard() -> Vec<PresetCategory> {
        vec![
            PresetCategory::new(
                "photo",
                "Photo",
                vec![
                    DocumentPreset::new(DEFAULT_ID, "Default Size", 2100, 1500, 300.0),
                    DocumentPreset::new("landscape-3x2", "Landscape, 3 x 2", 900, 600, 300.0),
                    DocumentPreset::new("landscape-6x4", "Landscape, 6 x 4", 1800, 1200, 300.0),
                    DocumentPreset::new("landscape-7x5", "Landscape, 7 x 5", 2100, 1500, 300.0),
                    DocumentPreset::new("landscape-10x8", "Landscape, 10 x 8", 3000, 2400, 300.0),
                    DocumentPreset::new("portrait-2x3", "Portrait, 2 x 3", 600, 900, 300.0),
                    DocumentPreset::new("portrait-4x6", "Portrait, 4 x 6", 1200, 1800, 300.0),
                    DocumentPreset::new("portrait-5x7", "Portrait, 5 x 7", 1500, 2100, 300.0),
                    DocumentPreset::new("portrait-8x10", "Portrait, 8 x 10", 2400, 3000, 300.0),
                ],
            ),
            PresetCategory::new(
                "print",
                "Print",
                vec![
                    DocumentPreset::new("letter", "Letter", 2550, 3300, 300.0),
                    DocumentPreset::new("legal", "Legal", 2550, 4200, 300.0),
                    DocumentPreset::new("tabloid", "Tabloid", 3300, 5100, 300.0),
                    DocumentPreset::new("a4", "A4", 2480, 3508, 300.0),
                    DocumentPreset::new("a6", "A6", 1240, 1748, 300.0),
                    DocumentPreset::new("a5", "A5", 1748, 2480, 300.0),
                    DocumentPreset::new("a3", "A3", 3508, 4961, 300.0),
                    DocumentPreset::new("b5", "B5", 2079, 2953, 300.0),
                    DocumentPreset::new("b4", "B4", 2953, 4169, 300.0),
                    DocumentPreset::new("b3", "B3", 4169, 5906, 300.0),
                    DocumentPreset::new("c4", "C4", 2705, 3827, 300.0),
                    DocumentPreset::new("c5", "C5", 1913, 2705, 300.0),
                    DocumentPreset::new("c6", "C6", 1346, 1913, 300.0),
                    DocumentPreset::new("dl", "DL", 1299, 2598, 300.0),
                ],
            ),
            PresetCategory::new(
                "art",
                "Art & Illustration",
                vec![
                    DocumentPreset::new("grid-1000", "1000 pixel grid", 1000, 1000, 300.0).pixel_sized(),
                    DocumentPreset::new("grid-2000", "2000 pixel grid", 2000, 2000, 300.0).pixel_sized(),
                    DocumentPreset::new("poster", "Poster", 5400, 7200, 300.0),
                    DocumentPreset::new("postcard", "Postcard", 1200, 1800, 300.0),
                    DocumentPreset::new("video-1080p", "1080p", 1920, 1080, 72.0),
                    DocumentPreset::new("video-720p", "720p", 1280, 720, 72.0),
                ],
            ),
            PresetCategory::new(
                "web",
                "Web",
                vec![
                    DocumentPreset::new("web-most-common", "Web Most Common", 1366, 768, 72.0).artboard(),
                    DocumentPreset::new("web-minimum", "Web Minimum", 1024, 768, 72.0).artboard(),
                    DocumentPreset::new("web-large", "Web Large", 1920, 1080, 72.0).artboard(),
                    DocumentPreset::new("macbook-pro-16", "MacBook Pro 16\"", 3456, 2234, 72.0).artboard(),
                    DocumentPreset::new("imac-24", "iMac 24\"", 4480, 2520, 72.0).artboard(),
                ],
            ),
            PresetCategory::new(
                "mobile",
                "Mobile",
                vec![
                    DocumentPreset::new("iphone-16", "iPhone 16", 1179, 2556, 72.0).artboard(),
                    DocumentPreset::new("iphone-16-pro-max", "iPhone 16 Pro Max", 1320, 2868, 72.0).artboard(),
                    DocumentPreset::new("ipad-pro-13", "iPad Pro 13\"", 2064, 2752, 72.0).artboard(),
                    DocumentPreset::new("android-1080p", "Android 1080p", 1080, 1920, 72.0).artboard(),
                    DocumentPreset::new("apple-watch-45", "Apple Watch 45mm", 396, 484, 72.0).artboard(),
                ],
            ),
            PresetCategory::new(
                "film",
                "Film & Video",
                vec![
                    DocumentPreset::new("hdtv-1080p", "HDTV 1080p", 1920, 1080, 72.0),
                    DocumentPreset::new("hdtv-720p", "HDTV 720p", 1280, 720, 72.0),
                    DocumentPreset::new("uhd-4k", "UHD 4K", 3840, 2160, 72.0),
                    DocumentPreset::new("dci-4k", "DCI 4K", 4096, 2160, 72.0),
                    DocumentPreset::new("uhd-8k", "UHD 8K", 7680, 4320, 72.0),
                ],
            ),
        ]
    }

    /// The cards Recent shows after the recent documents when the app gives none: "Default {app} Size" and HDTV 1080p.
    pub fn default_recent() -> Vec<DocumentPreset> {
        vec![DocumentPreset::new(DEFAULT_ID, "Default Size", 2100, 1500, 300.0), DocumentPreset::new("hdtv-1080p", "HDTV 1080p", 1920, 1080, 72.0)]
    }
}

/// The colour profiles one colour mode offers.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ColorProfiles {
    /// The mode the profiles are for.
    pub mode: ColorMode,
    /// The working profile's name, listed first as "Working: name".
    pub working: String,
    /// The other profiles.
    pub others: Vec<String>,
}

impl ColorProfiles {
    /// The profiles of `mode`: the `working` one and the `others`.
    pub fn new(mode: ColorMode, working: impl Into<String>, others: Vec<String>) -> Self {
        Self { mode, working: working.into(), others }
    }
}

/// The recent list after creating `settings`: equal settings move to the front instead of repeating, and the list
/// keeps at most `max` entries (20 in the dialog).
pub(crate) fn push_recent<S: DocumentSettings>(recent: &mut Vec<S>, settings: S, max: usize) {
    recent.retain(|held| !held.same_document(&settings));
    recent.insert(0, settings);
    recent.truncate(max);
}

/// The size line of a card or a saved preset: "1800 x 1200 px @ 300 ppi", or inches for a print preset
/// ("6 x 4 in @ 300 ppi"), numbers in the user's format with at most two decimals.
pub(crate) fn size_line(options: &DocumentOptions, words: &NewDocumentWords, units: &Units) -> String {
    let unit = if SIZE_UNITS.contains(&options.unit) { options.unit } else { UnitKind::Pixels };
    let mut linked = options.linked(units.points_per_inch());
    linked.set_unit(unit);
    let (width, height) = (units.number_ungrouped(linked.width(), 0..=2), units.number_ungrouped(linked.height(), 0..=2));
    let resolution = units.show(UnitKind::PixelsPerInch, options.pixels_per_inch);
    words.size_line.fill(&[("width", &width), ("height", &height), ("unit", &units.symbol(unit)), ("resolution", &resolution)])
}

craft_ui::words! {
    /// The names of the built-in presets, in the order of `PRESET_IDS`.
    pub struct NewDocumentPresetWords {
        names: array [
            "newDocumentPreset.landscape3x2" = "Landscape, 3 x 2",
            "newDocumentPreset.landscape6x4" = "Landscape, 6 x 4",
            "newDocumentPreset.landscape7x5" = "Landscape, 7 x 5",
            "newDocumentPreset.landscape10x8" = "Landscape, 10 x 8",
            "newDocumentPreset.portrait2x3" = "Portrait, 2 x 3",
            "newDocumentPreset.portrait4x6" = "Portrait, 4 x 6",
            "newDocumentPreset.portrait5x7" = "Portrait, 5 x 7",
            "newDocumentPreset.portrait8x10" = "Portrait, 8 x 10",
            "newDocumentPreset.letter" = "Letter",
            "newDocumentPreset.legal" = "Legal",
            "newDocumentPreset.tabloid" = "Tabloid",
            "newDocumentPreset.a4" = "A4",
            "newDocumentPreset.a6" = "A6",
            "newDocumentPreset.a5" = "A5",
            "newDocumentPreset.a3" = "A3",
            "newDocumentPreset.b5" = "B5",
            "newDocumentPreset.b4" = "B4",
            "newDocumentPreset.b3" = "B3",
            "newDocumentPreset.c4" = "C4",
            "newDocumentPreset.c5" = "C5",
            "newDocumentPreset.c6" = "C6",
            "newDocumentPreset.dl" = "DL",
            "newDocumentPreset.grid1000" = "1000 pixel grid",
            "newDocumentPreset.grid2000" = "2000 pixel grid",
            "newDocumentPreset.poster" = "Poster",
            "newDocumentPreset.postcard" = "Postcard",
            "newDocumentPreset.video1080p" = "1080p",
            "newDocumentPreset.video720p" = "720p",
            "newDocumentPreset.webMostCommon" = "Web Most Common",
            "newDocumentPreset.webMinimum" = "Web Minimum",
            "newDocumentPreset.webLarge" = "Web Large",
            "newDocumentPreset.macbookPro16" = "MacBook Pro 16\"",
            "newDocumentPreset.imac24" = "iMac 24\"",
            "newDocumentPreset.iphone16" = "iPhone 16",
            "newDocumentPreset.iphone16ProMax" = "iPhone 16 Pro Max",
            "newDocumentPreset.ipadPro13" = "iPad Pro 13\"",
            "newDocumentPreset.android1080p" = "Android 1080p",
            "newDocumentPreset.appleWatch45" = "Apple Watch 45mm",
            "newDocumentPreset.hdtv1080p" = "HDTV 1080p",
            "newDocumentPreset.hdtv720p" = "HDTV 720p",
            "newDocumentPreset.uhd4k" = "UHD 4K",
            "newDocumentPreset.dci4k" = "DCI 4K",
            "newDocumentPreset.uhd8k" = "UHD 8K",
        ];
    }
}

/// The ids of the built-in presets, in the order of `PresetWords::names`.
const PRESET_IDS: [&str; 43] = [
    "landscape-3x2",
    "landscape-6x4",
    "landscape-7x5",
    "landscape-10x8",
    "portrait-2x3",
    "portrait-4x6",
    "portrait-5x7",
    "portrait-8x10",
    "letter",
    "legal",
    "tabloid",
    "a4",
    "a6",
    "a5",
    "a3",
    "b5",
    "b4",
    "b3",
    "c4",
    "c5",
    "c6",
    "dl",
    "grid-1000",
    "grid-2000",
    "poster",
    "postcard",
    "video-1080p",
    "video-720p",
    "web-most-common",
    "web-minimum",
    "web-large",
    "macbook-pro-16",
    "imac-24",
    "iphone-16",
    "iphone-16-pro-max",
    "ipad-pro-13",
    "android-1080p",
    "apple-watch-45",
    "hdtv-1080p",
    "hdtv-720p",
    "uhd-4k",
    "dci-4k",
    "uhd-8k",
];

impl NewDocumentPresetWords {
    /// The translated name of the built-in preset `id`; `None` for an id the set does not name.
    pub fn name_of(&self, id: &str) -> Option<&Label> {
        PRESET_IDS.iter().position(|listed| *listed == id).and_then(|at| self.names.get(at))
    }
}
