//! The `Document` tree: root values and included designmap nodes in source order.

use craft_xml::El;

use super::{AlternatingFillValues, AlternatingStrokeValues, MojikumiRow, Spread, Story, StrokeValues, Values};
use crate::value::{escape_id, num};
use crate::{prop, with_props};

fn bool_str(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

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
    /// Emits a `TOCStyle` element and its `TOCStyleEntry` children.
    pub fn to_el(&self, name: &str, mut fresh: impl FnMut() -> String) -> El {
        let mut el = El::new("TOCStyle")
            .with_attr("Self", self.self_id.as_deref().unwrap_or("TOCStyle/Table of Contents"))
            .with_attr("Name", name)
            .with_attr("Title", self.title.as_deref().unwrap_or(""))
            .with_attr("TitleStyle", self.title_style.as_deref().unwrap_or("ParagraphStyle/$ID/NormalParagraphStyle"))
            .with_attr("RunIn", "false")
            .with_attr("IncludeHidden", "false")
            .with_attr("IncludeBookDocuments", "false")
            .with_attr("CreateBookmarks", bool_str(self.create_bookmarks.unwrap_or(false)));
        for entry in &self.entries {
            el.push(
                El::new("TOCStyleEntry")
                    .with_attr("Self", fresh())
                    .with_attr("Name", entry.style.as_deref().unwrap_or(""))
                    .with_attr("FormatStyle", entry.entry_style.as_deref().unwrap_or(""))
                    .with_attr("Level", entry.level.unwrap_or(1).to_string())
                    .with_attr("PageNumberPosition", entry.page_number_position.as_deref().unwrap_or("AfterEntry"))
                    .with_attr("Separator", entry.separator.as_deref().unwrap_or("\t"))
                    .with_attr("PageNumberStyle", entry.page_number_style.as_deref().unwrap_or("CharacterStyle/$ID/[No character style]"))
                    .with_attr("SeparatorStyle", entry.separator_style.as_deref().unwrap_or("CharacterStyle/$ID/[No character style]"))
                    .with_attr("Sort", bool_str(entry.sort.unwrap_or(false))),
            );
        }
        el
    }
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

    /// Emits a `Condition` element from these metadata values.
    pub fn to_el(&self) -> El {
        let name = self.name.as_deref().unwrap_or("");
        let self_id = self.self_id.clone().unwrap_or_else(|| format!("Condition/{}", escape_id(name)));
        let rgb = self.indicator_rgb.unwrap_or([0, 0, 0]);
        El::new("Condition")
            .with_attr("Self", self_id)
            .with_attr("Name", name)
            .with_attr("IndicatorColor", format!("{} {} {}", rgb[0], rgb[1], rgb[2]))
            .with_attr("IndicatorMethod", "UseUnderline")
            .with_attr("UnderlineIndicatorAppearance", "Solid")
            .with_attr("Visible", bool_str(self.visible))
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

    /// Emits a `CellStyle` element from these values.
    pub fn to_el(&self) -> El {
        let mut el = El::new("CellStyle");
        if let Some(self_id) = &self.self_id {
            el.attrs.push(("Self".into(), self_id.clone()));
        }
        if let Some(name) = &self.name {
            el.attrs.push(("Name".into(), name.clone()));
        }
        if let Some(fill) = &self.fill {
            el.attrs.push(("FillColor".into(), fill.clone()));
        }
        if let Some(tint) = self.fill_tint {
            el.attrs.push(("FillTint".into(), num(tint)));
        }
        for ((text, legacy), value) in
            [("TextTopInset", "TopInset"), ("TextLeftInset", "LeftInset"), ("TextBottomInset", "BottomInset"), ("TextRightInset", "RightInset")]
                .iter()
                .zip(self.insets)
        {
            if let Some(value) = value {
                let s = num(value);
                el.attrs.push(((*text).into(), s.clone()));
                el.attrs.push(((*legacy).into(), s));
            }
        }
        if let Some(vj) = &self.vertical_justification {
            el.attrs.push(("VerticalJustification".into(), vj.clone()));
        }
        if let Some(fb) = &self.first_baseline {
            el.attrs.push(("FirstBaselineOffset".into(), fb.clone()));
        }
        if let Some(min) = self.first_baseline_min {
            el.attrs.push(("MinimumFirstBaselineOffset".into(), num(min)));
        }
        for (side, stroke) in ["TopEdge", "LeftEdge", "BottomEdge", "RightEdge"].iter().zip(&self.edge_strokes) {
            stroke.push_prefixed_attrs(&mut el, side, false);
        }
        if let Some(ps) = &self.paragraph_style {
            el.attrs.push(("AppliedParagraphStyle".into(), ps.clone()));
        }
        let based_on = match &self.based_on {
            Some(base) if base != "$ID/[None]" => prop("BasedOn", "object", base.clone()),
            _ => prop("BasedOn", "string", "$ID/[None]"),
        };
        with_props(el, vec![based_on])
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

    /// Emits a `TableStyle` element from these values.
    pub fn to_el(&self, self_id: &str) -> El {
        let mut el = El::new("TableStyle").with_attr("Self", self_id);
        if let Some(name) = &self.name {
            el.attrs.push(("Name".into(), name.clone()));
        }
        for (key, value) in [
            ("HeaderRegionCellStyle", &self.header_cell_style),
            ("BodyRegionCellStyle", &self.body_cell_style),
            ("FooterRegionCellStyle", &self.footer_cell_style),
            ("LeftColumnRegionCellStyle", &self.left_column_cell_style),
            ("RightColumnRegionCellStyle", &self.right_column_cell_style),
        ] {
            if let Some(value) = value {
                el.attrs.push((key.into(), value.clone()));
            }
        }
        for (key, value) in [
            ("HeaderRegionSameAsBodyRegion", self.header_same_as_body),
            ("FooterRegionSameAsBodyRegion", self.footer_same_as_body),
            ("LeftColumnRegionSameAsBodyRegion", self.left_column_same_as_body),
            ("RightColumnRegionSameAsBodyRegion", self.right_column_same_as_body),
        ] {
            if let Some(value) = value {
                el.attrs.push((key.into(), bool_str(value).into()));
            }
        }
        for (side, stroke) in ["TopBorder", "LeftBorder", "BottomBorder", "RightBorder"].iter().zip(&self.border_strokes) {
            stroke.push_prefixed_attrs(&mut el, side, false);
        }
        self.row_fills.push_attrs(&mut el, "Row");
        self.column_fills.push_attrs(&mut el, "Column");
        self.row_strokes.push_attrs(&mut el, "Row");
        self.column_strokes.push_attrs(&mut el, "Column");
        if let Some(value) = self.space_before {
            el.attrs.push(("SpaceBefore".into(), num(value)));
        }
        if let Some(value) = self.space_after {
            el.attrs.push(("SpaceAfter".into(), num(value)));
        }
        let mut props = Vec::new();
        if let Some(base) = &self.based_on {
            props.push(if base == "$ID/[No table style]" {
                prop("BasedOn", "string", "$ID/[No table style]")
            } else {
                prop("BasedOn", "object", base.clone())
            });
        }
        with_props(el, props)
    }
}

impl NumberingListValues {
    /// The default IDML numbering list (`$ID/[Default]`).
    pub fn default_list() -> Self {
        Self { name: "$ID/[Default]".into(), continue_across_stories: false }
    }

    /// Reads a `NumberingList` element.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        Self {
            name: values.get("Name").unwrap_or("").to_owned(),
            continue_across_stories: values.boolean("ContinueNumbersAcrossStories") == Some(true),
        }
    }

    /// Emits a `NumberingList` element.
    pub fn to_el(&self) -> El {
        El::new("NumberingList")
            .with_attr("Self", format!("NumberingList/{}", escape_id(&self.name)))
            .with_attr("Name", &self.name)
            .with_attr("ContinueNumbersAcrossStories", bool_str(self.continue_across_stories))
            .with_attr("ContinueNumbersAcrossDocuments", "false")
    }
}

