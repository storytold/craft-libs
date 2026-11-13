//! Stories: paragraph and character ranges, their text, and what text holds (tables, notes,
//! anchored page items, hyperlink and change containers).

use craft_xml::{El, Local, Node};
use std::collections::{BTreeMap, HashMap};

use super::{PageItem, Values, tidy};

/// Resolved IDML anchored-object options from an item and its inherited style levels.
#[derive(Clone, Debug, PartialEq)]
pub struct AnchoredObjectValues {
    pub position: String,
    pub anchor_point: String,
    pub horizontal_alignment: String,
    pub horizontal_reference: String,
    pub vertical_alignment: String,
    pub vertical_reference: String,
    pub pin_position: bool,
    pub spine_relative: bool,
    pub space_above: f64,
    pub x_offset: f64,
    pub y_offset: f64,
}

/// An IDML `AnchoredObjectSetting` element.
#[derive(Clone, Debug, PartialEq)]
pub struct AnchoredObjectSetting {
    attributes: Vec<(String, String)>,
}

impl AnchoredObjectSetting {
    pub fn inline_position(y_offset: f64) -> Self {
        Self { attributes: vec![("AnchoredPosition".into(), "InlinePosition".into()), ("AnchorYoffset".into(), crate::value::num(y_offset))] }
    }

    pub fn above_line(horizontal_alignment: &str, space_above: f64, y_offset: f64) -> Self {
        Self {
            attributes: vec![
                ("AnchoredPosition".into(), "AboveLine".into()),
                ("HorizontalAlignment".into(), horizontal_alignment.into()),
                ("AnchorSpaceAbove".into(), crate::value::num(space_above)),
                ("AnchorYoffset".into(), crate::value::num(y_offset)),
            ],
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn anchored(
        anchor_point: &str,
        horizontal_reference: &str,
        vertical_reference: &str,
        horizontal_alignment: &str,
        vertical_alignment: &str,
        x_offset: f64,
        y_offset: f64,
        pin_position: bool,
        spine_relative: bool,
    ) -> Self {
        Self {
            attributes: vec![
                ("AnchoredPosition".into(), "Anchored".into()),
                ("AnchorPoint".into(), anchor_point.into()),
                ("HorizontalReferencePoint".into(), horizontal_reference.into()),
                ("VerticalReferencePoint".into(), vertical_reference.into()),
                ("HorizontalAlignment".into(), horizontal_alignment.into()),
                ("VerticalAlignment".into(), vertical_alignment.into()),
                ("AnchorXoffset".into(), crate::value::num(x_offset)),
                ("AnchorYoffset".into(), crate::value::num(y_offset)),
                ("PinPosition".into(), bool_string(pin_position)),
                ("SpineRelative".into(), bool_string(spine_relative)),
            ],
        }
    }

    pub fn to_el(&self) -> El {
        let mut element = El::new("AnchoredObjectSetting");
        for (key, value) in &self.attributes {
            element.set_attr(key, value);
        }
        element
    }
}

fn bool_string(value: bool) -> String {
    if value { "true" } else { "false" }.into()
}

impl AnchoredObjectValues {
    /// Resolves attributes item-first from `AnchoredObjectSetting` children.
    pub fn from_elements(elements: &[&El]) -> Self {
        let get = |key: &str, default: &str| {
            elements
                .iter()
                .filter_map(|element| element.child(Local("AnchoredObjectSetting")))
                .find_map(|setting| Values::of(setting).get(key).map(str::to_owned))
                .unwrap_or_else(|| default.to_owned())
        };
        let number = |key: &str| get(key, "0").parse::<f64>().ok().filter(|value| value.is_finite()).unwrap_or(0.0);
        Self {
            position: get("AnchoredPosition", "InlinePosition"),
            anchor_point: get("AnchorPoint", "BottomRightAnchor"),
            horizontal_alignment: get("HorizontalAlignment", "LeftAlign"),
            horizontal_reference: get("HorizontalReferencePoint", "TextFrame"),
            vertical_alignment: get("VerticalAlignment", "BottomAlign"),
            vertical_reference: get("VerticalReferencePoint", "LineBaseline"),
            pin_position: get("PinPosition", "true") == "true",
            spine_relative: get("SpineRelative", "false") == "true",
            space_above: number("AnchorSpaceAbove"),
            x_offset: number("AnchorXoffset"),
            y_offset: number("AnchorYoffset"),
        }
    }
}

/// A `Story`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Story {
    pub values: Values,
    pub children: Vec<TextNode>,
}

/// The IDML `StoryPreference` element written for a story.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StoryPreference {
    pub optical_margin_alignment: bool,
    pub optical_margin_size: f64,
    pub vertical: bool,
    pub right_to_left: bool,
}

