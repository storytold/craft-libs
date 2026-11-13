//! The `Document` tree: root values and included designmap nodes in source order.

use craft_xml::El;

use super::{Spread, Story, Values};

/// One IDML table-of-contents style entry before document style names are resolved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TocStyleEntryValues {
    pub style: Option<String>,
    pub level: Option<u8>,
    pub page_number_position: Option<String>,
    pub separator: Option<String>,
    pub entry_style: Option<String>,
    pub page_number_style: Option<String>,
    pub separator_style: Option<String>,
    pub sort: Option<bool>,
}

/// An IDML table-of-contents style and its entry grammar.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TocStyleValues {
    pub self_id: Option<String>,
    pub title: Option<String>,
    pub title_style: Option<String>,
    pub create_bookmarks: Option<bool>,
    pub entries: Vec<TocStyleEntryValues>,
}

impl TocStyleValues {
    /// Reads a `TOCStyle` element and its bounded `TOCStyleEntry` children.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        let mut result = Self {
            self_id: values.self_id().map(str::to_owned),
            title: values.get("Title").map(str::to_owned),
            title_style: values.get("TitleStyle").map(str::to_owned),
            create_bookmarks: values.get("CreateBookmarks").or_else(|| values.get("CreatePdfBookmarks")).map(|value| value == "true"),
            entries: Vec::new(),
        };
        result.entries.extend(el.elements().filter(|entry| entry.local() == "TOCStyleEntry").take(64).map(|entry| {
            let values = Values::of(entry);
            TocStyleEntryValues {
                style: values.get("Name").map(str::to_owned),
                level: values.get("Level").and_then(|value| value.trim().parse().ok()),
                page_number_position: values.get("PageNumberPosition").map(str::to_owned),
                separator: values.get("Separator").map(str::to_owned),
                entry_style: values.prop("FormatStyle"),
                page_number_style: values.prop("PageNumberStyle"),
                separator_style: values.prop("SeparatorStyle"),
                sort: values.get("Sort").map(|value| value.trim() == "true"),
            }
        }));
        result
    }
}

/// The top-level IDML elements consumed by document importers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopLevelKind {
    DocumentPreference,
    TransparencyPreference,
    TextPreference,
    MarginPreference,
    ViewPreference,
    GridPreference,
    PasteboardPreference,
    CompositeFont,
    KinsokuTable,
    MojikumiTable,
    FootnoteOption,
    EpubExportPreference,
    Layer,
    CrossReferenceFormat,
    Index,
    TextVariable,
    Story,
    Hyperlink,
    HyperlinkUrlDestination,
    HyperlinkPageDestination,
    HyperlinkPageItemSource,
    HyperlinkOtherDestination,
    MasterSpread,
    Spread,
    Section,
    TocStyle,
    Condition,
    NumberingList,
    StyleGroup,
    Other,
}

/// One included node in `designmap.xml`, classified by the shared IDML grammar.
#[derive(Clone, Debug, PartialEq)]
pub struct TopLevelNode {
    pub kind: TopLevelKind,
    data: TopLevelData,
}

#[derive(Clone, Debug, PartialEq)]
enum TopLevelData {
    Story(Story),
    Spread(Spread),
    Raw(El),
}

impl TopLevelNode {
    fn from_el(el: El) -> TopLevelNode {
        let kind = top_level_kind(el.local());
        let data = match kind {
            TopLevelKind::Story => TopLevelData::Story(Story::from_el(&el)),
            TopLevelKind::Spread | TopLevelKind::MasterSpread => TopLevelData::Spread(Spread::from_el(&el)),
            _ => TopLevelData::Raw(el),
        };
        TopLevelNode { kind, data }
    }

    /// The typed story or spread tree when this is one of those parts.
    pub fn story(&self) -> Option<&Story> {
        if let TopLevelData::Story(story) = &self.data { Some(story) } else { None }
    }

    pub fn spread(&self) -> Option<&Spread> {
        if let TopLevelData::Spread(spread) = &self.data { Some(spread) } else { None }
    }

    /// The original element, reconstructed from the typed story/spread model where applicable.
    pub fn to_el(&self) -> El {
        match &self.data {
            TopLevelData::Story(story) => story.to_el(),
            TopLevelData::Spread(spread) => spread.to_el(),
            TopLevelData::Raw(el) => el.clone(),
        }
    }

    pub fn values(&self) -> Values {
        Values::of(&self.to_el())
    }
}

/// A neutral view of an IDML package's designmap and included top-level parts.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelDocument {
    pub root: Values,
    pub top: Vec<TopLevelNode>,
    pub warnings: Vec<String>,
    pub metadata: Option<String>,
}

impl ModelDocument {
    /// Converts the shared bounded package reader's parsed document into typed document nodes.
    pub fn from_package(document: crate::Document) -> ModelDocument {
        ModelDocument {
            root: Values::of(&document.root),
            top: document.top.into_iter().map(TopLevelNode::from_el).collect(),
            warnings: document.warnings,
            metadata: document.metadata,
        }
    }
}

fn top_level_kind(name: &str) -> TopLevelKind {
    match name {
        "DocumentPreference" => TopLevelKind::DocumentPreference,
        "TransparencyPreference" => TopLevelKind::TransparencyPreference,
        "TextPreference" => TopLevelKind::TextPreference,
        "MarginPreference" => TopLevelKind::MarginPreference,
        "ViewPreference" => TopLevelKind::ViewPreference,
        "GridPreference" => TopLevelKind::GridPreference,
        "PasteboardPreference" => TopLevelKind::PasteboardPreference,
        "CompositeFont" => TopLevelKind::CompositeFont,
        "KinsokuTable" => TopLevelKind::KinsokuTable,
        "MojikumiTable" => TopLevelKind::MojikumiTable,
        "FootnoteOption" => TopLevelKind::FootnoteOption,
        "EPubExportPreference" => TopLevelKind::EpubExportPreference,
        "Layer" => TopLevelKind::Layer,
        "CrossReferenceFormat" => TopLevelKind::CrossReferenceFormat,
        "Index" => TopLevelKind::Index,
        "TextVariable" => TopLevelKind::TextVariable,
        "Story" => TopLevelKind::Story,
        "Hyperlink" => TopLevelKind::Hyperlink,
        "HyperlinkURLDestination" => TopLevelKind::HyperlinkUrlDestination,
        "HyperlinkPageDestination" => TopLevelKind::HyperlinkPageDestination,
        "HyperlinkPageItemSource" => TopLevelKind::HyperlinkPageItemSource,
        "MasterSpread" => TopLevelKind::MasterSpread,
        "Spread" => TopLevelKind::Spread,
        "Section" => TopLevelKind::Section,
        "TOCStyle" => TopLevelKind::TocStyle,
        "Condition" => TopLevelKind::Condition,
        "NumberingList" => TopLevelKind::NumberingList,
        name if name.starts_with("Root") && name.ends_with("StyleGroup") => TopLevelKind::StyleGroup,
        name if name.starts_with("Hyperlink") && name.ends_with("Destination") => TopLevelKind::HyperlinkOtherDestination,
        _ => TopLevelKind::Other,
    }
}