impl EpubSplitValues {
    /// Reads an `EPubExportPreference` element.
    pub fn from_el(el: &El) -> Self {
        let values = Values::of(el);
        Self { break_document: values.boolean("BreakDocument") == Some(true), paragraph_style: values.get("ParagraphStyleName").map(str::to_owned) }
    }

    /// Emits an `EPubExportPreference` element.
    pub fn to_el(&self) -> El {
        El::new("EPubExportPreference")
            .with_attr("BreakDocument", bool_str(self.break_document))
            .with_attr("ParagraphStyleName", self.paragraph_style.as_deref().unwrap_or(""))
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

    /// Emits a `TextVariable` element with its type-specific preference child.
    pub fn to_el(&self) -> El {
        let pref = |name: &str| El::new(name).with_attr("TextBefore", &self.before);
        let (ty, settings) = match &self.kind {
            TextVariableKind::CustomText { text } => ("CustomTextType", Some(pref("CustomTextVariablePreference").with_attr("Contents", text))),
            TextVariableKind::LastPageNumber { section } => (
                "LastPageNumberType",
                Some(
                    pref("PageNumberVariablePreference")
                        .with_attr("Format", "Current")
                        .with_attr("Scope", if *section { "SectionScope" } else { "DocumentScope" }),
                ),
            ),
            TextVariableKind::ChapterNumber => ("ChapterNumberType", Some(pref("ChapterNumberVariablePreference").with_attr("Format", "Current"))),
            TextVariableKind::FileName { extension } => (
                "FileNameType",
                Some(pref("FileNameVariablePreference").with_attr("IncludePath", "false").with_attr("IncludeExtension", bool_str(*extension))),
            ),
            TextVariableKind::Date { kind, format } => {
                let ty = match kind {
                    TextDateKind::Creation => "CreationDateType",
                    TextDateKind::Modification => "ModificationDateType",
                    TextDateKind::Output => "OutputDateType",
                };
                (ty, Some(pref("DateVariablePreference").with_attr("Format", format)))
            }
            TextVariableKind::RunningHeader { style, character, last_on_page } => {
                let (ty, el, key) = if *character {
                    ("MatchCharacterStyleType", "MatchCharacterStylePreference", "AppliedCharacterStyle")
                } else {
                    ("MatchParagraphStyleType", "MatchParagraphStylePreference", "AppliedParagraphStyle")
                };
                let strategy = if *last_on_page { "LastOnPage" } else { "FirstOnPage" };
                (
                    ty,
                    Some(
                        pref(el)
                            .with_attr(key, style)
                            .with_attr("SearchStrategy", strategy)
                            .with_attr("ChangeCase", "None")
                            .with_attr("DeleteEndPunctuation", "false"),
                    ),
                )
            }
            TextVariableKind::CrossReference { page_number } => (if *page_number { "XrefPageNumberType" } else { "XrefChapterNumberType" }, None),
            TextVariableKind::Imported { variable_type, settings_element, settings } => {
                let el = (!settings_element.is_empty()).then(|| El { name: settings_element.clone(), attrs: settings.clone(), children: Vec::new() });
                (variable_type.as_str(), el)
            }
        };
        let self_id = self.self_id.clone().unwrap_or_else(|| format!("dTextVariablen{}", self.name));
        let mut el = El::new("TextVariable").with_attr("Self", self_id).with_attr("Name", &self.name).with_attr("VariableType", ty);
        if let Some(mut s) = settings {
            if !matches!(self.kind, TextVariableKind::Imported { .. }) {
                s.set_attr("TextAfter", &self.after);
            }
            el.push(s);
        }
        el
    }
}

/// The standard `[No Language]` and `English: USA` `Language` elements emitted in `designmap.xml`.
pub fn default_languages() -> [El; 2] {
    [
        El::new("Language")
            .with_attr("Self", "Language/$ID/[No Language]")
            .with_attr("Name", "$ID/[No Language]")
            .with_attr("SingleQuotes", "''")
            .with_attr("DoubleQuotes", "\"\"")
            .with_attr("PrimaryLanguageName", "$ID/[No Language]")
            .with_attr("SublanguageName", "$ID/[No Language]")
            .with_attr("Id", "0")
            .with_attr("HyphenationVendor", "$ID/")
            .with_attr("SpellingVendor", "$ID/"),
        El::new("Language")
            .with_attr("Self", "Language/$ID/English%3a USA")
            .with_attr("Name", "$ID/English: USA")
            .with_attr("SingleQuotes", "\u{2018}\u{2019}")
            .with_attr("DoubleQuotes", "\u{201C}\u{201D}")
            .with_attr("PrimaryLanguageName", "$ID/English")
            .with_attr("SublanguageName", "$ID/USA")
            .with_attr("Id", "269")
            .with_attr("HyphenationVendor", "Hunspell")
            .with_attr("SpellingVendor", "Hunspell"),
    ]
}

/// The built-in cross-reference chapter/page number `TextVariable` elements.
pub fn default_xref_text_variables() -> [El; 2] {
    [("XRefChapterNumber", "XrefChapterNumberType"), ("XRefPageNumber", "XrefPageNumberType")].map(|(name, ty)| {
        let full_name = format!("<?AID 001b?>TV {name}");
        El::new("TextVariable").with_attr("Self", format!("dTextVariablen{full_name}")).with_attr("Name", &full_name).with_attr("VariableType", ty)
    })
}

/// Reusable IDML `KinsokuTable` schema values.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KinsokuTableValues {
    pub name: String,
    pub cant_begin_line_chars: String,
    pub cant_end_line_chars: String,
    pub cant_be_separated_chars: String,
    pub hanging_punctuation_chars: String,
}

