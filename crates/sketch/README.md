# craft-sketch

A clean-room reader for the `.sketch` container, for files saved by Sketch 48 and newer.
No `unsafe`.

This crate is the container layer. It opens the archive, applies the version cut, and hands out
`document.json`, `meta.json` and the pages as the JSON the file holds (`serde_json::Value`, every
object with its kind in `_class`) and the images as bytes. What the layers mean, and how they map
onto an application's document, stays with the caller.

## Provenance

Written by FigCraft's contributors (the `.sketch` import, mostly by Jindřich Šaršon), from the
public description of the format only, and files built for each case, checked against Sketch's
own renders and real files (Sketch 50, 63 and 2026). No code of Sketch or of any other product was copied, and no GPL, LGPL
or AGPL code was consulted. Moved here from `figcraft/crates/format/src/sketch.rs` (the archive
half).

## The format

- A `.sketch` file is a ZIP archive: `document.json` (the pages in order as `_ref`s to
  `pages/<id>`, shared styles, swatches, library symbols copied into the document),
  `meta.json` (`appVersion`, the format `version`), `user.json`, one `pages/<id>.json` per page
  (the layer tree), `images/<name>` and, in newer files, `fonts/`.
- Before version 43 a Sketch file was a SQLite database; until 47 text was a binary property
  list inside the JSON. Both are refused as too old (`Error::TooOld`, with a message saying to
  save the file again in a newer Sketch).
- `meta.json` versions above 139 (Sketch 83, the newest checked against real files) open with a
  warning.

## Reading

```rust
if craft_sketch::is_sketch(&bytes) {
    let archive = craft_sketch::Archive::open(&bytes)?;
    for (id, page) in &archive.pages {                       // in the document's order
        let layers = page.get("layers");
    }
    let styles = archive.document.get("layerStyles");
    let image = archive.image("images/0a1b….png");            // Option<Vec<u8>>
    let fonts = craft_sketch::embedded_font_families(&archive.document);
    for warning in &archive.warnings { /* pages skipped, a newer format than tested */ }
}
```

A `document.json` that can't be read (damaged, nested too deep) loses the shared styles, not the
pages: they are then taken in archive order, with a warning. A page that can't be read is
skipped with a warning; the others open.

## Limits

Input is hostile. Nothing panics; every limit has a test.

| Limit | Value |
|---|---|
| `meta.json`, `document.json` (`MAX_JSON_MEMBER`) | 512 MiB |
| A page (`Limits::max_page`) | 512 MiB |
| JSON read in all (`MAX_JSON_TOTAL`) | 2 GiB (512 MiB on 32-bit targets) |
| Unpacked size per compressed byte (`MAX_RATIO`), past a 16 MiB floor (`MIN_ALLOWANCE`) | 100 |
| Pages (`MAX_PAGES`) | 1000 |
| An image (`Limits::max_image`) | 256 MiB |
| Image bytes in all (`Limits::max_image_total`) | 1 GiB (256 MiB on 32-bit targets) |
| A page id or image name | 128 characters of letters, digits, `-` and `_` (an image may have one extension) |
| JSON nesting | serde_json's 128 levels |

A member is refused before unpacking when its declared size is past its allowance, and again if
it unpacks past it. A read that fails is charged in full, and a page listed twice is read once,
so a page list can't repeat an unpacking within the budget.

## ZIP

The archive is read with the `zip` crate directly, with the bounds above applied to each member.
A shared bounded ZIP reader is planned (craft-libs #126); this crate moves to it when it lands.

## Not here yet

- A typed model of the document and the layers (`_class` objects: pages, artboards, groups,
  shape paths, text with attributed strings, symbols and overrides, styles). FigCraft maps the
  JSON straight onto its own import shape today.
- Writing.
