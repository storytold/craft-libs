# Craft Archive

`craft-archive` holds the checks that must run *before* an archive or XML parser touches a file the
app did not write. It has no dependencies and no `unsafe`, and it parses nothing: a file it accepts
may still be malformed and be rejected by the parser afterwards, but a file it refuses is never fed
to a parser that could exhaust memory or the stack.

Two modules, one counter type. Each format keeps its own numbers; the crate holds none.

## `Budget` and `Exceeded`

A named running total. Charge it before allocating, and a breach says which limit was hit:

```rust
use craft_archive::Budget;

let mut total = Budget::new("document", 512 * 1024 * 1024);
total.charge(1024)?;                 // Ok: 1024 bytes charged
let left = total.remaining();        // what is left
# Ok::<(), craft_archive::Exceeded>(())
```

`zip::read_entry` charges a budget itself, so one budget covers a whole import and the totals are
bytes that really arrived, not bytes the archive claimed.

## `zip::preflight`, `zip::read_entry`

`zip::ZipArchive::new` reads the end records and then reads and indexes every central-directory
entry before a caller can count them, and a ZIP64 archive may declare billions. `preflight` runs
first, on the raw bytes, without allocating:

```rust
use craft_archive::zip::{self, Limits};

# fn example(bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
let limits = Limits { max_entries: 20_000, max_central_directory_bytes: 16 * 1024 * 1024, max_name_bytes: 1_024 };
let directory = zip::preflight(bytes, limits)?;
# let _ = directory;
# Ok(())
# }
```

It validates the single end-of-central-directory record that ends the file, the optional ZIP64
record and locator, the declared entry count and directory size against the limits, the directory
offset and size arithmetic, and every entry header in the directory. Split archives, prefixed
(self-extracting) archives and archives with more than one plausible end record are refused rather
than guessed at.

`zip::read_entry` then reads one entry against a per-entry limit and the remaining total budget. It
refuses a declared size over the limit, reserves a fixed small amount rather than the declared size,
charges only the bytes actually read, and reports more bytes than the bound as an overrun — what a
lying declared size or a decompression bomb looks like. It takes any `std::io::Read`, so a reader
from the `zip` crate and a raw reader both work.

## `xml::check`, `xml::Depth`

Recursive XML parsers consume stack per element, so deeply nested input aborts the process instead
of returning an error. `xml::check` measures the nesting in one iterative pass over the bytes
first, and finds the `<!DOCTYPE` so the application can apply its DTD policy:

```rust
use craft_archive::xml::{self, Dtd, Limits};

# fn example(bytes: &[u8]) -> Result<(), craft_archive::xml::Error> {
let limits = Limits { max_depth: 256, dtd: Dtd::Refuse };
xml::check(bytes, &limits)
# }
```

Comments, CDATA sections, processing instructions and quoted attribute values cannot fake a tag.
`Dtd` is `Refuse`, `DeclarationOnly` (a declaration without an internal subset) or
`InternalSubset { max_bytes }`. `xml::Depth` is the same counter for a reader that counts as it
parses, so both agree on where the limit is.

**`xml::check` reads UTF-8 bytes.** A document in any other encoding (a UTF-16 BOM is the tell) is
refused with `Error::UnsupportedEncoding` rather than silently admitted: decode to UTF-8 first and
check those bytes — the same bytes the downstream parser reads.

## What stays in the applications

Every number, and which parts are read or skipped. Entry-name rules and part allowlists. Whether a
breach fails the import or drops a part, and the wording of the user-facing message. The choice of
XML parser and its own node limit, and node, text and model budgets inside a parser. The
application's own tree depth limits (math, content types, styles). OPC relationships and content
types. Image-decode admission is not in this crate: its consumers and checks depend on each decoder.

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