impl KinsokuTableValues {
    /// Emits a `KinsokuTable` element.
    pub fn to_el(&self) -> El {
        El::new("KinsokuTable")
            .with_attr("Self", format!("KinsokuTable/{}", escape_id(&self.name)))
            .with_attr("Name", &self.name)
            .with_attr("CantBeginLineChars", &self.cant_begin_line_chars)
            .with_attr("CantEndLineChars", &self.cant_end_line_chars)
            .with_attr("CantBeSeparatedChars", &self.cant_be_separated_chars)
            .with_attr("HangingPunctuationChars", &self.hanging_punctuation_chars)
    }
}

/// Reusable IDML `MojikumiTable` schema values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MojikumiTableValues {
    pub name: String,
    pub based_on: String,
    pub overrides: Vec<MojikumiRow>,
}

impl MojikumiTableValues {
    /// Emits a `MojikumiTable` element with optional spacing overrides.
    pub fn to_el(&self) -> El {
        let mut el = El::new("MojikumiTable").with_attr("Self", format!("MojikumiTable/{}", escape_id(&self.name))).with_attr("Name", &self.name);
        if !self.based_on.is_empty() {
            el.set_attr("BasedOnMojikumiSet", &self.based_on);
        }
        if !self.overrides.is_empty() {
            let mut list = El::new("OverrideMojikumiAkiList");
            for r in &self.overrides {
                list.push(
                    El::new("OverrideMojikumiAkiType")
                        .with_attr("TargetMojikumiClass", r.target_class.to_string())
                        .with_attr("SideMojikumiClass", r.side_class.to_string())
                        .with_attr("SideIsAfterTarget", bool_str(r.after))
                        .with_attr("Minimum", num(r.minimum))
                        .with_attr("Desired", num(r.desired))
                        .with_attr("Maximum", num(r.maximum))
                        .with_attr("CompressionPriority", r.priority.to_string())
                        .with_attr("AkiDoesNotFloat", bool_str(r.does_not_float)),
                );
            }
            let mut props = El::new("Properties");
            props.push(list);
            el.push(props);
        }
        el
    }
}