impl StoryPreference {
    pub fn to_el(self) -> El {
        El::new("StoryPreference")
            .with_attr("OpticalMarginAlignment", self.optical_margin_alignment)
            .with_attr("OpticalMarginSize", self.optical_margin_size)
            .with_attr("FrameType", "TextFrameType")
            .with_attr("StoryOrientation", if self.vertical { "Vertical" } else { "Horizontal" })
            .with_attr("StoryDirection", if self.right_to_left { "RightToLeftDirection" } else { "LeftToRightDirection" })
    }
}

/// One node of a story's text, in document order. IDML nests freely (an `XMLElement` or a
/// `HyperlinkTextSource` can hold paragraph ranges or content), so every container holds these.
#[derive(Clone, Debug, PartialEq)]
pub enum TextNode {
    /// `ParagraphStyleRange`.
    Paragraph(Range),
    /// `CharacterStyleRange`.
    Character(Range),
    /// `Content`: text, with the special characters InDesign writes as `<?ACE n?>`.
    Content(Vec<ContentPart>),
    /// `Br`: the end of a paragraph.
    Break,
    /// A processing instruction at a structural story position. The parent serializer preserves
    /// it in place; `to_el` represents it as a one-instruction `Content` fragment.
    ProcessingInstruction {
        target: String,
        data: String,
    },
    /// Text inside another element ([`CONTAINERS`]): a hyperlink's source, a tracked change, an
    /// XML element, a cross-reference's source, a footnote, a note.
    Container(Container),
    Table(Table),
    Anchor(StoryAnchor),
    PageReference(PageReference),
    TextVariable(Values),
    /// A page item anchored in the text.
    Item(PageItem),
    /// Anything else (`StoryPreference`, `HyperlinkTextDestination`, `TextVariableInstance`, …),
    /// as written.
    Other(El),
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoryAnchor {
    pub name: String,
    pub values: Values,
}

impl StoryAnchor {
    /// Builds the IDML `Self` value for a hyperlink text destination.
    pub fn destination_self(id: u64) -> String {
        format!("HyperlinkTextDestination/a{id}")
    }

    /// Writes an IDML `HyperlinkTextDestination` from a DesignCraft-independent anchor identity.
    pub fn destination_el(id: u64, name: &str) -> El {
        El::new("HyperlinkTextDestination")
            .with_attr("Self", Self::destination_self(id))
            .with_attr("Name", name)
            .with_attr("Hidden", "false")
            .with_attr("DestinationUniqueKey", id & 0xFFFF_FFFF)
    }

    pub fn self_id(&self) -> Option<&str> {
        self.values.self_id()
    }
    pub fn label(&self) -> Option<&str> {
        self.values.name()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageReference {
    pub values: Values,
}

/// Nested index topics and their See / See also references for one IDML `Index`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IndexValues {
    pub topics: BTreeMap<Vec<String>, String>,
    pub cross_references: Vec<IndexCrossReferenceValues>,
}

/// An index cross-reference attached to a topic path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexCrossReferenceValues {
    pub from: Vec<String>,
    pub reference_type: String,
    pub target: String,
}

/// A topic path and its sort keys, as they are saved on nested IDML `Topic` elements.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexTopicPath {
    pub path: Vec<String>,
    pub sort: Vec<String>,
}

/// An IDML index cross-reference as stored under an index topic.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexReference {
    pub from: IndexTopicPath,
    pub referenced_topic: Option<String>,
    pub reference_type: Option<String>,
}

/// Parsed IDML index topics and cross-references.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexDocument {
    pub topics: HashMap<String, IndexTopicPath>,
    pub cross_references: Vec<IndexReference>,
}

