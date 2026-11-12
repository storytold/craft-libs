# craft-figma-rest

A typed model of the Figma REST API file JSON: the response of `GET /v1/files/:key` (a `document`
with `CANVAS` pages, plus `components`, `componentSets` and `styles`) and of
`GET /v1/files/:key/nodes`. Read and written with serde. No `unsafe`.

This shape is the neutral model for design files in craft-libs: the `.fig` reader
([`craft-fig`](../fig/README.md)) and the `.sketch` reader ([`craft-sketch`](../sketch/README.md))
map what they read onto it, and an application's importer takes it. It is the file's own shape,
named as the API names it, and knows nothing of any application's document.

## Provenance

Written from the published REST API reference only (the documented node types and their
properties). No code or schema file of the API's owner or of any other product was copied.

## The model

- `File`: `name`, `lastModified`, `version`, `schemaVersion`, `document`, `components`,
  `componentSets`, `styles`. `FileNodes` / `FileNode`: the `nodes` response.
- `Node`: a layer's `props` (`NodeProps`, read through the node: `node.fills`) and its `children`.
  One record for every node type, since the API's types share most properties.
- `NodeProps` names: identity (`id`, `name`, `type`, `visible`, `locked`), geometry
  (`absoluteBoundingBox`, `absoluteRenderBounds`, `relativeTransform`, `size`, `rotation`,
  `constraints`), appearance (`fills`, `strokes` and the stroke properties, `fillGeometry`,
  `strokeGeometry`, `cornerRadius`, `rectangleCornerRadii`, `cornerSmoothing`, `effects`,
  `opacity`, `blendMode`, `isMask`, `maskType`), auto layout (`layoutMode` and the padding,
  spacing, sizing and alignment properties), `layoutGrids`, `exportSettings`, `styles`, text
  (`characters`, `style`, `characterStyleOverrides`, `styleOverrideTable`, `lineTypes`,
  `lineIndentations`), `componentId`, `booleanOperation`, `arcData`.
- `Paint`, `ColorStop`, `Effect`, `TypeStyle`, `LayoutGrid`, `LayoutConstraint`, `ExportSetting`,
  `Path`, `ArcData`, `Color`, `Vector`, `Rectangle`, `Transform`, and the metadata records
  `Component`, `ComponentSet`, `Style`.
- `NodeType`, `PaintType`, `EffectType`, `BlendMode`, `StyleType`: the values known here and
  `Other(String)` for the rest. Other enumerations are strings, with their values in the field's
  documentation.

Every field is optional: the API leaves out what has its default.

### Not typed yet

Kept in `extra` (see below), as JSON: prototyping (`interactions`, `flowStartingPoints`,
`prototypeDevice`, transitions), variables (`boundVariables`, `explicitVariableModes`, the local
variables response), component properties (`componentPropertyDefinitions`,
`componentProperties`, `componentPropertyReferences`, `overrides`) and grid auto layout tracks.

## Nothing is lost

Every record has `extra`: the fields the model has no name for, in the JSON they came in, written
back on output. A file read and written keeps properties the API added after this model was
written. Two things change on a round trip: a field that was `null` is left out, and a whole
number written as `10` comes back as `10.0`.

## Extensions

Fields that are not in the REST API, added by the readers that map other files onto this shape so
that nothing those files hold is lost (`ext` documents each):

| Field | On | Holds |
|---|---|---|
| `vectorNetwork` | a `VECTOR` node | the editable network (`ext::VectorNetwork`, typed on `NodeProps`): vertices with corner radii, segments with tangents, regions with loops and winding |
| `vectorNetworkRef` | a `VECTOR` node | an index into a list of networks handed over beside the JSON |
| `pointCount`, `innerRadius` | `STAR`, `REGULAR_POLYGON` | points or sides; the star's inner radius as a fraction |
| `_letterSpacingPercent` | a text style | letter spacing as a percentage of the font size |

A consumer that doesn't know an extension ignores it; the public fields still describe the layer.
Fields starting with `_fig` are private handoffs between one reader and one importer and are not
part of this contract; they are kept like any unknown field.

## For readers

`Warnings` collects a reader's notes (each once, with a count, at most 200) and `style_name`
spells a font style the way this shape names it ("SemiBold" → "Semi Bold"), so the readers that
build this shape report and name things alike.

## Reading and writing

```rust
let file = craft_figma_rest::File::from_slice(&bytes)?;          // or from_value(value, max_nodes)
for node in file.document.iter().flat_map(|d| d.walk()) {         // parents first, no recursion
    if node.kind == Some(craft_figma_rest::NodeType::Text) {
        let text = node.characters.as_deref().unwrap_or_default();
    }
}
let json = file.to_value()?;
```

Building one:

```rust
use craft_figma_rest::{Node, NodeProps, NodeType};
let rect = Node::new(NodeProps { id: Some("1:1".into()), kind: Some(NodeType::Rectangle), ..Default::default() });
let page = Node::with_children(NodeProps { kind: Some(NodeType::Canvas), ..Default::default() }, vec![rect]);
```

## Limits

Input is hostile. Nothing panics; every limit has a test.

| Limit | Value |
|---|---|
| JSON nesting (`MAX_DEPTH`): about 500 layers inside one another | 1024 levels |
| Nodes (`from_value` takes the limit; `from_slice` uses `DEFAULT_MAX_NODES`) | 200 000 |
| JSON text nesting (`from_slice`) | serde_json's 128 levels |

Both limits are checked on the JSON, without recursion, before any typed value is built.
`from_value` and `to_value` take the layer tree apart and put it together level by level, so a
tree at the depth limit is read, written and dropped within a 2 MiB stack (tested). The `Serialize`
and `Deserialize` implementations go through `serde_json::Value` as well, but their serializer
recurses once per level: use `to_value` / `from_value` for deep trees.

A field of the wrong type fails the read, and the error names the type expected. A reader that
must skip bad fields one by one (an importer taking files from anywhere) reads the
`serde_json::Value` instead.
