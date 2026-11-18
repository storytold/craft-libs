# craft-opc

Open Packaging Conventions (ECMA-376 Part 2): the ZIP container that DOCX, XLSX and PPTX files
share. It replaces the package layer each Craft app carried for its own format (WordCraft's
`docx::package`, GridCraft's `xlsx::package`, DeckCraft's `pptx::opc`).

What the parts hold stays with each format's crate. Parts are read with `zip` and XML with
`craft-xml`.

```rust
use craft_opc::{Limits, Package, PackageOut, RelsOut};

let mut out = PackageOut::default();
out.xml_part("xl/workbook.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml", b"<workbook/>".to_vec());
out.binary("xl/media/image1.png", "image/png", vec![0x89, b'P', b'N', b'G']);
let mut rels = RelsOut::default();
rels.add("officeDocument", "xl/workbook.xml", false);
out.rels("", &rels);
let bytes = out.finish().unwrap();

let limits = Limits::new(10_000, 64 << 20, 256 << 20);
let pkg = Package::open(&bytes, &limits).unwrap();
let main = pkg.rels("").first("officeDocument").map(|r| r.target.clone()).unwrap();
assert_eq!(main, "xl/workbook.xml");
assert_eq!(pkg.content_types().of("xl/media/image1.png"), Some("image/png"));
assert!(pkg.get("XL/Workbook.xml").is_some());
```

## Reading

Two readers, because the apps read in two ways:

| | `Package` | `LazyPackage` |
| --- | --- | --- |
| Inflates | every part, in `open` | a part when `read` asks for it |
| Borrows the file's bytes | no | yes |
| A part over a limit | refuses the package, or is left out (`Limits::oversized`) | is that read's error |
| A damaged entry | left out with a logged warning | is that read's error, and ends the budget (`read_optional_limited`: costs only its own allowance) |
| Came from | WordCraft (refuse), DeckCraft (leave out) | GridCraft |

Both look parts up without regard to case, leading `/` or `\` for `/`.

### Limits

`Limits` has no defaults: each format keeps its own numbers.

| Limit | Checked | Error |
| --- | --- | --- |
| `max_entries` | from the end records, before `zip` indexes the entries (`check_entry_count`), and again on the count `zip` found | `TooManyEntries`, `TooManyEndRecords` |
| `max_part` | on the declared size, then on what is inflated (a read stops one byte past it) | `PartTooLarge` |
| `max_total` | on the bytes kept | `PackageTooLarge` |
| `max_work` | on the bytes inflated, kept or not: an entry inflates in full before its checksum is compared, so entries that fail it would otherwise be free | `PackageTooLarge` |

`Limits::new` sets `max_work` to `max_total` and refuses oversized parts. At most 16 MiB is
reserved for a part before its bytes arrive, whatever size it declares.

`check_entry_count` is a bytes-only check of what the end of central directory records declare.
It does not validate the archive; `zip` does.

### Names and relationships

- `Package::canonical(name)` is the stored spelling. When entries differ only in case, they are
  one part: the spelling a relationship names is kept, else the first in byte order.
- `Rels`: `by_id`, `target`, `first(type)`, `all(type)`. A type is matched by its full URI or
  by its last segment, so `first("image")` finds the transitional and the strict spelling
  (`type_is`). The first relationship of a repeated id wins.
- `Rel::target` is a part name for an internal target and the URL for an external one;
  `target_raw` is what the file wrote. A target is looked up as written, then with `%hh`
  escapes decoded; when the package has the part, `target` is its stored spelling.
- `resolve(dir, target)` drops a fragment and cannot climb out of the package. `relative(from, to)`
  is its inverse.
- `ContentTypes::of(part)`: Override by part name, then Default by extension.

A missing or unreadable relationships or content types part reads as empty, with a logged
warning for an unreadable one.

## Writing

- `PackageOut`: `xml_part` (with an Override), `binary` (a Default for its extension, or an
  Override when the extension already has another type), `raw`, `rels`; `finish` writes
  `[Content_Types].xml` first and leaves out a second part under a name already written.
- `RelsOut`: `add` reuses the relationship already made for a type, target and mode; `add_new`
  never does. Ids are `rId1`, `rId2`…
- `ZipOut` and `zip_entries` for a writer that builds its own content types. Output is
  deterministic: deflate level 6, one fixed timestamp. Names with an already-compressed
  extension are stored (`compression_for`).

## Not here

- Flat OPC (`pkg:package` XML) and encrypted packages (a compound file holding
  `EncryptedPackage`): still with the apps.
- A full ZIP preflight (overlapping entries, lying offsets): `check_entry_count` covers the
  entry count only.