impl IndexDocument {
    pub fn from_el(element: &El) -> Self {
        let mut result = Self::default();
        let mut stack: Vec<(&El, IndexTopicPath)> =
            element.elements().filter(|child| child.local() == "Topic").map(|topic| (topic, IndexTopicPath::default())).collect();
        while let Some((topic, prefix)) = stack.pop() {
            let name = topic.attr("Name").unwrap_or_default().to_string();
            let mut current = prefix.clone();
            current.path.push(name);
            if let Some(key) = topic.attr("SortOrder").filter(|key| !key.is_empty()) {
                current.sort.resize(prefix.path.len(), String::new());
                current.sort.push(key.to_string());
            }
            if let Some(self_id) = topic.attr("Self") {
                result.topics.insert(self_id.to_string(), current.clone());
            }
            for child in topic.elements() {
                if child.local() == "CrossReference" {
                    result.cross_references.push(IndexReference {
                        from: current.clone(),
                        referenced_topic: child.attr("ReferencedTopic").map(str::to_string),
                        reference_type: child.attr("CrossReferenceType").map(str::to_string),
                    });
                } else if child.local() == "Topic" {
                    stack.push((child, current.clone()));
                }
            }
        }
        result
    }
}

impl IndexValues {
    /// Builds the IDML `Self` value for a topic path.
    pub fn topic_self(path: &[String]) -> String {
        let mut id = String::from("ix");
        for part in path {
            id.push_str("Topicn");
            id.push_str(&crate::value::escape_id(part));
        }
        id
    }

    /// Writes the nested IDML index grammar, assigning a fresh ID to each cross-reference.
    pub fn to_el(&self, mut fresh: impl FnMut() -> String) -> El {
        let mut topics = self.topics.clone();
        for reference in &self.cross_references {
            topics.entry(vec![reference.target.clone()]).or_default();
        }

        fn build(
            topics: &BTreeMap<Vec<String>, String>,
            references: &[IndexCrossReferenceValues],
            prefix: &[String],
            fresh: &mut impl FnMut() -> String,
        ) -> Vec<El> {
            let mut out = Vec::new();
            for (path, sort_order) in topics.iter().filter(|(path, _)| path.len() == prefix.len() + 1 && path.starts_with(prefix)) {
                let mut element = El::new("Topic")
                    .with_attr("Self", IndexValues::topic_self(path))
                    .with_attr("SortOrder", sort_order)
                    .with_attr("Name", path.last().map_or("", String::as_str));
                for reference in references.iter().filter(|reference| reference.from == *path) {
                    element.push(
                        El::new("CrossReference")
                            .with_attr("Self", fresh())
                            .with_attr("ReferencedTopic", IndexValues::topic_self(std::slice::from_ref(&reference.target)))
                            .with_attr("CrossReferenceType", &reference.reference_type)
                            .with_attr("CustomTypeString", ""),
                    );
                }
                element.children.extend(build(topics, references, path, fresh).into_iter().map(Node::El));
                out.push(element);
            }
            out
        }

        let mut element = El::new("Index").with_attr("Self", "ix");
        element.children = build(&topics, &self.cross_references, &[], &mut fresh).into_iter().map(Node::El).collect();
        element
    }
}

/// Text saved inside an IDML `CrossReferenceSource` before the application regenerates it from
/// the cross-reference format. Page-number variables are separate segments so callers can retain
/// the last displayed page while mapping to their own text model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedCrossReference {
    pub self_id: Option<String>,
    pub applied_format: Option<String>,
    pub parts: Vec<SavedCrossReferencePart>,
    /// The first directly nested character range, which supplies the marker's formatting.
    pub character_range: Option<Range>,
}

/// The IDML cross-reference source wrapper around its saved character range.
pub struct CrossReferenceSource;

