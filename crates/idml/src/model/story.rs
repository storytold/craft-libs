//! Stories: paragraph and character ranges, their text, and what text holds (tables, notes,
//! anchored page items, hyperlink and change containers).

use craft_xml::{El, Local, Node};

use super::{PageItem, Values, tidy};

/// A `Story`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Story {
    pub values: Values,
    pub children: Vec<TextNode>,
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
    /// Text inside another element ([`CONTAINERS`]): a hyperlink's source, a tracked change, an
    /// XML element, a cross-reference's source, a footnote, a note.
    Container(Container),
    Table(Table),
    /// A page item anchored in the text.
    Item(PageItem),
    /// Anything else (`StoryPreference`, `HyperlinkTextDestination`, `TextVariableInstance`, …),
    /// as written.
    Other(El),
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

/// The children of `el` other than `<Properties>`, as text nodes.
fn nodes(el: &El) -> Vec<TextNode> {
    el.elements().filter(|e| !e.is(Local("Properties"))).map(TextNode::from_el).collect()
}

fn with_children(mut el: El, children: &[TextNode]) -> El {
    el.children.extend(children.iter().map(|c| Node::El(c.to_el())));
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
            TextNode::Container(c) => with_children(c.values.element(&c.name), &c.children),
            TextNode::Table(t) => t.to_el(),
            TextNode::Item(i) => i.to_el(),
            TextNode::Other(e) => e.clone(),
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
            TextNode::Paragraph(r) | TextNode::Character(r) => stack.extend(r.children.iter().rev()),
            TextNode::Container(c) if inline_container(&c.name) => stack.extend(c.children.iter().rev()),
            _ => {}
        }
    }
    out
}

impl Range {
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
