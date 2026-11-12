# craft-idml

IDML (InDesign Markup Language) packages, and InDesign Tagged Text. No `unsafe`.

An IDML package is a ZIP whose first entry is an uncompressed `mimetype`, followed by
`designmap.xml` (the root `Document` element, which includes the other parts) and the
`Resources/`, `MasterSpreads/`, `Spreads/` and `Stories/` parts.

Implemented clean-room from the public IDML File Format Specification and from inspecting IDML
packages exported from synthetic documents.

This crate is the layer between the bytes and IDML's own element tree, with a typed model of
the layout, the stories and the resources (`model`): a caller reads it and maps it onto its own
document. InCopy Markup (ICML), one
story as one XML document, is read and written too (`icml`).

## Reading

```rust
# fn main() -> craft_idml::Result<()> {
# let bytes: Vec<u8> = Vec::new();
# if bytes.is_empty() { return Ok(()); }
use craft_idml::{Local, Props};

let mut package = craft_idml::Package::open(&bytes)?;
let doc = package.document()?;          // designmap.xml with its includes in place
for story in doc.top.iter().filter(|e| e.name == "Story") {
    for range in story.find_all(Local("CharacterStyleRange")) {
        let size = range.num("PointSize");          // attribute or <Properties> child
        let font = range.value("AppliedFont").and_then(|f| craft_idml::value::font_family(&f));
    }
}
for line in &doc.warnings { /* a part designmap.xml lists and the package lacks */ }
# Ok(()) }
```

- `Package::part` finds a part by its exact name, then leniently: writers differ in
  percent-encoding, case, path separators and a leading `./` or `/`.
- `Package::document` refuses a package whose `mimetype` says something else, that has no
  `designmap.xml`, or whose root is not `Document`. An included part that is missing is a
  warning; one that cannot be read is an error.
- Elements are `craft_xml::El`, names as written (`idPkg:Story`), text exact. Processing
  instructions inside elements are kept (`<?ACE 18?>` is how InDesign writes special
  characters). Parts are read strictly: one that is cut off is an error, not half a part.
- `Props` reads a property in either spelling: an attribute, or a child of `<Properties>`.

### Limits

| `Limits` field | Default | Past it |
|---|---|---|
| `max_entries` | 100,000 | `Error::TooManyEntries` |
| `max_part_bytes` | 512 MiB | `Error::TooLarge` |
| `max_total_bytes` (all parts read from one package) | 2 GiB | `Error::TooLarge` |
| `xml.max_depth` | 256 | `Error::Part` |
| `xml.max_nodes`, `xml.max_attrs_total` (one part) | 8,000,000 and 32,000,000 | `Error::Part` |

The ZIP layer is craft-zip: the declared entry count is checked before the archive is indexed, and
sizes are checked against what a part really expands to, not what the archive declares.

## Writing

```rust
use craft_idml::{El, part_root, part_xml, designmap_xml, write_package, prop, with_props};

let range = with_props(El::new("CharacterStyleRange").with_attr("PointSize", "9"), vec![prop("Leading", "unit", "14")]);
let story = part_root("Story").with_child(El::new("Story").with_attr("Self", "u1").with_child(range));
let designmap = El::new("Document")
    .with_attr("xmlns:idPkg", craft_idml::PACKAGING_NS)
    .with_attr("DOMVersion", craft_idml::DOM_VERSION)
    .with_child(El::new("idPkg:Story").with_attr("src", "Stories/Story_u1.xml"));
let bytes = write_package(&[
    ("META-INF/container.xml", craft_idml::CONTAINER_XML.as_bytes().to_vec()),
    ("designmap.xml", designmap_xml(&designmap)),
    ("Stories/Story_u1.xml", part_xml(&story)),
])?;
assert!(craft_idml::is_idml(&bytes));
# Ok::<(), craft_idml::Error>(())
```

Parts are written with an element that has only element children one child per line, and any
other element on one line, so story text is never changed by indentation.

## The layout model

```rust
# fn main() -> craft_idml::Result<()> {
# let bytes: Vec<u8> = Vec::new();
# if bytes.is_empty() { return Ok(()); }
use craft_idml::model::{apply, spreads};

let doc = craft_idml::Package::open(&bytes)?.document()?;
for spread in spreads(&doc) {
    for page in spread.pages() {
        let _bounds = page.geometric_bounds();      // top left bottom right
        let _parent = page.applied_master();
    }
    for (item, to_spread) in spread.all_items() {  // nested items too, with their transform
        let outline = item.geometry();             // subpaths of anchor and direction points
        if let Some([top, left, _, _]) = outline.bounds() {
            let _corner = apply(&to_spread, [left, top]);
        }
        for graphic in item.graphics() {
            let _file = graphic.link().and_then(|l| l.uri());
            let _embedded = graphic.contents();
        }
    }
}
# Ok(()) }
```

- `Spread` (and parent spreads, `master`), `Page`, `Guide`, `PageItem` (`ItemKind`: text frames,
  rectangles, ovals, polygons, lines, groups, buttons, EPS text, and other page item elements by
  name), `Graphic` (`GraphicKind`: images, EPS, PDF, imported pages, WMF, PICT, SVG) and `Link`.
  Children keep their document order: page items stack back to front.
- Each node keeps its attributes and `<Properties>` as written (`Values`) and reads them through
  accessors: `item_transform`, `geometric_bounds`, `applied_master`, `override_list`,
  `parent_story`, `next_text_frame`, `graphic_bounds`, `contents`, and `num`, `boolean`,
  `value` for any other property. Anything the model does not type is kept as XML where it
  stood, without the part's indentation (`tidy`). `to_el` writes a node back: what is read and
  written again reads the same.