impl CrossReferenceSource {
    pub fn to_el(self_id: &str, applied_format: &str, name: &str, character_range: El) -> El {
        El::new("CrossReferenceSource")
            .with_attr("Self", self_id)
            .with_attr("AppliedFormat", applied_format)
            .with_attr("Name", name)
            .with_attr("Hidden", "false")
            .with_child(character_range)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SavedCrossReferencePart {
    Text(String),
    PageNumber { result_text: Option<String> },
}

impl SavedCrossReference {
    /// Reads the bounded saved-text grammar. `page_variables` contains IDs whose variable type is
    /// the cross-reference page number; when the document omits variable definitions, InDesign's
    /// conventional `TV XRefPageNumber` suffix is recognized.
    pub fn from_el(el: &El, page_variables: &[String]) -> SavedCrossReference {
        let mut parsed = SavedCrossReference {
            self_id: el.attr("Self").map(str::to_string),
            applied_format: el.attr("AppliedFormat").map(str::to_string),
            parts: Vec::new(),
            character_range: el.find(Local("CharacterStyleRange")).and_then(|range| match TextNode::from_el(range) {
                TextNode::Character(range) => Some(range),
                _ => None,
            }),
        };
        saved_xref_parts(el, page_variables, &mut parsed.parts, 0);
        parsed
    }

    pub fn from_container(container: &Container, page_variables: &[String]) -> SavedCrossReference {
        Self::from_el(&container.to_el(), page_variables)
    }
}

fn saved_xref_parts(el: &El, page_variables: &[String], parts: &mut Vec<SavedCrossReferencePart>, depth: usize) {
    if depth > 16 {
        return;
    }
    for child in el.elements() {
        match child.local() {
            "Content" => parts.push(SavedCrossReferencePart::Text(child.deep_text())),
            "TextVariableInstance" => {
                let id = child.attr("AssociatedTextVariable").unwrap_or_default();
                let page_number =
                    page_variables.iter().any(|variable| variable == id) || (page_variables.is_empty() && id.ends_with("TV XRefPageNumber"));
                if page_number {
                    parts.push(SavedCrossReferencePart::PageNumber { result_text: child.attr("ResultText").map(str::to_string) });
                } else {
                    parts.push(SavedCrossReferencePart::Text(child.attr("ResultText").unwrap_or_default().to_string()));
                }
            }
            "Change" if child.attr("ChangeType") == Some("DeletedText") => {}
            "CharacterStyleRange" | "Change" => saved_xref_parts(child, page_variables, parts, depth + 1),
            _ => {}
        }
    }
}

impl PageReference {
    pub fn referenced_topic(&self) -> Option<&str> {
        self.values.get("ReferencedTopic")
    }
    pub fn reference_type(&self) -> Option<&str> {
        self.values.get("PageReferenceType")
    }
    pub fn limit(&self) -> Option<f64> {
        self.values.num("PageReferenceLimit")
    }

    /// Writes a page-reference record from the shared index model's type and target.
    pub fn to_el(self_id: &str, reference_type: &str, topic: &str, id: usize, limit: Option<u32>) -> El {
        let mut el = El::new("PageReference")
            .with_attr("Self", self_id)
            .with_attr("PageReferenceType", reference_type)
            .with_attr("ReferencedTopic", topic)
            .with_attr("Id", id);
        if let Some(limit) = limit {
            el.set_attr("PageReferenceLimit", limit);
        }
        el
    }
}

/// The elements read as [`TextNode::Container`].
pub const CONTAINERS: [&str; 7] = ["HyperlinkTextSource", "Change", "XMLElement", "CrossReferenceSource", "Footnote", "Note", "EndnoteRange"];

/// A paragraph or character range: its style and overrides ([`Values`]) and its content.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Range {
    pub values: Values,
    pub children: Vec<TextNode>,
}

/// An element that holds text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Container {
    /// The element's name (one of [`CONTAINERS`]).
    pub name: String,
    pub values: Values,
    pub children: Vec<TextNode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerKind {
    HyperlinkTextSource,
    Change,
    XMLElement,
    CrossReferenceSource,
    Footnote,
    Note,
    EndnoteRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackedChangeKind {
    Deleted,
    Inserted,
    Other,
}

impl Container {
    pub fn to_el(&self) -> El {
        with_children(self.values.element(&self.name), &self.children)
    }

    pub fn kind(&self) -> ContainerKind {
        match self.name.as_str() {
            "HyperlinkTextSource" => ContainerKind::HyperlinkTextSource,
            "Change" => ContainerKind::Change,
            "XMLElement" => ContainerKind::XMLElement,
            "CrossReferenceSource" => ContainerKind::CrossReferenceSource,
            "Footnote" => ContainerKind::Footnote,
            "Note" => ContainerKind::Note,
            _ => ContainerKind::EndnoteRange,
        }
    }

    pub fn self_id(&self) -> Option<&str> {
        self.values.self_id()
    }

    pub fn change_kind(&self) -> Option<TrackedChangeKind> {
        (self.kind() == ContainerKind::Change).then(|| match self.values.get("ChangeType") {
            Some("DeletedText") => TrackedChangeKind::Deleted,
            Some("InsertedText" | "MovedText") => TrackedChangeKind::Inserted,
            _ => TrackedChangeKind::Other,
        })
    }
}

/// A piece of a `Content` element.
#[derive(Clone, Debug, PartialEq)]
pub enum ContentPart {
    Text(String),
    /// `<?ACE n?>`: a special character.
    Ace(Ace),
    /// Another processing instruction, as written.
    Pi {
        target: String,
        data: String,
    },
}

/// The special characters InDesign writes as `<?ACE n?>`. Codes this crate does not name are
/// kept by number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ace {
    /// 4: the footnote's number, at the start of its text.
    FootnoteNumber,
    /// 7: Indent to Here.
    IndentToHere,
    /// 8: Right Indent Tab.
    RightIndentTab,
    /// 18: the current page's number.
    PageNumber,
    /// 19: the section marker.
    SectionMarker,
    Other(u32),
}

impl Ace {
    /// Parses an IDML ACE instruction's decimal code.
    pub fn from_decimal(value: &str) -> Option<Ace> {
        value.trim().parse::<u32>().ok().map(Ace::from_code)
    }