/// IDML `LayerColor` property value: either a named enumeration or an RGB triplet.
#[derive(Clone, Debug, PartialEq)]
pub enum LayerColorValue {
    Named(String),
    Rgb([f64; 3]),
}

/// Reusable IDML `Layer` schema values.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerValues {
    pub self_id: String,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub ignore_wrap: bool,
    pub show_guides: bool,
    pub lock_guides: bool,
    pub printable: bool,
    pub color: LayerColorValue,
}

impl LayerValues {
    /// Emits a `Layer` element with its `<Properties><LayerColor>` child.
    pub fn to_el(&self) -> El {
        let color = match &self.color {
            LayerColorValue::Named(name) => prop("LayerColor", "enumeration", name),
            LayerColorValue::Rgb(rgb) => {
                let mut el = El::new("LayerColor").with_attr("type", "list");
                for c in rgb {
                    el.push(prop("ListItem", "double", num(*c)));
                }
                el
            }
        };
        let el = El::new("Layer")
            .with_attr("Self", &self.self_id)
            .with_attr("Name", &self.name)
            .with_attr("Visible", bool_str(self.visible))
            .with_attr("Locked", bool_str(self.locked))
            .with_attr("IgnoreWrap", bool_str(self.ignore_wrap))
            .with_attr("ShowGuides", bool_str(self.show_guides))
            .with_attr("LockGuides", bool_str(self.lock_guides))
            .with_attr("UI", "true")
            .with_attr("Expendable", "true")
            .with_attr("Printable", bool_str(self.printable));
        with_props(el, vec![color])
    }
}

