//! The `Document` tree: root values and included designmap nodes in source order.

use craft_xml::El;

use super::{Spread, Story, Values};

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
