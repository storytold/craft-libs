//! Page setup (N3.08): paper sizes, orientation and margins, and the page setup that Word's Page
//! Setup and InDesign's Document Setup edit ([`crate::ui::setup`] has the dialog and the Layout
//! menus).
//!
//! The paper catalog is here rather than in craft-print: print spools PDFs and has no paper list,
//! and this crate is what the page apps share.

/// A named paper size, portrait, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaperSize {
    /// The English name, which is also its id ("Letter", "A4", "B5 (JIS)"); menus translate it.
    pub name: &'static str,
    /// Page width in points.
    pub width: f64,
    /// Page height in points.
    pub height: f64,
}

const fn paper(name: &'static str, width: f64, height: f64) -> PaperSize {
    PaperSize { name, width, height }
}

/// The paper sizes the page apps offer, in Word's order with InDesign's extras after: Word's twelve
/// (WordCraft), then A0–A2, A6, ISO B5, Letter - Half, Legal - Half and US Business Card
/// (DesignCraft). An app shows the ones it wants ([`crate::ui::setup::PageSetupDialog::papers`]).
pub const PAPER_SIZES: &[PaperSize] = &[
    paper("Letter", 612.0, 792.0),
    paper("Legal", 612.0, 1008.0),
    paper("Executive", 522.0, 756.0),
    paper("A3", 841.89, 1190.55),
    paper("A4", 595.28, 841.89),
    paper("A5", 419.53, 595.28),
    paper("B4 (JIS)", 728.5, 1031.8),
    paper("B5 (JIS)", 515.9, 728.5),
    paper("Tabloid", 792.0, 1224.0),
    paper("Statement", 396.0, 612.0),
    paper("Envelope #10", 297.0, 684.0),
    paper("Envelope DL", 311.8, 623.6),
    paper("A0", 2383.94, 3370.39),
    paper("A1", 1683.78, 2383.94),
    paper("A2", 1190.55, 1683.78),
    paper("A6", 297.638, 419.528),
    paper("B5", 498.898, 708.661),
    paper("Letter - Half", 396.0, 612.0),
    paper("Legal - Half", 504.0, 612.0),
    paper("US Business Card", 144.0, 252.0),
];

/// The paper in `list` that `width` × `height` is, either way round, within half a point (Word's
/// `paper_name`; DesignCraft allows 0.6 pt). `None` for a custom size. Where two names share a
/// size (Statement and Letter - Half), the first in `list` wins.
// wordcraft: crates/geom/src/lib.rs:294 `paper_name`; designcraft:
// crates/ui-egui/src/panels/properties/document.rs:26 `preset_name`.
pub fn paper_named(list: &[PaperSize], width: f64, height: f64) -> Option<&PaperSize> {
    const TOLERANCE: f64 = 0.5;
    let near = |a: f64, b: f64| (a - b).abs() < TOLERANCE;
    list.iter().find(|p| (near(p.width, width) && near(p.height, height)) || (near(p.width, height) && near(p.height, width)))
}

/// The smallest and largest page side, points (0.1" and 22", as Word allows).
pub const MIN_PAGE_SIDE: f64 = 7.2;
/// The largest page side, points (22").
pub const MAX_PAGE_SIDE: f64 = 1584.0;

/// The least text width and height margins leave on a page, points (WordCraft's
/// `MIN_TEXT_ROOM`).
pub const MIN_TEXT_ROOM: f64 = 36.0;

/// Which way the page's long side runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Orientation {
    #[allow(missing_docs)]
    #[default]
    Portrait,
    #[allow(missing_docs)]
    Landscape,
}

impl Orientation {
    /// `(width, height)` turned to this orientation: the long side across in landscape.
    pub fn turn(self, size: (f64, f64)) -> (f64, f64) {
        let (short, long) = (size.0.min(size.1), size.0.max(size.1));
        match self {
            Orientation::Portrait => (short, long),
            Orientation::Landscape => (long, short),
        }
    }
}

/// Page margins, points. With mirrored margins (facing pages) `left` is the inside and `right`
/// the outside; labels say Inside/Outside then.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Margins {
    /// Top margin in points.
    pub top: f64,
    /// Bottom margin in points.
    pub bottom: f64,
    /// Left margin in points.
    pub left: f64,
    /// Right margin in points.
    pub right: f64,
}

impl Margins {
    /// Margins with the same width `m` (points) on all four sides.
    pub const fn uniform(m: f64) -> Margins {
        Margins { top: m, bottom: m, left: m, right: m }
    }

    /// Whether the margins and `gutter` leave at least [`MIN_TEXT_ROOM`] across and down on a
    /// page `width` × `height` (Word refuses margins that don't: "The margins don't fit on a page
    /// this size; make the margins smaller first.").
    // wordcraft: crates/engine/src/cmd/page.rs:541 `margins_fit`.
    pub fn fit(&self, width: f64, height: f64, gutter: f64) -> bool {
        self.left + self.right + gutter <= width - MIN_TEXT_ROOM && self.top + self.bottom <= height - MIN_TEXT_ROOM
    }
}

/// A named margin set from Layout › Margins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarginPreset {
    /// The id settings and commands use (`"narrow"`).
    pub id: &'static str,
    /// The menu name ("Narrow"); the uniform presets add the margin in the unit ("Narrow
    /// 1.27 cm all", [`crate::ui::setup::PageLayoutMenus`]).
    pub name: &'static str,
    /// The margins this preset applies.
    pub margins: Margins,
    /// Mirrored margins (facing pages) come with this preset.
    pub mirrored: bool,
}