/// Reusable IDML `Section` schema values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionValues {
    pub self_id: String,
    pub length: usize,
    pub name: String,
    pub continue_numbering: bool,
    pub include_section_prefix: bool,
    pub page_number_start: Option<u32>,
    pub marker: String,
    pub page_start: String,
    pub section_prefix: String,
    pub alternate_layout: Option<(String, usize)>,
    pub page_number_style: String,
}

impl SectionValues {
    /// Emits a `Section` element with its `<Properties><PageNumberStyle>` child.
    pub fn to_el(&self) -> El {
        let mut el = El::new("Section")
            .with_attr("Self", &self.self_id)
            .with_attr("Length", self.length.to_string())
            .with_attr("Name", &self.name)
            .with_attr("ContinueNumbering", bool_str(self.continue_numbering))
            .with_attr("IncludeSectionPrefix", bool_str(self.include_section_prefix));
        if let Some(n) = self.page_number_start {
            el.set_attr("PageNumberStart", n.to_string());
        }
        el = el.with_attr("Marker", &self.marker).with_attr("PageStart", &self.page_start).with_attr("SectionPrefix", &self.section_prefix);
        if let Some((layout, layout_len)) = &self.alternate_layout {
            el = el.with_attr("AlternateLayout", layout).with_attr("AlternateLayoutLength", layout_len.to_string());
        }
        with_props(el, vec![prop("PageNumberStyle", "enumeration", &self.page_number_style)])
    }
}

/// Emits a `HyperlinkURLDestination` element.
pub fn hyperlink_url_destination_el(self_id: &str, name: &str, url: &str) -> El {
    El::new("HyperlinkURLDestination")
        .with_attr("Self", self_id)
        .with_attr("Name", name)
        .with_attr("DestinationURL", url)
        .with_attr("Hidden", "false")
}

/// Emits a `HyperlinkPageDestination` element.
pub fn hyperlink_page_destination_el(self_id: &str, name: &str, destination_page: &str) -> El {
    El::new("HyperlinkPageDestination")
        .with_attr("Self", self_id)
        .with_attr("Name", name)
        .with_attr("DestinationPage", destination_page)
        .with_attr("ViewSetting", "FitPage")
        .with_attr("Hidden", "false")
}

/// Emits a `HyperlinkPageItemSource` element.
pub fn hyperlink_page_item_source_el(self_id: &str, name: &str, source_page_item: &str) -> El {
    El::new("HyperlinkPageItemSource")
        .with_attr("Self", self_id)
        .with_attr("Name", name)
        .with_attr("SourcePageItem", source_page_item)
        .with_attr("Hidden", "false")
}

/// Emits a `Hyperlink` element joining `source` and `destination`.
pub fn hyperlink_el(self_id: &str, name: &str, source: &str, destination: &str, visible: bool, destination_unique_key: u64) -> El {
    let el = El::new("Hyperlink")
        .with_attr("Self", self_id)
        .with_attr("Name", name)
        .with_attr("Source", source)
        .with_attr("Visible", bool_str(visible))
        .with_attr("Highlight", "None")
        .with_attr("Width", "Thin")
        .with_attr("BorderStyle", "Solid")
        .with_attr("Hidden", "false")
        .with_attr("DestinationUniqueKey", (destination_unique_key & 0xFFFF_FFFF).to_string());
    with_props(el, vec![prop("Destination", "object", destination)])
}

/// Emits the root `ColorGroup` (`ColorGroup/[Root Color Group]`) with `ColorGroupSwatch` children.
pub fn root_color_group_el(swatch_ids: &[String], swatch_id_prefix: &str) -> El {
    let mut cg = El::new("ColorGroup")
        .with_attr("Self", "ColorGroup/[Root Color Group]")
        .with_attr("Name", "[Root Color Group]")
        .with_attr("IsRootColorGroup", "true");
    for (n, id) in swatch_ids.iter().enumerate() {
        cg.push(El::new("ColorGroupSwatch").with_attr("Self", format!("{swatch_id_prefix}ColorGroupSwatch{n:x}")).with_attr("SwatchItemRef", id));
    }
    cg
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
