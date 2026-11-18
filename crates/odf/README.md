# craft-odf

OpenDocument packages (ODF 1.3 Part 2): what every OpenDocument file has in common, whatever it
holds. Text documents, spreadsheets, presentations and drawings all sit in the same container;
this crate reads and writes that container and nothing inside it.

| Module | What it does |
| --- | --- |
| `Package`, `PackageWriter` | The ZIP container: the `mimetype` file, the manifest, the files, within `Limits`. |
| `Manifest` | `META-INF/manifest.xml`: each file's media type, and how an encrypted file is encrypted. Read by namespace, whatever prefix the file uses; `parse_strict` / `Package::manifest_strict` refuse a damaged one instead of reading what they can. |
| `crypt` | Password-protected packages: AES-256-CBC, SHA-256 start key, PBKDF2 (HMAC-SHA-1), SHA-256 or SHA-1 checksums. A package in, a package out. |
| `flat` | Flat single-file documents (`.fodt`, `.fods`, `.fodp`, `.fodg`): one `office:document` taken apart into a package's files, and a package put together into one. |
| `zip_guard` | Refuses a ZIP whose end record declares too many entries, before `zip` reserves room for them. |

```rust
use craft_odf::{Limits, Package, PackageWriter, crypt, flat, mime};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut w = PackageWriter::new(mime::SPREADSHEET);
    w.file("content.xml", "text/xml", b"<office:document-content/>");
    w.stored("Pictures/chart.png", "image/png", b"...");
    let plain = w.finish()?;

    let locked = crypt::encrypt(&plain, "password")?;
    assert!(crypt::is_encrypted(&locked));
    let opened = crypt::decrypt(&locked, "password")?;

    let mut p = Package::open(&opened, &Limits::DEFAULT)?;
    assert_eq!(p.mimetype().as_deref(), Some(mime::SPREADSHEET));
    assert_eq!(p.read("content.xml")?.as_deref(), Some(b"<office:document-content/>".as_slice()));

    let one_file = flat::from_package(&opened, &Limits::DEFAULT)?;
    let again = flat::to_package(&one_file, mime::SPREADSHEET, &Limits::DEFAULT)?;
    assert_eq!(Package::open(&again, &Limits::DEFAULT)?.mimetype().as_deref(), Some(mime::SPREADSHEET));
    Ok(())
}
```

## Text documents (feature `text`)

`craft_odf::text` reads and writes OpenDocument Text (`.odt`, `.ott`, `.fodt`) through the
[craft-flow](../flow/README.md) model and a craft-wordmodel `Document`: paragraphs and headings
with their formatting and styles, lists and outline numbering, tables, pictures, shapes, text
boxes and groups, charts, equations (as formula objects, through craft-math's MathML), embedded
objects, fields, notes, comments, bookmarks, indexes, forms, sections and page setup with headers
and footers, tracked changes, metadata and settings. What a document holds that the model does not
is kept as read and written back (`craft_wordmodel::kept`). Packages are read and written with
this crate's `Package` and `PackageWriter`, password protection is `crypt`, flat files are `flat`.

## Limits

Input is hostile. Nothing here panics on any bytes.

| Limit | Default | Where |
| --- | --- | --- |
| Entries a ZIP may declare | 20,000 | `Limits::max_entries`, checked on the raw bytes before `zip` parses the directory |
| Bytes one file inflates to | 256 MiB | `Limits::max_entry`; the size a file declares is not trusted |
| Bytes read from one package | 2 GiB | `Limits::max_total` |
| `mimetype` file | 255 bytes | longer is no media type |
| Manifest entries | 20,000 | the rest are dropped |
| PBKDF2 iterations for one file | 10,000,000 | more is refused |
| Key-derivation work for a package | 200,000,000 iteration-blocks | a thousand files as LibreOffice writes them |
| Files decrypted | 10,000 | |
| Pictures and objects taken out of a flat document | 10,000 | the rest are dropped |
| Nesting of one element name in a flat document | 512 | deeper is treated as unclosed |

## Not supported

- Blowfish CFB (the default before ODF 1.2) and OpenPGP encryption: such packages are refused with
  a message that names the algorithm.
- Digital signatures (`META-INF/documentsignatures.xml`) are not checked or written; the file is
  kept like any other.
- The meaning of `content.xml`, `styles.xml`, `meta.xml` and `settings.xml`.

## Provenance

Moved from WordCraft (`crates/formats/src/odt/crypt.rs`, `odt/flat.rs`, the package reading and
writing in `odt/import.rs` and `odt/package.rs`, and `crates/doc/src/zip_guard.rs`), with
GridCraft's namespace-aware manifest reading. Written from the OASIS OpenDocument 1.3
specification, W3C XML Encryption (padding), RFC 2898 (PBKDF2) and PKWARE's APPNOTE.TXT.
