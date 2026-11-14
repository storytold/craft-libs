//! The `Document` tree: root values and included designmap nodes in source order.

use craft_xml::El;

use super::{AlternatingFillValues, AlternatingStrokeValues, Spread, Story, StrokeValues, Values};

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

/// Date-based IDML text variable kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDateKind {
    Creation,
    Modification,
    Output,
}

/// Semantically classified `TextVariable` definitions before caller model mapping.
#[derive(Clone, Debug, PartialEq)]
pub enum TextVariableKind {
    CustomText { text: String },
    LastPageNumber { section: bool },
    ChapterNumber,
    FileName { extension: bool },
    Date { kind: TextDateKind, format: String },
    RunningHeader { style: String, character: bool, last_on_page: bool },
    CrossReference { page_number: bool },
    Imported { variable_type: String, settings_element: String, settings: Vec<(String, String)> },
}

/// Parsed values from an IDML `TextVariable` definition.
#[derive(Clone, Debug, PartialEq)]
pub struct TextVariableValues {
    pub self_id: Option<String>,
    pub name: String,
    pub before: String,
    pub after: String,
    pub kind: TextVariableKind,
}

/// A recognized IDML cross-reference format building block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CrossReferenceBuildingBlock {
    CustomText(String),
    FullParagraph,
    ParagraphText,
    ParagraphNumber,
    PageNumber,
    BookmarkName,
    ChapterNumber,
    FileName,
    PartialParagraph { delimiter: String, include_delimiter: bool },
}

/// Parsed IDML cross-reference format definition.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CrossReferenceFormatValues {
    pub self_id: Option<String>,
    pub name: String,
    pub blocks: Vec<CrossReferenceBuildingBlock>,
}

impl CrossReferenceFormatValues {
    /// Reads a `CrossReferenceFormat` and recognizes its building-block grammar.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        let mut result =
            Self { self_id: values.self_id().map(str::to_owned), name: values.get("Name").unwrap_or("Format").to_owned(), blocks: Vec::new() };
        for block in el.elements().filter(|child| child.local() == "BuildingBlock") {
            let values = Values::of(block);
            let text = |key: &str| values.get(key).filter(|value| *value != "$ID/").unwrap_or("").to_owned();
            let parsed = match values.get("BlockType").unwrap_or("") {
                "CustomStringBuildingBlock" => Some(CrossReferenceBuildingBlock::CustomText(text("CustomText"))),
                "FullParagraphBuildingBlock" => Some(CrossReferenceBuildingBlock::FullParagraph),
                "ParagraphTextBuildingBlock" => Some(CrossReferenceBuildingBlock::ParagraphText),
                "ParagraphNumberBuildingBlock" => Some(CrossReferenceBuildingBlock::ParagraphNumber),
                "PageNumberBuildingBlock" => Some(CrossReferenceBuildingBlock::PageNumber),
                "BookmarkNameBuildingBlock" => Some(CrossReferenceBuildingBlock::BookmarkName),
                "ChapterNumberBuildingBlock" => Some(CrossReferenceBuildingBlock::ChapterNumber),
                "FileNameBuildingBlock" => Some(CrossReferenceBuildingBlock::FileName),
                "PartialParagraphBuildingBlock" => Some(CrossReferenceBuildingBlock::PartialParagraph {
                    delimiter: text("AppliedDelimiter"),
                    include_delimiter: values.get("IncludeDelimiter") == Some("true"),
                }),
                _ => None,
            };
            if let Some(parsed) = parsed {
                result.blocks.push(parsed);
            }
        }
        result
    }

    /// Builds an IDML cross-reference format from DesignCraft's stored token definition.
    pub fn to_el(self_id: &str, name: &str, definition: &str) -> El {
        let mut root = El::new("CrossReferenceFormat").with_attr("Self", self_id).with_attr("Name", name).with_attr("AppliedCharacterStyle", "n");
        let mut index = 0;
        let mut block = |root: &mut El, kind: &str, text: &str, delimiter: &str, include: bool| {
            root.push(
                El::new("BuildingBlock")
                    .with_attr("Self", format!("{self_id}BuildingBlock{index}"))
                    .with_attr("BlockType", kind)
                    .with_attr("AppliedCharacterStyle", "n")
                    .with_attr("CustomText", if text.is_empty() { "$ID/" } else { text })
                    .with_attr("AppliedDelimiter", if delimiter.is_empty() { "$ID/" } else { delimiter })
                    .with_attr("IncludeDelimiter", if include { "true" } else { "false" }),
            );
            index += 1;
        };
        let mut rest = definition;
        while let Some(start) = rest.find('<') {
            let Some(end) = rest[start..].find('>') else { break };
            if start > 0 {
                block(&mut root, "CustomStringBuildingBlock", &rest[..start], "", false);
            }
            let tag = rest[start + 1..start + end].trim().trim_end_matches('/').trim();
            let attr =
                |key: &str| tag.find(&format!("{key}=\"")).and_then(|position| tag[position + key.len() + 2..].split('"').next()).unwrap_or("");
            match tag.split_whitespace().next().unwrap_or("") {
                "fullPara" => block(&mut root, "FullParagraphBuildingBlock", "", "", false),
                "paraText" => block(&mut root, "ParagraphTextBuildingBlock", "", "", false),
                "paraNum" => block(&mut root, "ParagraphNumberBuildingBlock", "", "", false),
                "pageNum" => block(&mut root, "PageNumberBuildingBlock", "", "", false),
                "txtAnchrName" => block(&mut root, "BookmarkNameBuildingBlock", "", "", false),
                "chapNum" => block(&mut root, "ChapterNumberBuildingBlock", "", "", false),
                "fileName" => block(&mut root, "FileNameBuildingBlock", "", "", false),
                "partialPara" => block(&mut root, "PartialParagraphBuildingBlock", "", attr("delim"), attr("includeDelim") == "true"),
                _ => block(&mut root, "CustomStringBuildingBlock", &rest[start..start + end + 1], "", false),
            }
            rest = &rest[start + end + 1..];
        }
        if !rest.is_empty() {
            block(&mut root, "CustomStringBuildingBlock", rest, "", false);
        }
        root
    }
}