    pub fn from_code(code: u32) -> Ace {
        match code {
            4 => Ace::FootnoteNumber,
            7 => Ace::IndentToHere,
            8 => Ace::RightIndentTab,
            18 => Ace::PageNumber,
            19 => Ace::SectionMarker,
            n => Ace::Other(n),
        }
    }

    pub fn code(self) -> u32 {
        match self {
            Ace::FootnoteNumber => 4,
            Ace::IndentToHere => 7,
            Ace::RightIndentTab => 8,
            Ace::PageNumber => 18,
            Ace::SectionMarker => 19,
            Ace::Other(n) => n,
        }
    }
}

/// A `Table`: its rows and columns as written, and its cells.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub values: Values,
    pub children: Vec<TableChild>,
}

/// The row and column grid of an IDML table, read without allocating cell models.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableLayout {
    pub rows: Vec<TableRow>,
    pub columns: Vec<TableColumn>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableRow {
    pub values: Values,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableColumn {
    pub values: Values,
}

impl TableLayout {
    pub fn from_el(el: &El) -> TableLayout {
        Self::from_table(&Table::from_el(el))
    }

    pub fn from_table(table: &Table) -> TableLayout {
        let mut layout = TableLayout::default();
        for child in &table.children {
            if let TableChild::Other(element) = child {
                match element.local() {
                    "Row" => layout.rows.push(TableRow { values: Values::of(element) }),
                    "Column" => layout.columns.push(TableColumn { values: Values::of(element) }),
                    _ => {}
                }
            }
        }
        layout
    }
}

impl TableRow {
    pub fn auto_grow(&self) -> Option<bool> {
        self.values.boolean("AutoGrow")
    }

    pub fn minimum_height(&self) -> Option<f64> {
        self.values.num("MinimumHeight")
    }

    pub fn single_row_height(&self) -> Option<f64> {
        self.values.num("SingleRowHeight")
    }
}

impl TableColumn {
    pub fn single_column_width(&self) -> Option<f64> {
        self.values.num("SingleColumnWidth")
    }
}

/// A child of a table, in document order.
#[derive(Clone, Debug, PartialEq)]
pub enum TableChild {
    Cell(Cell),
    /// `Row`, `Column`, and anything else, as written.
    Other(El),
}

/// A table `Cell`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
    pub values: Values,
    pub children: Vec<TextNode>,
}

/// Format-level options on an IDML table cell, before DesignCraft maps them into its model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellOptions {
    pub fill_color: Option<String>,
    pub fill_tint: Option<f64>,
    pub insets: [Option<f64>; 4],
    pub vertical_justification: Option<String>,
    pub first_baseline_offset: Option<String>,
    pub minimum_first_baseline_offset: Option<f64>,
    pub rotation_angle: Option<f64>,
    pub edge_stroke_priorities: [i32; 4],
    pub edge_strokes: [super::StrokeValues; 4],
}

/// The children of `el` other than `<Properties>`, as text nodes.
fn nodes(el: &El) -> Vec<TextNode> {
    el.children
        .iter()
        .filter_map(|node| match node {
            Node::El(e) if !e.is(Local("Properties")) => Some(TextNode::from_el(e)),
            Node::Pi { target, data } => Some(TextNode::ProcessingInstruction { target: target.clone(), data: data.clone() }),
            _ => None,
        })
        .collect()
}

fn with_children(mut el: El, children: &[TextNode]) -> El {
    el.children.extend(children.iter().map(TextNode::to_node));
    el
}

