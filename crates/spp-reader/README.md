# texturecraft-import-reader

Reads projects of the reference app into plain Rust data: the file container, the reference app's records and its
layer model, and decompiles the compiled graphs a project embeds into graph text for TextureCraft's graph runtime. It
knows no application: the importer is built on it, and any other painting app can build its own importer on the same
reader.

Its dependencies are small pure-Rust decoders: `lz4_flex` (compressed records), `sevenz-rust2` (graph packages, which are 7z archives),
`hayro-jpeg2000` (version 12.2's image resources) and `zune-jpeg` (JPEGs inside compiled graphs). The crate contains no
code, data or resources of the reference app: everything is read from the user's own files at run time, from what those files
contain.

## What it reads

| Module | What |
|---|---|
| `container` | The generic formats: HDF5 (the project file itself), Qt QDataStream, byte cursors. Knows nothing of the reference app. |
| `decode` | The reference app's records: project settings, the resource table and images, the texture cache (the flattened channels the reference app saved), the mesh scene (`scene.assbin`, decoded into `SceneMesh`), mesh metadata and normalisation, cameras, and `document.bin` as a raw object tree. |
| `model` | The reference app's concepts built from `document.bin`: texture sets, channels, the layer tree (fill, paint, folder), per-channel blending, masks, effects (fills, filters, generators, levels, colour selection, anchors, polygon fills), sources (uniform, bitmap, procedural, reference), brushes and strokes, bake settings. Unknown classes are kept raw and counted, never dropped. |
| `project` | One opened project file with typed access to its parts. |
| `decompile` | A compiled graph to graph text (graph format 3 of the graph runtime): one node per compiled node, function subgraphs for the expressions, pattern programs for the fx-maps, stored images as embedded PNGs. Text only: the crate does not depend on the runtime, and the code-to-name tables of the compiled format live here and nowhere else. |
| `inspect` | Human-readable reports about a project, its document and its compiled graphs. |
| `blockers` | The shared vocabulary for what an importer can't evaluate yet (see below). |

## Example

```sh
cargo run -p texturecraft-import-reader --example layers -- path/to/project-file
```

```rust
use texturecraft_import_reader::decode::assbin::AssbinGeometry;
use texturecraft_import_reader::project::{ProjectFile, paths};

let project = ProjectFile::open(path)?;          // or ProjectFile::from_bytes(bytes)?
let mesh = AssbinGeometry::parse(&project.read(paths::SCENE)?)?; // world-space parts, glTF UVs
let (document, report) = project.model()?;                        // texture sets and their layer trees
for set in &document.texture_sets {
    println!("{}: {} layers", set.name, set.layers.len());
}
```

See `examples/layers.rs` for the whole program. `examples/import_inspect` is the developer tool
(`tree`, `doc`, `model`, `compiled-graphs`, `decompile`).

## Reading from bytes

`ProjectFile::from_bytes(Vec<u8>)` opens a project already in memory, `ProjectFile::from_source` one served by any
`container::hdf5::ReadAt` (`MemorySource` is the in-memory one). Opening from a path (`ProjectFile::open`,
`ProjectReport::build`) exists only off wasm (`cfg(not(target_arch = "wasm32"))`); the web build reads bytes. Graph packages are read from bytes in any case (`decode::graph_package::GraphPackage::read`).

## Blockers: a shared vocabulary

Projects are full of compiled graphs (generators, procedural fills, filters), brush alphas and
brush types that an importer can't evaluate until someone writes an equivalent. `blockers` gives each one a stable
key, so every importer, report and corpus census names the same thing the same way:

| Key | What blocks |
|---|---|
| `graph:<graph>` | a compiled graph, e.g. `graph:mg_dirt` |
| `brush-alpha:<name>` | strokes painted with a brush alpha the importer doesn't have |
| `brush:<what>` | strokes the importer's brush can't paint (particles, 3D paths, stencils, projection …) |
| `strokes:<what>` | strokes that were not replayed |
| `feature:<id>` | a project feature the importer doesn't model yet |
| `class:<class>` | a layer, effect or source class the reader doesn't know |

What an importer can *do* about a key (replace it, replay it, or keep the layer locked) is the importer's own
business; our answer is its support catalog (`crates/import/src/translate/catalog.rs`).

## File formats

`document.bin` comes in two layouts, picked by the header: tagged (up to format 78, 2023) and compact (format 92, the reference app
12.2; see `decode/document/compact.rs`). Version 12.2 also stores image resources as tiled JPEG 2000, decoded with the
pure-Rust `hayro-jpeg2000`.

## Robustness

The reader parses untrusted files. Malformed input is reported as an error (`ProjectError`, `DecodeError`); unknown
classes, fields and newer formats are kept raw and reported rather than guessed. The never-panic tests
(`src/robustness_tests.rs`) feed truncated, corrupted and random bytes into every public entry point.

## Tests

`cargo test -p texturecraft-import-reader` runs everything that needs no project files. The corpus tests run over real projects
when given them, and are ignored otherwise: `TC_IMPORT_CORPUS=<folders or project files> cargo test --release -p
texturecraft-import-reader -- --ignored`. Nothing read from a project is ever committed. `tests/fixtures/small.h5` is a
generic HDF5 file written by `tests/fixtures/make_fixture.c` (libhdf5) from a byte pattern; it holds no project data.

## Licence

MIT OR Apache-2.0, like the rest of the workspace.