- `PathGeometry` (`geometry`, `set_geometry`) holds subpaths of points. A point without a usable
  anchor is left out, a missing direction point is the anchor, and a subpath holding `NaN` or an
  infinity is left out whole. At most `MAX_PATH_POINTS` (1,000,000) points are kept.
- `Spread::all_items` walks nested groups without recursion, giving each item's transform into
  spread coordinates (`concat` of the transforms on the way).

## Stories

```rust
use craft_idml::model::{Ace, TextNode, stories};

# fn main() -> Result<(), craft_xml::Error> {
let part = craft_idml::parse_part(br#"<idPkg:Story><Story Self="u1"><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body">
  <CharacterStyleRange PointSize="9"><Content>Page <?ACE 18?></Content><Br/></CharacterStyleRange>
</ParagraphStyleRange></Story></idPkg:Story>"#)?;
let story = stories(&part.elements().cloned().collect::<Vec<_>>()).remove(0);
let body = story.paragraphs()[0];
assert_eq!(body.paragraph_style(), Some("ParagraphStyle/Body"));
assert_eq!(body.characters()[0].values.num("PointSize"), Some(9.0));
assert_eq!(story.text(|a| (a == Ace::PageNumber).then_some('#')), "Page #\n");
# Ok(()) }
```

- `Story`, its `TextNode`s in document order: paragraph and character ranges (`Range`),
  `Content` (text, and the special characters InDesign writes as `<?ACE n?>`: `Ace`), `Br`,
  containers of text (`HyperlinkTextSource`, `Change`, `XMLElement`, `CrossReferenceSource`,
  `Footnote`, `Note`, `EndnoteRange`), `Table` (rows, columns, and `Cell`s holding text), and
  page items anchored in the text (the layout model's `PageItem`).
- `text` gives a story's, range's, cell's or note's text: content, `\n` for each break, and a
  character for each special character the caller names. A footnote's, note's or table's text
  is its own, not the text around it; a hyperlink's or a change's is.
- Content keeps its whitespace exactly; elements the model does not type are kept as written.
  Containers nest as deep as the XML reader allows (256), and text is gathered without
  recursion.

## Resources

- `style_groups` gives the root style groups (`RootParagraphStyleGroup` and the character,
  object, cell and table ones) as trees of `StyleGroup` and `Style`, written back exactly.
  `StyleGroup::styles` lists every style with the names of the groups it is in; `Style` has
  `based_on`, `next_style` and its other children (`other`: an object style's
  `TextFramePreference`, its effect settings).
- `swatches` gives `Color`, `Tint`, `Gradient` (with `stops`), `MixedInk`, `MixedInkGroup` and the
  None `Swatch`: `spot`, `space`, `color_value`, `base_color`, `tint_value`, `radial`.
- `stroke_styles` (dashed with `dash_array`, dotted, striped with `stripe_array`) and
  `color_groups` (with `members`).
- `preference` finds a preference element (`DocumentPreference`, `GridPreference`, …) and
  `named` every element of a name (`Ink`, `Layer`, `Section`, `Condition`, `NumberingList`).

## ICML

`icml::is_icml` sniffs the `SnippetType="InCopyInterchange"` instruction in the first 4 KiB.
`icml::read_icml` reads the file as a package part is read, at most `MAX_ICML_BYTES` (512 MiB);
a root that is not `Document` or a file without a `Story` is refused, and a reference to a
package part (an ICML file has none) is left out with a warning. `Icml::stories` gives its
stories; which one to place (usually the first no text frame of the file holds) is the
caller's choice. `icml::icml_xml` writes a `Document` with the two `<?aid?>` instructions that
mark the file.

## Values

`craft_idml::value`: `num` and `point_str` (numbers as IDML writes them), `nums`, `matrix`,
`point`, `tint`, `escape_id` / `unescape_id` / `uid` (`Self` ids), `style_name_in` /
`style_name_out` / `style_self` (style groups and built-in names), `font_family`
(`AppliedFont`), `xmp_value` (dates in `META-INF/metadata.xml`), `base64_encode` /
`base64_decode` (embedded `Contents`), `uri_to_path` / `path_to_uri` (link URIs).

## Tagged Text

`craft_idml::tagged` reads and writes InDesign Tagged Text: plain text with `<Tag:value>` markup.

```rust
use craft_idml::tagged;

let file = b"<ASCII-WIN>\r\n<Version:8>\r\n<ParaStyle:Body>A <CharStyle:Em>word<CharStyle:>";
assert!(tagged::is_tagged(file));
let text = tagged::parse(file);
assert_eq!(text.paragraphs[0].style, "Body");
assert_eq!(text.paragraphs[0].runs[1].format.style.as_deref(), Some("Em"));
assert_eq!(tagged::parse(tagged::write(&text.paragraphs).as_bytes()).paragraphs, text.paragraphs);
```

Paragraph styles, character styles, font family, font style and size are kept; other tags are
counted (`skipped_tags`) and skipped. ASCII, UTF-8 and UTF-16 (by byte-order mark) are read;
ASCII is written, other characters as `<0xXXXX>` UTF-16 code units. Style and font names are cut
to `MAX_NAME_CHARS` (256). Every input reads as something.

## Where it came from

DesignCraft's `crates/idml` (the package reader and writer, the `<Properties>` accessors, the
value codecs) and `crates/textimport/src/tagged.rs`. DesignCraft's own XML tree is replaced by
craft-xml.