/// Word's six Layout › Margins presets, top, bottom, left, right in points.
// wordcraft: crates/engine/src/cmd/page.rs:372 `MARGIN_PRESETS`.
pub const MARGIN_PRESETS: [MarginPreset; 6] = [
    MarginPreset { id: "normal", name: "Normal", margins: Margins::uniform(72.0), mirrored: false },
    MarginPreset { id: "narrow", name: "Narrow", margins: Margins::uniform(36.0), mirrored: false },
    MarginPreset { id: "moderate", name: "Moderate", margins: Margins { top: 72.0, bottom: 72.0, left: 54.0, right: 54.0 }, mirrored: false },
    MarginPreset { id: "wide", name: "Wide", margins: Margins { top: 72.0, bottom: 72.0, left: 144.0, right: 144.0 }, mirrored: false },
    MarginPreset { id: "mirrored", name: "Mirrored", margins: Margins { top: 72.0, bottom: 72.0, left: 90.0, right: 72.0 }, mirrored: true },
    MarginPreset {
        id: "office2003",
        name: "Office 2003 Default",
        margins: Margins { top: 72.0, bottom: 72.0, left: 90.0, right: 90.0 },
        mirrored: false,
    },
];

impl MarginPreset {
    /// The preset `margins` match within half a point, with the same mirroring (Word checks the
    /// matching item in Layout › Margins).
    // wordcraft: crates/engine/src/cmd/page.rs:383 `margins_preset_at`.
    pub fn matching<'a>(presets: &'a [MarginPreset], margins: &Margins, mirrored: bool) -> Option<&'a MarginPreset> {
        const TOLERANCE: f64 = 0.5;
        let near = |a: f64, b: f64| (a - b).abs() < TOLERANCE;
        presets.iter().find(|p| {
            p.mirrored == mirrored
                && near(p.margins.top, margins.top)
                && near(p.margins.bottom, margins.bottom)
                && near(p.margins.left, margins.left)
                && near(p.margins.right, margins.right)
        })
    }
}

/// Word's Page Setup › Layout › Vertical alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PageVerticalAlign {
    #[allow(missing_docs)]
    #[default]
    Top,
    #[allow(missing_docs)]
    Center,
    #[allow(missing_docs)]
    Justify,
    #[allow(missing_docs)]
    Bottom,
}

/// What a page setup dialog edits. Sizes are portrait-agnostic: `width` × `height` as the page
/// shows in its orientation.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PageSetup {
    /// Page width in points, as shown in the current orientation.
    pub width: f64,
    /// Page height in points, as shown in the current orientation.
    pub height: f64,
    #[allow(missing_docs)]
    pub orientation: Orientation,
    #[allow(missing_docs)]
    pub margins: Margins,
    /// Word's gutter (binding room added to the inside margin).
    pub gutter: f64,
    /// Mirror margins (Word) or Facing Pages (InDesign).
    pub mirrored: bool,
    /// The number of pages (InDesign's Document Setup); `None` where pages flow (Word).
    pub pages: Option<u32>,
    #[allow(missing_docs)]
    pub vertical_align: PageVerticalAlign,
    /// Headers and footers: Different First Page.
    pub different_first: bool,
    /// Headers and footers: Different Odd & Even Pages.
    pub different_odd_even: bool,
    /// Section direction right-to-left (Word).
    pub right_to_left: bool,
}

impl Default for PageSetup {
    /// Letter, portrait, 1 inch margins.
    fn default() -> Self {
        PageSetup {
            width: 612.0,
            height: 792.0,
            orientation: Orientation::Portrait,
            margins: Margins::uniform(72.0),
            gutter: 0.0,
            mirrored: false,
            pages: None,
            vertical_align: PageVerticalAlign::Top,
            different_first: false,
            different_odd_even: false,
            right_to_left: false,
        }
    }
}

impl PageSetup {
    /// The page size turned to `orientation` (the fields swap, the margins turn with the page as
    /// Word's `layout.orientation` does: going to landscape the top takes the old left, the left the old bottom, the bottom the old right and the right the old top).
    // wordcraft: crates/ui-egui/src/dialogs/page_setup.rs:82 `page_paper_group` (the swap rule).
    pub fn set_orientation(&mut self, orientation: Orientation) {
        if orientation == self.orientation {
            return;
        }
        (self.width, self.height) = orientation.turn((self.width, self.height));
        let Margins { top, bottom, left, right } = self.margins;
        self.margins = match orientation {
            Orientation::Landscape => Margins { top: left, bottom: right, left: bottom, right: top },
            Orientation::Portrait => Margins { top: right, bottom: left, left: top, right: bottom },
        };
        self.orientation = orientation;
    }

    /// A typed size wider than tall turns the orientation to landscape, taller than wide to
    /// portrait; a square keeps it (Word).
    pub fn set_size(&mut self, width: f64, height: f64) {
        self.width = width.clamp(MIN_PAGE_SIDE, MAX_PAGE_SIDE);
        self.height = height.clamp(MIN_PAGE_SIDE, MAX_PAGE_SIDE);
        if self.width > self.height {
            self.orientation = Orientation::Landscape;
        } else if self.width < self.height {
            self.orientation = Orientation::Portrait;
        }
    }
}
