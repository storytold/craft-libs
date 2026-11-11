//! The pure rules of a tab strip: what each kind of strip does, and which side a tab's close button is on.

use craft_behavior::fit::{TabFit, fit, fit_whole};
use craft_host::target::Os;

/// The kind of a strip. It sets the defaults below; the host picks one per strip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TabKind {
    /// Document tabs: whole titles, a close button on every closable tab, every document in the overflow menu.
    #[default]
    Document,
    /// Panel tabs: tabs shrink before they overflow, no close button, only the hidden tabs in the overflow menu.
    Panel,
    /// Text tabs: only the labels, the selected one in the text colour and the others dimmed, no tab surface,
    /// divider or close button. Whole titles, every tab in the overflow menu (a row of categories in a dialog).
    Text,
}

/// How tab widths are fitted to the strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FitMode {
    /// Whole widths; only an active tab wider than the strip is cut.
    Whole,
    /// The widest tabs shrink to the minimum width, then the rest overflow.
    Shrink,
}

impl FitMode {
    /// Fits tabs of the `natural` widths into `available`, keeping the `selected` tab. `minimum` is the narrowest a
    /// shrinking tab becomes and `overflow` the width of the overflow button.
    pub fn fit(self, natural: &[f32], selected: usize, available: f32, minimum: f32, overflow: f32) -> TabFit {
        match self {
            FitMode::Whole => fit_whole(natural, selected, available, overflow),
            FitMode::Shrink => fit(natural, selected, available, minimum, overflow),
        }
    }
}

/// Where the overflow button sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OverflowPlace {
    /// Pinned to the reading end of the tab area.
    StripEnd,
    /// Right after the last shown tab.
    AfterTabs,
}

/// Which tabs the overflow menu lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OverflowMenu {
    /// Every tab, the active one checked.
    AllTabs,
    /// The hidden tabs only.
    HiddenOnly,
}

impl TabKind {
    /// How tab widths are fitted: whole titles for document and text tabs, shrinking for panel tabs.
    pub fn fit_mode(self) -> FitMode {
        match self {
            Self::Document | Self::Text => FitMode::Whole,
            Self::Panel => FitMode::Shrink,
        }
    }

    /// Where the overflow button sits: at the strip's end for document and text tabs, after the last tab for panel
    /// tabs.
    pub fn overflow_place(self) -> OverflowPlace {
        match self {
            Self::Document | Self::Text => OverflowPlace::StripEnd,
            Self::Panel => OverflowPlace::AfterTabs,
        }
    }

    /// Which tabs the overflow menu lists: every tab for document and text tabs, the hidden ones for panel tabs.
    pub fn overflow_menu(self) -> OverflowMenu {
        match self {
            Self::Document | Self::Text => OverflowMenu::AllTabs,
            Self::Panel => OverflowMenu::HiddenOnly,
        }
    }

    /// Whether closable tabs show a close button on the tab; a panel's Close is in the host's context menu.
    pub fn shows_close(self) -> bool {
        self == Self::Document
    }

    /// Whether tabs draw only their label, with no surface, indicator or strip fill.
    pub fn text_only(self) -> bool {
        self == Self::Text
    }

    /// Whether a middle click closes a closable tab.
    pub fn middle_click_closes(self) -> bool {
        self == Self::Document
    }

    /// Whether the strip draws a divider at each tab's trailing edge in a theme that divides rows.
    pub fn compact_dividers(self) -> bool {
        self == Self::Document
    }
}

/// The physical side of a tab its close button is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CloseSide {
    /// The left side.
    Left,
    /// The right side.
    Right,
}

/// The side of the tab the close button is on: before the title on macOS, after it elsewhere, in reading order, so
/// a mirrored strip puts it on the other physical side.
pub fn close_side(os: Os, rtl: bool) -> CloseSide {
    let before_title = os == Os::Mac;
    if before_title != rtl { CloseSide::Left } else { CloseSide::Right }
}