impl TextNode {
    pub fn from_el(el: &El) -> TextNode {
        match el.local() {
            "ParagraphStyleRange" => TextNode::Paragraph(Range { values: Values::of(el), children: nodes(el) }),
            "CharacterStyleRange" => TextNode::Character(Range { values: Values::of(el), children: nodes(el) }),
            "Content" => TextNode::Content(content_parts(el)),
            "Br" => TextNode::Break,
            "Table" => TextNode::Table(Table::from_el(el)),
            "HyperlinkTextDestination" | "ParagraphDestination" => {
                TextNode::Anchor(StoryAnchor { name: el.local().to_string(), values: Values::of(el) })
            }
            "PageReference" => TextNode::PageReference(PageReference { values: Values::of(el) }),
            "TextVariableInstance" => TextNode::TextVariable(Values::of(el)),
            n if CONTAINERS.contains(&n) => TextNode::Container(Container { name: n.to_string(), values: Values::of(el), children: nodes(el) }),
            _ => PageItem::from_el(el).map_or_else(|| TextNode::Other(tidy(el)), TextNode::Item),
        }
    }

    pub fn to_el(&self) -> El {
        match self {
            TextNode::Paragraph(r) => with_children(r.values.element("ParagraphStyleRange"), &r.children),
            TextNode::Character(r) => with_children(r.values.element("CharacterStyleRange"), &r.children),
            TextNode::Content(parts) => {
                let mut c = El::new("Content");
                c.children = parts
                    .iter()
                    .map(|p| match p {
                        ContentPart::Text(t) => Node::Text(t.clone()),
                        ContentPart::Ace(a) => Node::Pi { target: "ACE".into(), data: a.code().to_string() },
                        ContentPart::Pi { target, data } => Node::Pi { target: target.clone(), data: data.clone() },
                    })
                    .collect();
                c
            }
            TextNode::Break => El::new("Br"),
            TextNode::ProcessingInstruction { target, data } => {
                let mut content = El::new("Content");
                content.children.push(Node::Pi { target: target.clone(), data: data.clone() });
                content
            }
            TextNode::Container(c) => with_children(c.values.element(&c.name), &c.children),
            TextNode::Table(t) => t.to_el(),
            TextNode::Anchor(a) => a.values.element(&a.name),
            TextNode::PageReference(r) => r.values.element("PageReference"),
            TextNode::TextVariable(v) => v.element("TextVariableInstance"),
            TextNode::Item(i) => i.to_el(),
            TextNode::Other(e) => e.clone(),
        }
    }

    /// Serializes this node in its parent position, preserving processing instructions as XML
    /// instructions rather than wrapping them in an element.
    pub fn to_node(&self) -> Node {
        match self {
            TextNode::ProcessingInstruction { target, data } => Node::Pi { target: target.clone(), data: data.clone() },
            _ => Node::El(self.to_el()),
        }
    }

    /// The nodes this one holds; none for content, breaks, page items and others.
    pub fn children(&self) -> &[TextNode] {
        match self {
            TextNode::Paragraph(r) | TextNode::Character(r) => &r.children,
            TextNode::Container(c) => &c.children,
            _ => &[],
        }
    }
}

/// A `Content` element's text and instructions, in order. CDATA is text.
fn content_parts(el: &El) -> Vec<ContentPart> {
    let mut parts: Vec<ContentPart> = Vec::new();
    for n in &el.children {
        let part = match n {
            Node::Text(t) | Node::CData(t) => ContentPart::Text(t.clone()),
            Node::Pi { target, data } if target == "ACE" => match data.trim().parse::<u32>() {
                Ok(code) => ContentPart::Ace(Ace::from_code(code)),
                Err(_) => ContentPart::Pi { target: target.clone(), data: data.clone() },
            },
            Node::Pi { target, data } => ContentPart::Pi { target: target.clone(), data: data.clone() },
            Node::El(_) | Node::Comment(_) => continue,
        };
        // Adjacent text is one piece, as written.
        match (parts.last_mut(), part) {
            (Some(ContentPart::Text(a)), ContentPart::Text(b)) => a.push_str(&b),
            (_, p) => parts.push(p),
        }
    }
    parts
}

impl Story {
    pub fn from_el(el: &El) -> Story {
        Story { values: Values::of(el), children: nodes(el) }
    }

    pub fn to_el(&self) -> El {
        with_children(self.values.element("Story"), &self.children)
    }

    /// `StoryPreference` (`StoryDirection`, `StoryOrientation`, `OpticalMarginAlignment`, …).
    pub fn preference(&self) -> Option<Values> {
        self.children.iter().find_map(|c| match c {
            TextNode::Other(e) if e.local() == "StoryPreference" => Some(Values::of(e)),
            _ => None,
        })
    }