/// Parsed EPUB document-splitting preference.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EpubSplitValues {
    pub break_document: bool,
    pub paragraph_style: Option<String>,
}

/// Parsed IDML condition metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConditionValues {
    pub self_id: Option<String>,
    pub name: Option<String>,
    pub indicator_rgb: Option<[u8; 3]>,
    pub visible: bool,
}

impl ConditionValues {
    /// Reads condition fields and its optional three-channel indicator color.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        let rgb: Vec<u8> = values
            .get("IndicatorColor")
            .unwrap_or("")
            .split_whitespace()
            .filter_map(|value| value.parse::<f64>().ok())
            .map(|value| value.clamp(0.0, 255.0) as u8)
            .collect();
        Self {
            self_id: values.self_id().map(str::to_owned),
            name: values.get("Name").map(str::to_owned),
            indicator_rgb: (rgb.len() == 3).then(|| [rgb[0], rgb[1], rgb[2]]),
            visible: values.get("Visible") != Some("false"),
        }
    }
}

/// Parsed IDML numbering-list metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NumberingListValues {
    pub name: String,
    pub continue_across_stories: bool,
}

/// Parsed cell-style assignments that are independent of DesignCraft's style resolution.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellStyleValues {
    pub self_id: Option<String>,
    pub name: Option<String>,
    pub based_on: Option<String>,
    pub fill: Option<String>,
    pub fill_tint: Option<f64>,
    pub insets: [Option<f64>; 4],
    pub vertical_justification: Option<String>,
    pub first_baseline: Option<String>,
    pub first_baseline_min: Option<f64>,
    pub paragraph_style: Option<String>,
    pub edge_strokes: [StrokeValues; 4],
}

impl CellStyleValues {
    /// Reads the cell-style value assignments and supports the older inset property names.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        let inset = |modern, legacy| values.num(modern).or_else(|| values.num(legacy)).map(|v| v.max(0.0));
        Self {
            self_id: values.self_id().map(str::to_owned),
            name: values.name().map(str::to_owned),
            based_on: values.prop("BasedOn"),
            fill: values.prop("FillColor"),
            fill_tint: values.num("FillTint"),
            insets: [
                inset("TextTopInset", "TopInset"),
                inset("TextLeftInset", "LeftInset"),
                inset("TextBottomInset", "BottomInset"),
                inset("TextRightInset", "RightInset"),
            ],
            vertical_justification: values.get("VerticalJustification").map(str::to_owned),
            first_baseline: values.get("FirstBaselineOffset").map(str::to_owned),
            first_baseline_min: values.num("MinimumFirstBaselineOffset").filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 8640.0)),
            paragraph_style: values.get("AppliedParagraphStyle").map(str::to_owned),
            edge_strokes: ["TopEdge", "LeftEdge", "BottomEdge", "RightEdge"].map(|edge| values.stroke_values(edge)),
        }
    }
}

/// Parsed table-style assignments that are independent of DesignCraft's style resolution.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableStyleValues {
    pub name: Option<String>,
    pub based_on: Option<String>,
    pub header_cell_style: Option<String>,
    pub body_cell_style: Option<String>,
    pub footer_cell_style: Option<String>,
    pub left_column_cell_style: Option<String>,
    pub right_column_cell_style: Option<String>,
    pub header_same_as_body: Option<bool>,
    pub footer_same_as_body: Option<bool>,
    pub left_column_same_as_body: Option<bool>,
    pub right_column_same_as_body: Option<bool>,
    pub space_before: Option<f64>,
    pub space_after: Option<f64>,
    pub border_strokes: [StrokeValues; 4],
    pub row_fills: AlternatingFillValues,
    pub column_fills: AlternatingFillValues,
    pub row_strokes: AlternatingStrokeValues,
    pub column_strokes: AlternatingStrokeValues,
}

