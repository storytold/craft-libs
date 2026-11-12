# craft-fig

A clean-room reader and writer for the `.fig` container: the local copies Figma Design saves
(`.fig`) and FigJam whiteboards (`.jam`, the same archive with another canvas header). No `unsafe`.

This crate is the wire layer. It opens the archive, decompresses the canvas, hands the schema and
the message to [`craft-kiwi`](../kiwi/README.md), finds images, and reads and writes the two blob
encodings (vector networks and outline geometry). What the decoded node changes mean, and how
they map onto an application's document, stays with the caller: the message is a `kiwi::Kv` tree
read by field name, in the file's own terms.

## Provenance

Written by FigCraft's contributors, from public format knowledge only: public descriptions of the
container, the kiwi format's published specification, and black-box observation of files made
from synthetic documents. Nothing inside the incumbent's application bundle, web bundle or
network traffic was read, disassembled or copied, and no GPL, LGPL or AGPL code was consulted.
Written files were checked by opening them in other software. Moved here from
`figcraft/crates/format/src/fig.rs`, `fig/network.rs`, `fig/export.rs` (the container half) and
`fig/map/layer.rs` (the geometry blob).

## The format

- A `.fig` file is a ZIP archive: `canvas.fig` (the document), `meta.json` (`file_name`,
  `client_meta.background_color`, `client_meta.thumbnail_size`), `thumbnail.png` and
  `images/<hash>` (the hash in hex, no extension). A bare `canvas.fig` is a file too: the
  clipboard carries one.
- `canvas.fig` is an 8-byte magic (`fig-kiwi`; `fig-jam.` for a whiteboard), a little-endian
  `u32` version, then length-prefixed chunks, each compressed with raw deflate, zlib or zstd. The
  first chunk is a kiwi binary schema, the second a message encoded with it.
- The message's `nodeChanges` list the layers (each with its id, its parent's id and its position
  among the siblings). Its `blobs` hold vector networks (`vectorData.vectorNetworkBlob`), outline
  geometry (`fillGeometry[].commandsBlob`) and, in files written by plugins and other tools, the
  bytes of images (`image.dataBlob`).

## Reading

```rust
let archive = craft_fig::Archive::open(&bytes)?;             // a zip or a bare canvas
let packed = archive.canvas.len();
let (schema, message) = craft_fig::decode_canvas(&archive.canvas)?;
let root = schema.find("Message").ok_or("no Message definition")?;
let msg = schema.decode_message_within(root, &message, craft_fig::value_budget(packed))?;
let embedded = craft_fig::embedded_copies(&msg, craft_fig::Limits::default().max_image);
let image = archive.image("0a1b…").or_else(|| embedded.get("0a1b…").map(|b| b.to_vec()));
let network = craft_fig::network::decode(blob_bytes);        // Option<Network>
let outline = craft_fig::path::decode(blob_bytes);           // Option<Vec<Command>>
```

## Writing

```rust
let canvas = craft_fig::encode_canvas(&kiwi::encode::schema(&schema), &kiwi::encode::message(&schema, root, &msg)?)?;
let bytes = craft_fig::write_archive(&craft_fig::Package {
    file_name: "Untitled", background: [1.0, 1.0, 1.0, 1.0], canvas: &canvas, images: &images, thumbnail: None,
})?;
```

## Limits

Input is hostile. Nothing panics; every limit has a test.

| Limit | Value |
|---|---|
| `canvas.fig` read or written (`MAX_CANVAS`) | 256 MiB |
| A decompressed chunk (`MAX_CHUNK`) | 512 MiB |
| The decompressed schema chunk (`MAX_SCHEMA`) | 16 MiB |
| Chunks read (`MAX_CHUNKS`) | 16 |
| `meta.json` | 1 MiB |
| Values decoded from the message (`value_budget`) | 4 per compressed canvas byte, at least 20 million, at most 128 million (32 million on 32-bit targets) |
| An image (`Limits::max_image`) | 256 MiB |
| Image bytes read in all, failed reads included (`Limits::max_image_total`) | 4 GiB (1 GiB on 32-bit targets) |
| An image hash (`MAX_IMAGE_HASH`) | 64 bytes |
| Vertices, segments, regions or loop entries in a vector network (`network::MAX_ITEMS`) | 200 000 |
| Commands in a geometry blob (`path::MAX_COMMANDS`) | 200 000 |
| A written file (canvas and images) | 2 GiB |

The value budget follows the compressed size, which the file itself bounds, not what it unpacks
to: a small file can't claim a large tree however far its chunks inflate.

## ZIP

The archive is read with the `zip` crate directly, with the bounds above applied to each member.
A shared bounded ZIP reader is planned (craft-libs #126); this crate moves to it when it lands.

## The message as REST file JSON (`rest`)

`rest::to_rest` maps a decoded message onto the REST API file JSON that
[`craft-figma-rest`](../figma-rest/README.md) models: pages, the layer tree, geometry, paints,
strokes, effects, text with styled runs, auto layout and grids, components, component sets and
instances (expanded from their main, with overrides and property values applied), shared styles,
variables, prototype interactions and flows. Layer kinds the REST shape doesn't have (sticky
notes, connectors, tables, …) come out as the closest frames, text and vectors. What isn't
understood is skipped with a warning.

```rust
let mapped = craft_fig::rest::to_rest(&msg, "Untitled", &craft_fig::rest::Options {
    max_nodes: craft_fig::rest::layer_budget(packed),
    ..Default::default()
});
let file = craft_figma_rest::File::from_value(mapped.json, usize::MAX)?;  // or read the JSON
// mapped.networks: the vector networks layers name by `vectorNetworkRef`
// mapped.warnings: what was skipped or approximated
```

- **Geometry the caller owns.** A connector's route between its ends, and whether a text path can
  be measured, come from `Options::geometry` (the `rest::Geometry` trait): an editor routes
  imported connectors as it routes its own. `rest::Straight` draws straight lines and accepts
  every text path.
- **Extensions.** Besides the public fields, the JSON carries the extensions craft-figma-rest
  documents and private `_fig*` handoffs (each layer's provenance, cached library definitions,
  mains the layer cap kept out, region paints, saved line breaks, text paths) for an importer
  that wants them.
- **Budgets.** At most `Options::max_nodes` layers are produced or scanned, instance expansion
  included (`rest::layer_budget` follows the compressed canvas: 8 bytes a layer, 200 000 to
  2 000 000); nesting is followed 256 levels deep (the walk recurses once per level in small frames: an unoptimised build maps past that limit on 512 KiB, half the 1 MiB stack of a WebAssembly build, and a test at the limit runs on a 1 MiB thread); conditional actions 16, expressions 32; grid
  tracks 1 000; region paint elements 200 000 per file; scale factors between 0.001 and 1000.

The mapping was written in FigCraft (`crates/format/src/fig/map`) by its contributors, from the
same public format knowledge as the rest of this crate.
