# craft-zip

ZIP archives from untrusted files, read within limits and written. No `unsafe`.

The [`zip`](https://crates.io/crates/zip) crate does the decoding. This crate is what every
reader of a ZIP-based format (OOXML, OpenDocument, EPUB, IDML, design-tool archives) otherwise
writes again around it. Written from the ZIP specification (PKWARE APPNOTE.TXT).

Every limit is the caller's number: the crate has no defaults. Entry-name rules (which names
are safe, which spellings are one part) stay with the format that knows them.

## Reading

```rust
use craft_zip::{Archive, Compression, Limits, Writer};

# let mut w = Writer::new();
# w.add("mimetype", b"application/epub+zip", Compression::Stored)?;
# w.add("content.xml", b"<a/>", Compression::Deflated)?;
# let bytes = w.finish()?;
let mut zip = Archive::open(&bytes, Limits { max_entries: 20_000, max_total_bytes: 512 << 20 })?;
if zip.has_duplicate_names() {
    // Two entries share a name; a format that must not guess which is real refuses here.
}
let part = zip.read("content.xml", 64 << 20)?;       // Ok(None) when there is no such entry
assert_eq!(part.as_deref(), Some(&b"<a/>"[..]));
assert_eq!(zip.budget().used(), 4);
# Ok::<(), craft_zip::Error>(())
```

- `Archive::open` runs `check_entry_count` first. `zip` reserves room for, and parses, as many
  central directory headers as the end record declares before its caller can count them, and a
  ZIP64 end record can declare millions. The check finds the end records the way `zip` does and
  refuses on the raw bytes.
- `Archive::read` refuses an entry that declares more than `max`, that expands past what it
  declares, or that does not fit in what is left of `max_total_bytes`. At most 64 KiB is
  reserved before bytes arrive, whatever the entry declares. Only a read that succeeds is
  charged.
- `has_duplicate_names`: `zip` keeps one entry per name, so a central directory with more
  headers than `zip` has entries repeats a name.
- `read_head(name, n)` reads the first `n` bytes only (a signature), charged to the total.
- `compressed_size` with `declared_size` gives the compression ratio an entry claims, for a
  format that limits it.
- A clone shares the directory and the bytes and keeps its own count, for reading behind `&self`.
- `budget_mut` is for a caller with its own charging rule (charging a failed read, say).
- `Error::is_limit` tells a limit from a broken archive.

## Writing

`Writer::add(name, data, Compression::Stored | Deflated | Level(n))`, then `finish()`. Entries
keep their order (a stored `mimetype` goes first by being added first). Every entry has the same
1980 timestamp, so equal input gives equal bytes. An entry of 2 GiB or more is written in the
ZIP64 form. A repeated name is an error.

## Strict preflight

`preflight::preflight(bytes, limits)` checks the whole structure on the raw bytes without
allocating: exactly one end record may span the end of the file, the ZIP64 pair must agree with
it, the directory must sit exactly before the end records and be filled exactly by its entries,
single disk only, with limits on entries, directory bytes and name length. It is stricter than
`zip` (it refuses archives with data before them and archives with two plausible end records),
so `Archive::open` does not run it: a caller that wants nothing ambiguous to reach a ZIP reader
calls it first.

`preflight::read_entry(reader, declared, max, &mut budget)` is the bounded read on any
`std::io::Read`; `Budget` is a named limit with a running total.

## Where it came from

`preflight`, `read_entry` and `Budget`: the ZIP half of the craft-archive proposal (#126, #129).
`check_entry_count`: craft-opc's `guard`. Duplicate detection: craft-penpot's rule, made exact
by walking the directory. The writer: craft-opc's `ZipOut`.
