//! The pure rules of a tab strip: what each kind of strip does, and which side a tab's close button is on.

use craft_behavior::fit::{TabFit, fit, fit_to_least, fit_whole};
use craft_host::target::Os;

/// The look of document tabs ([`TabKind::Document`]) app-wide. An app sets it once at its root (`RootEnv::values`);
/// the strip and its tabs read it from the env.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, crate::runtime::env::EnvValue, crate::contract::MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum DocumentTabLook {
    /// Whole titles side by side, a `*` after a modified tab's text.
    #[default]
    Whole,
    /// PdfCraft's tabs: they shrink before they overflow (a name at most 22 digits wide, at its least a readable
    /// prefix), stand the inline gap apart, and a modified tab shows the marker dot in place of the `*`.
    Shrinking,
}

/// The look of panel tabs ([`TabKind::Panel`]): a strip's choice ([`TabStrip::panel_look`](super::TabStrip)), which a
/// dock makes for its groups' tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, crate::contract::MemoProps)]
#[non_exhaustive]
pub enum PanelTabLook {
    /// The shown tab on the panel surface, square and as tall as the strip.
    #[default]
    Plain,
    /// VectorCraft's tabs: the shown tab on a filled pill (`surface.control` with `shape.radius.control` corners), a
    /// small control high in the middle of the strip; a tab under the pointer shows the pill too.
    Pill,
}

/// How far panel tabs ([`TabKind::Panel`]) shrink before they overflow, and how a label too long for its tab ends: a
/// strip's choice ([`TabStrip::panel_shrink`](super::TabStrip)), which a dock makes for its groups' tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, crate::contract::MemoProps)]
#[non_exhaustive]
pub enum PanelTabShrink {
    /// Tabs shrink to their title's first letters, the label clipped at the tab's edge with no ellipsis.
    #[default]
    FirstLetters,
    /// Tabs shrink no further than their label's readable least, its first
    /// [`TextRun::READABLE_CHARACTERS`](crate::primitives::TextRun::READABLE_CHARACTERS) characters and an ellipsis (a
    /// title at most one character longer stays whole), and a cut label ends in "…". Every tab shrinks, the shown one
    /// too, the widest first; only the tabs that do not fit at their least go to the overflow menu, the last ones
    /// first ([`FitMode::ShrinkToLeast`]).
    Ellipsis,
}

/// The kind of a strip. It sets the defaults below; the host picks one per strip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TabKind {
    /// Document tabs: whole titles, a close button on every closable tab, every document in the overflow menu. An app
    /// may give them PdfCraft's look ([`DocumentTabLook::Shrinking`]).
    #[default]
    Document,
    /// Panel tabs: tabs shrink before they overflow, no close button, only the hidden tabs in the overflow menu.
    Panel,
    /// Text tabs: only the labels, the selected one in the text colour and the others dimmed, no tab surface,
    /// divider or close button. Whole titles, every tab in the overflow menu (a row of categories in a dialog).
    Text,
    /// Ribbon tabs: label-only tabs, the shown tab in semibold, one sliding accent underline. Whole titles, overflow
    /// at the end of the strip.
    Ribbon,
}

/// How tab widths are fitted to the strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FitMode {
    /// Whole widths; only an active tab wider than the strip is cut.
    Whole,
    /// The widest tabs shrink to the minimum width, then the rest overflow. With `keep_selected` the selected tab is
    /// not among the tabs that shrink, and the overflow takes the last tabs first.
    #[non_exhaustive]
    Shrink {
        /// Whether the selected tab keeps its natural width.
        keep_selected: bool,
    },
    /// Every tab shrinks towards its own least width, the selected one too, the widest first; only the tabs that do
    /// not fit at their least overflow, the last ones first (`fit_to_least`). Panel tabs under
    /// [`PanelTabShrink::Ellipsis`].
    ShrinkToLeast,
}

