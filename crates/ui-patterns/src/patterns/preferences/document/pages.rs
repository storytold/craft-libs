//! The pages of a settings document: StandardPage and PageInfo.

/// A page of the usual creative-app preferences, in its order, with the id (the top-level key of the settings struct
/// that fills it) and English title each has. An app shows the pages it has settings for; [`PageInfo::new`] adds its
/// own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StandardPage {
    /// General.
    General,
    /// Interface.
    Interface,
    /// Workspace.
    Workspace,
    /// Tools.
    Tools,
    /// History Log.
    HistoryLog,
    /// File Handling.
    FileHandling,
    /// Export.
    Export,
    /// Performance.
    Performance,
    /// Scratch Disks.
    ScratchDisks,
    /// Cursors.
    Cursors,
    /// Transparency & Gamut.
    TransparencyAndGamut,
    /// Units & Rulers.
    UnitsAndRulers,
    /// Guides, Grid & Slices.
    GuidesGridAndSlices,
    /// Plug-ins.
    PlugIns,
    /// Type.
    Type,
    /// Enhanced Controls.
    EnhancedControls,
    /// Camera Raw Defaults.
    RawDefaults,
    /// Integrations.
    Integrations,
}

impl StandardPage {
    /// Every page in its order.
    pub const ALL: [StandardPage; 18] = [
        StandardPage::General,
        StandardPage::Interface,
        StandardPage::Workspace,
        StandardPage::Tools,
        StandardPage::HistoryLog,
        StandardPage::FileHandling,
        StandardPage::Export,
        StandardPage::Performance,
        StandardPage::ScratchDisks,
        StandardPage::Cursors,
        StandardPage::TransparencyAndGamut,
        StandardPage::UnitsAndRulers,
        StandardPage::GuidesGridAndSlices,
        StandardPage::PlugIns,
        StandardPage::Type,
        StandardPage::EnhancedControls,
        StandardPage::RawDefaults,
        StandardPage::Integrations,
    ];

    /// The page's id and English title: `("general", "General")`, `("historyLog", "History Log")`,
    /// `("transparencyAndGamut", "Transparency & Gamut")`, `("guidesGridAndSlices", "Guides, Grid & Slices")`,
    /// `("plugIns", "Plug-ins")`, `("rawDefaults", "Camera Raw Defaults")`, and so on.
    pub fn info(self) -> PageInfo {
        let (id, title) = match self {
            StandardPage::General => ("general", "General"),
            StandardPage::Interface => ("interface", "Interface"),
            StandardPage::Workspace => ("workspace", "Workspace"),
            StandardPage::Tools => ("tools", "Tools"),
            StandardPage::HistoryLog => ("historyLog", "History Log"),
            StandardPage::FileHandling => ("fileHandling", "File Handling"),
            StandardPage::Export => ("export", "Export"),
            StandardPage::Performance => ("performance", "Performance"),
            StandardPage::ScratchDisks => ("scratchDisks", "Scratch Disks"),
            StandardPage::Cursors => ("cursors", "Cursors"),
            StandardPage::TransparencyAndGamut => ("transparencyAndGamut", "Transparency & Gamut"),
            StandardPage::UnitsAndRulers => ("unitsAndRulers", "Units & Rulers"),
            StandardPage::GuidesGridAndSlices => ("guidesGridAndSlices", "Guides, Grid & Slices"),
            StandardPage::PlugIns => ("plugIns", "Plug-ins"),
            StandardPage::Type => ("type", "Type"),
            StandardPage::EnhancedControls => ("enhancedControls", "Enhanced Controls"),
            StandardPage::RawDefaults => ("rawDefaults", "Camera Raw Defaults"),
            StandardPage::Integrations => ("integrations", "Integrations"),
        };
        PageInfo::new(id, title)
    }
}

/// A page: the top-level key of the settings struct it shows, and its English title, translated under
/// `settingsPage.<id>`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PageInfo {
    /// The top-level key of the settings struct the page shows (`"interface"`).
    pub id: String,
    /// The English title; translated under `settingsPage.<id>`.
    pub title: String,
}

impl PageInfo {
    /// A page with the given id and English title.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self { id: id.into(), title: title.into() }
    }
}

impl From<StandardPage> for PageInfo {
    fn from(page: StandardPage) -> Self {
        page.info()
    }
}