    /// Whether `StoryDirection` selects right-to-left order.
    pub fn right_to_left(&self) -> bool {
        self.preference().and_then(|values| values.get("StoryDirection").map(str::to_string)).as_deref() == Some("RightToLeftDirection")
    }

    /// Whether `StoryOrientation` selects vertical composition.
    pub fn vertical(&self) -> bool {
        self.preference().and_then(|values| values.get("StoryOrientation").map(str::to_string)).as_deref() == Some("Vertical")
    }

    /// Whether optical margin alignment is enabled; false when absent.
    pub fn optical_margin_enabled(&self) -> bool {
        self.preference().and_then(|values| values.boolean("OpticalMarginAlignment")) == Some(true)
    }

    /// The `OpticalMarginSize`, if finite.
    pub fn optical_margin_size(&self) -> Option<f64> {
        self.preference().and_then(|values| values.num("OpticalMarginSize"))
    }

    /// The story's paragraph ranges in reading order, those inside containers included, but not
    /// those of its tables, footnotes or notes (which are text of their own).
    pub fn paragraphs(&self) -> Vec<&Range> {
        paragraphs(&self.children)
    }

    /// The story's text: content, `\n` for each break, and `ace` for each special character
    /// (left out where it gives `None`). Tables, footnotes, notes and anchored items are left
    /// out: they are text of their own.
    pub fn text(&self, ace: impl Fn(Ace) -> Option<char>) -> String {
        text(&self.children, &ace)
    }
}

/// Whether a container's text belongs to the text around it (a hyperlink's text) or is text of
/// its own (a footnote).
fn inline_container(name: &str) -> bool {
    !matches!(name, "Footnote" | "Note" | "EndnoteRange")
}

/// The paragraph ranges in `nodes`, depth first, without recursing.
fn paragraphs(nodes: &[TextNode]) -> Vec<&Range> {
    let mut out = Vec::new();
    let mut stack: Vec<&TextNode> = nodes.iter().rev().collect();
    while let Some(n) = stack.pop() {
        match n {
            TextNode::Paragraph(r) => out.push(r),
            TextNode::Container(c) if inline_container(&c.name) => stack.extend(c.children.iter().rev()),
            _ => {}
        }
    }
    out
}

fn text(nodes: &[TextNode], ace: &dyn Fn(Ace) -> Option<char>) -> String {
    let mut out = String::new();
    let mut stack: Vec<&TextNode> = nodes.iter().rev().collect();
    while let Some(n) = stack.pop() {
        match n {
            TextNode::Content(parts) => {
                for p in parts {
                    match p {
                        ContentPart::Text(t) => out.push_str(t),
                        ContentPart::Ace(a) => out.extend(ace(*a)),
                        ContentPart::Pi { .. } => {}
                    }
                }
            }
            TextNode::Break => out.push('\n'),
            TextNode::ProcessingInstruction { target, data } if target == "ACE" => {
                if let Ok(code) = data.trim().parse::<u32>() {
                    out.extend(ace(Ace::from_code(code)));
                }
            }
            TextNode::Paragraph(r) | TextNode::Character(r) => stack.extend(r.children.iter().rev()),
            TextNode::Container(c) if inline_container(&c.name) => stack.extend(c.children.iter().rev()),
            _ => {}
        }
    }
    out
}

impl Range {
    /// `ParagraphBreakType`, except `Anywhere` which means no forced break.
    pub fn paragraph_break_type(&self) -> Option<&str> {
        self.values.get("ParagraphBreakType").filter(|kind| *kind != "Anywhere")
    }

    /// `AppliedParagraphStyle` of a paragraph range.
    pub fn paragraph_style(&self) -> Option<&str> {
        self.values.get("AppliedParagraphStyle")
    }

    /// `AppliedCharacterStyle` of a character range.
    pub fn character_style(&self) -> Option<&str> {
        self.values.get("AppliedCharacterStyle")
    }

    /// The range's text, as [`Story::text`] gives it.
    pub fn text(&self, ace: impl Fn(Ace) -> Option<char>) -> String {
        text(&self.children, &ace)
    }

    /// The character ranges in this range, those inside inline containers included.
    pub fn characters(&self) -> Vec<&Range> {
        let mut out = Vec::new();
        let mut stack: Vec<&TextNode> = self.children.iter().rev().collect();
        while let Some(n) = stack.pop() {
            match n {
                TextNode::Character(r) => out.push(r),
                TextNode::Container(c) if inline_container(&c.name) => stack.extend(c.children.iter().rev()),
                _ => {}
            }
        }
        out
    }
}

