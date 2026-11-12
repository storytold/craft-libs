# craft-penpot

A clean-room reader and writer for the Penpot 2.x export container: the `.penpot` archive
(`penpot/export-files`, version 1). Offline: no network, no converter, no `unsafe`.

This crate is the container layer. It validates the ZIP, checks the manifest, and hands out the
members as the JSON the file holds (`serde_json::Value`) and media objects as bytes; it writes
members back into an archive. What the shapes, components and tokens mean, and how they map onto
an application's document, stays with the caller.

## Provenance

Written by FigCraft's contributors (the offline Penpot import and export, first by Guillermo
Bueno) under FigCraft's clean-room rule: behaviour comes from public documentation and from
observing files, no code of another product is copied, and no GPL or AGPL code is used.
FigCraft's own notes say its tests do not establish compatibility with a live Penpot server;
that holds here too. Moved here from `figcraft/crates/format/src/penpot.rs` (the archive half)
and `penpot/embedded_fonts.rs`.

## The format

A ZIP of JSON members and blobs:

| Member | Holds |
|---|---|
| `manifest.json` | `type` (`penpot/export-files`), `version` (1), `files`: each with its `id` (a UUID) and `name` |
| `files/<file>.json` | the file's header: features, version, migrations, options |
| `files/<file>/pages/<page>.json` | a page: `id`, `name`, `index`, background, guides, flows |
| `files/<file>/pages/<page>/<shape>.json` | one shape; the root frame has the nil UUID (`ROOT_ID`) and lists its children in `shapes` |
| `files/<file>/components/<id>.json`, `colors/`, `typographies/` | library entries |
| `files/<file>/tokens.json` | design tokens |
| `files/<file>/media/<id>.json` | a media record: `mediaId` names the stored object, `mtype` its type |
| `objects/<id>.json`, `objects/<id>.<ext>` | a stored object's record and its bytes |

Penpot 1.x binary exports are a different format and are not read.

## Reading

```rust
let mut archive = craft_penpot::Archive::open(&bytes)?;
for file in archive.files.clone() {
    let header = archive.json(&format!("files/{}.json", file.id)).ok();
    for page in archive.pages(&file.id)? {                    // sorted by index
        let Some(page_id) = page.get("id").and_then(|v| v.as_str()) else { continue };
        let shapes = archive.page_shapes(&file.id, page_id)?; // id -> shape JSON
        let root = shapes.get(craft_penpot::ROOT_ID);
    }
    let colors = archive.uuid_members(&format!("files/{}/colors", file.id), 2048);
    let image = archive.media(&file.id, media_id, max_bytes);  // Option<Vec<u8>>
}
let fonts = archive.embedded_fonts();                          // counted by signature
archive.check()?;                                              // no budget was overrun
```

Members a caller treats as optional return errors it may drop; `check` then says whether a
budget ran out along the way, so exhaustion is never swallowed.

## Writing

```rust
let manifest = craft_penpot::manifest("my-app/1.0", &[(file_id, "Name", features)]);
let bytes = craft_penpot::write_archive([("manifest.json", manifest_bytes), (path, member_bytes)])?;
```

`write_archive` refuses paths `Archive::open` would refuse.

## Limits

Input is hostile. Nothing panics; every limit has a test.

| Limit | Value |
|---|---|
| The archive (`MAX_ARCHIVE`) | 512 MiB |
| ZIP members (`MAX_ENTRIES`) | 100 000 |
| Bytes unpacked, by declared sizes at open and by bytes actually read after (`MAX_DECOMPRESSED`) | 1 GiB |
| A JSON member (`MAX_JSON`) | 16 MiB |
| JSON values and keys, every member together, counted before parsing (`MAX_JSON_VALUES`) | 4 000 000 |
| Shape JSON for one page (`MAX_PAGE_SHAPES_BYTES`) | 128 MiB |
| Shapes on one page (`MAX_PAGE_SHAPES`) | 50 000 |
| Pages in one file (`MAX_PAGES`) | 500 |
| Files in the manifest (`MAX_FILES`) | 50 |
| A media object (`MAX_IMAGE`; `media` also takes the caller's own cap) | 128 MiB |
| JSON nesting | serde_json's 128 levels |

Duplicate member names (told by the directory's own member count, since the ZIP reader keeps one
member per name), absolute paths, `.` and `..` parts, backslashes and control characters
fail the archive. Ids are checked to be UUIDs before they are used in a member path.

## ZIP

The archive is read with the `zip` crate directly, with the bounds above applied to each member.
A shared bounded ZIP reader is planned (craft-libs #126); this crate moves to it when it lands.

## Typed members (`model`)

`model` types the members as the files name their fields: `Manifest`, `FileHeader`, `Page` (with
its `Guide`s and `Flow`s), `Shape` (geometry, `SelRect`, `Matrix`, points, `Fill`s with colours,
`Gradient`s and `ImageRef`s, `Stroke`s, `Shadow`s, `Blur`s, corner radii, constraints, flex and
grid layout fields, component links, applied tokens), and the library records `LibraryColor`,
`Typography`, `Component` and `Media`. `ShapeType`, `GradientType` and `BlendMode` keep values
they don't know (`Other`).

```rust
let page: craft_penpot::model::Page = archive.typed(&format!("files/{file}/pages/{page_id}.json"))?;
let shape: craft_penpot::model::Shape = archive.typed(&shape_path)?;
let bytes = serde_json::to_vec(&shape)?;                     // written back as the file had it
```

Every record keeps the fields it has no name for in `extra` and writes them back: a member read
and written loses only `null` fields, and whole numbers come back as floats. Kept as JSON: text
and path `content`, SVG attributes, interactions, display grids, grid tracks and cells, tokens
(`tokens.json`), and typography sizes (numbers or numeric strings in the files). A field of the
wrong type fails `typed`, naming the member; a reader that must skip bad fields one by one reads
`json` instead. `typed` reads within the same byte and value budgets as `json`. Shapes are flat
(children by id), so no record nests deeper than the JSON itself.

## Not here yet

Typed text and path `content`, interactions and tokens.