impl FitMode {
    /// Fits tabs of the `natural` widths into `available`, keeping the `selected` tab. Whole widths give the tabs
    /// marked in `kept` the room before the other tabs (`fit_whole`); shrinking tabs have no kept tabs. `minimum` is
    /// the narrowest a shrinking tab becomes and `overflow` the width of the overflow button; [`FitMode::ShrinkToLeast`]
    /// takes it as every tab's least (a strip passes each tab's own to `fit_to_least`).
    pub fn fit(self, natural: &[f32], selected: usize, kept: &[bool], available: f32, minimum: f32, overflow: f32) -> TabFit {
        match self {
            FitMode::Whole => fit_whole(natural, selected, kept, available, overflow),
            FitMode::Shrink { keep_selected } => fit(natural, selected, available, minimum, overflow, keep_selected),
            FitMode::ShrinkToLeast => fit_to_least(natural, &vec![minimum; natural.len()], selected, available, overflow),
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
    /// How tab widths are fitted: whole titles for document, text and ribbon tabs, shrinking for panel tabs.
    pub fn fit_mode(self) -> FitMode {
        match self {
            Self::Document | Self::Text | Self::Ribbon => FitMode::Whole,
            Self::Panel => FitMode::Shrink { keep_selected: self.keeps_active_width() },
        }
    }

    /// How tab widths are fitted under the app's document tab look: [`DocumentTabLook::Shrinking`] shrinks document
    /// tabs as panel tabs do; every other kind and look is [`TabKind::fit_mode`].
    pub fn fit_mode_in(self, look: DocumentTabLook) -> FitMode {
        if self.shrinks_in(look) { FitMode::Shrink { keep_selected: self.keeps_active_width() } } else { self.fit_mode() }
    }

    /// How tab widths are fitted under the app's document tab look and the strip's panel shrink: panel tabs under
    /// [`PanelTabShrink::Ellipsis`] all shrink to their own readable least, the shown tab too, before any overflows
    /// ([`FitMode::ShrinkToLeast`]); every other kind and shrink is [`TabKind::fit_mode_in`].
    pub fn fit_mode_for(self, look: DocumentTabLook, shrink: PanelTabShrink) -> FitMode {
        if self == Self::Panel && shrink == PanelTabShrink::Ellipsis { FitMode::ShrinkToLeast } else { self.fit_mode_in(look) }
    }

    /// Whether these are document tabs under [`DocumentTabLook::Shrinking`].
    pub fn shrinks_in(self, look: DocumentTabLook) -> bool {
        self == Self::Document && look == DocumentTabLook::Shrinking
    }

    /// Whether the shown tab keeps its whole title while the other tabs shrink: panel tabs that shrink to their first
    /// letters (under [`PanelTabShrink::Ellipsis`] the shown tab shrinks with the others, [`TabKind::fit_mode_for`]).
    pub fn keeps_active_width(self) -> bool {
        self == Self::Panel
    }

    /// Whether a label too long for its tab is clipped at the tab's edge (panel tabs, which shrink to their first
    /// letters) instead of ending in an ellipsis.
    pub fn clips_label(self) -> bool {
        self == Self::Panel
    }

    /// Whether a label too long for its tab is clipped at the tab's edge under the strip's `shrink`: panel tabs that
    /// shrink to their first letters ([`PanelTabShrink::FirstLetters`]); under [`PanelTabShrink::Ellipsis`] they end
    /// in an ellipsis as the other kinds do.
    pub fn clips_label_in(self, shrink: PanelTabShrink) -> bool {
        self.clips_label() && shrink == PanelTabShrink::FirstLetters
    }

    /// Where the overflow button sits: at the strip's end for document and text tabs, after the last tab for panel
    /// tabs.
    pub fn overflow_place(self) -> OverflowPlace {
        match self {
            Self::Document | Self::Text | Self::Ribbon => OverflowPlace::StripEnd,
            Self::Panel => OverflowPlace::AfterTabs,
        }
    }

    /// Which tabs the overflow menu lists: every tab for document and text tabs, the hidden ones for panel tabs.
    pub fn overflow_menu(self) -> OverflowMenu {
        match self {
            Self::Document | Self::Text | Self::Ribbon => OverflowMenu::AllTabs,
            Self::Panel => OverflowMenu::HiddenOnly,
        }
    }

    /// Whether closable tabs show a close button on the tab; a panel's Close is in the host's context menu.
    pub fn shows_close(self) -> bool {
        self == Self::Document
    }

    /// Whether tabs draw only their label, with no surface, indicator or strip fill: text and ribbon tabs.
    pub fn text_only(self) -> bool {
        matches!(self, Self::Text | Self::Ribbon)
    }

    /// Whether the shown tab is marked by one accent underline that slides to the newly shown tab (ribbon tabs).
    pub fn slides_underline(self) -> bool {
        self == Self::Ribbon
    }

    /// Whether a middle click closes a closable tab.
    pub fn middle_click_closes(self) -> bool {
        self == Self::Document
    }

    /// Whether only a primary press shows a tab: a secondary click on a document tab opens its menu and a middle click
    /// closes it, and neither shows it.
    pub const fn primary_shows(self) -> bool {
        matches!(self, Self::Document)
    }

    /// Whether a drag resting over an unselected tab selects it.
    pub const fn selects_under_drag(self) -> bool {
        matches!(self, Self::Document)
    }

    /// Whether tabs draw a divider at their trailing edge, in every theme and density.
    pub fn trailing_divider(self) -> bool {
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