impl TableStyleValues {
    /// Reads table-style regions, inheritance, spacing, and region reuse values.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        Self {
            name: values.name().map(str::to_owned),
            based_on: values.prop("BasedOn"),
            header_cell_style: values.get("HeaderRegionCellStyle").map(str::to_owned),
            body_cell_style: values.get("BodyRegionCellStyle").map(str::to_owned),
            footer_cell_style: values.get("FooterRegionCellStyle").map(str::to_owned),
            left_column_cell_style: values.get("LeftColumnRegionCellStyle").map(str::to_owned),
            right_column_cell_style: values.get("RightColumnRegionCellStyle").map(str::to_owned),
            header_same_as_body: values.boolean("HeaderRegionSameAsBodyRegion"),
            footer_same_as_body: values.boolean("FooterRegionSameAsBodyRegion"),
            left_column_same_as_body: values.boolean("LeftColumnRegionSameAsBodyRegion"),
            right_column_same_as_body: values.boolean("RightColumnRegionSameAsBodyRegion"),
            space_before: values.num("SpaceBefore"),
            space_after: values.num("SpaceAfter"),
            border_strokes: ["TopBorder", "LeftBorder", "BottomBorder", "RightBorder"].map(|border| values.stroke_values(border)),
            row_fills: values.alternating_fill_values("Row"),
            column_fills: values.alternating_fill_values("Column"),
            row_strokes: values.alternating_stroke_values("Row"),
            column_strokes: values.alternating_stroke_values("Column"),
        }
    }
}

impl NumberingListValues {
    /// Reads a `NumberingList` element.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        Self {
            name: values.get("Name").unwrap_or("").to_owned(),
            continue_across_stories: values.boolean("ContinueNumbersAcrossStories") == Some(true),
        }
    }
}

impl EpubSplitValues {
    /// Reads an `EPubExportPreference` element.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        Self { break_document: values.boolean("BreakDocument") == Some(true), paragraph_style: values.get("ParagraphStyleName").map(str::to_owned) }
    }
}

impl TextVariableValues {
    /// Reads a `TextVariable` and its preference record.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        let preference = el.elements().find(|child| child.name.ends_with("Preference"));
        let attr = |key: &str| preference.and_then(|child| child.attr(key)).unwrap_or("");
        let variable_type = values.get("VariableType").unwrap_or("");
        let date = |kind| TextVariableKind::Date { kind, format: if attr("Format").is_empty() { "MM/dd/yy".into() } else { attr("Format").into() } };
        let kind = match variable_type {
            "CustomTextType" => TextVariableKind::CustomText { text: attr("Contents").into() },
            "LastPageNumberType" => TextVariableKind::LastPageNumber { section: attr("Scope") == "SectionScope" },
            "ChapterNumberType" => TextVariableKind::ChapterNumber,
            "FileNameType" if attr("IncludePath") != "true" => TextVariableKind::FileName { extension: attr("IncludeExtension") == "true" },
            "CreationDateType" => date(TextDateKind::Creation),
            "ModificationDateType" => date(TextDateKind::Modification),
            "OutputDateType" => date(TextDateKind::Output),
            "MatchParagraphStyleType" | "MatchCharacterStyleType" => {
                let character = variable_type == "MatchCharacterStyleType";
                let style = attr(if character { "AppliedCharacterStyle" } else { "AppliedParagraphStyle" });
                if style.is_empty() {
                    TextVariableKind::Imported {
                        variable_type: variable_type.into(),
                        settings_element: preference.map(|child| child.name.clone()).unwrap_or_default(),
                        settings: preference.map(|child| child.attrs.clone()).unwrap_or_default(),
                    }
                } else {
                    TextVariableKind::RunningHeader { style: style.into(), character, last_on_page: attr("SearchStrategy") == "LastOnPage" }
                }
            }
            _ if variable_type.starts_with("Xref") => TextVariableKind::CrossReference { page_number: variable_type == "XrefPageNumberType" },
            _ => TextVariableKind::Imported {
                variable_type: variable_type.into(),
                settings_element: preference.map(|child| child.name.clone()).unwrap_or_default(),
                settings: preference.map(|child| child.attrs.clone()).unwrap_or_default(),
            },
        };
        let text = |key: &str| preference.and_then(|child| child.attr(key)).or_else(|| values.get(key)).unwrap_or("").to_owned();
        Self {
            self_id: values.self_id().map(str::to_owned),
            name: values.get("Name").unwrap_or("").to_owned(),
            before: text("TextBefore"),
            after: text("TextAfter"),
            kind,
        }
    }
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