impl Container {
    /// The container's own paragraph ranges (a footnote's, a note's).
    pub fn paragraphs(&self) -> Vec<&Range> {
        paragraphs(&self.children)
    }

    pub fn text(&self, ace: impl Fn(Ace) -> Option<char>) -> String {
        text(&self.children, &ace)
    }
}

impl Table {
    pub fn from_el(el: &El) -> Table {
        let children =
            el.elements()
                .filter(|e| !e.is(Local("Properties")))
                .map(|c| {
                    if c.local() == "Cell" {
                        TableChild::Cell(Cell { values: Values::of(c), children: nodes(c) })
                    } else {
                        TableChild::Other(tidy(c))
                    }
                })
                .collect();
        Table { values: Values::of(el), children }
    }

    pub fn to_el(&self) -> El {
        let mut el = self.values.element("Table");
        el.children.extend(self.children.iter().map(|c| {
            Node::El(match c {
                TableChild::Cell(cell) => with_children(cell.values.element("Cell"), &cell.children),
                TableChild::Other(e) => e.clone(),
            })
        }));
        el
    }

    /// The `Row` elements' values, in order.
    pub fn rows(&self) -> Vec<Values> {
        self.others("Row")
    }

    /// The `Column` elements' values, in order.
    pub fn columns(&self) -> Vec<Values> {
        self.others("Column")
    }

    fn others(&self, name: &str) -> Vec<Values> {
        self.children
            .iter()
            .filter_map(|c| match c {
                TableChild::Other(e) if e.local() == name => Some(Values::of(e)),
                _ => None,
            })
            .collect()
    }

    pub fn cells(&self) -> impl DoubleEndedIterator<Item = &Cell> {
        self.children.iter().filter_map(|c| if let TableChild::Cell(c) = c { Some(c) } else { None })
    }
}

impl Cell {
    pub fn from_el(el: &El) -> Cell {
        Cell { values: Values::of(el), children: nodes(el) }
    }

    pub fn to_el(&self) -> El {
        with_children(self.values.element("Cell"), &self.children)
    }

    /// `Name`: `column:row` of the cell's top left (`2:0`).
    pub fn position(&self) -> Option<(u32, u32)> {
        let (c, r) = self.values.name()?.split_once(':')?;
        Some((c.trim().parse().ok()?, r.trim().parse().ok()?))
    }

    /// `ColumnSpan` and `RowSpan`; 1 when absent or not a positive whole number.
    pub fn span(&self) -> (u32, u32) {
        let n = |k: &str| self.values.num(k).filter(|v| *v >= 1.0 && v.fract() == 0.0 && *v <= f64::from(u32::MAX)).map_or(1, |v| v as u32);
        (n("ColumnSpan"), n("RowSpan"))
    }

    pub fn options(&self) -> CellOptions {
        let inset = |current: &str, legacy: &str| self.values.num(current).or_else(|| self.values.num(legacy));
        let priority = |side: &str| self.values.prop(&format!("{side}StrokePriority")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        CellOptions {
            fill_color: self.values.value("FillColor"),
            fill_tint: self.values.num("FillTint"),
            insets: [
                inset("TextTopInset", "TopInset"),
                inset("TextLeftInset", "LeftInset"),
                inset("TextBottomInset", "BottomInset"),
                inset("TextRightInset", "RightInset"),
            ],
            vertical_justification: self.values.value("VerticalJustification"),
            first_baseline_offset: self.values.value("FirstBaselineOffset"),
            minimum_first_baseline_offset: self.values.num("MinimumFirstBaselineOffset"),
            rotation_angle: self.values.num("RotationAngle"),
            edge_stroke_priorities: [priority("TopEdge"), priority("LeftEdge"), priority("BottomEdge"), priority("RightEdge")],
            edge_strokes: ["TopEdge", "LeftEdge", "BottomEdge", "RightEdge"].map(|side| self.values.stroke_values(side)),
        }
    }

    pub fn paragraphs(&self) -> Vec<&Range> {
        paragraphs(&self.children)
    }

    pub fn text(&self, ace: impl Fn(Ace) -> Option<char>) -> String {
        text(&self.children, &ace)
    }
}

/// The stories among `top` (a document's or an ICML file's top-level elements), in order.
pub fn stories(top: &[El]) -> Vec<Story> {
    top.iter().filter(|e| e.local() == "Story").map(Story::from_el).collect()
}
