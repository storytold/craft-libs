# craft-cfb

A clean-room reader and writer for the Compound File Binary format (Microsoft "OLE structured
storage"): the container of legacy Office files (`.doc`, `.xls`, `.ppt`), Outlook `.msg` messages,
password-protected OOXML packages and AAF. No dependencies beyond `std`, no `unsafe`. No
third-party implementation was consulted.

| Document | Edition | Used for |
|---|---|---|
| [MS-CFB] Compound File Binary File Format (Microsoft Open Specifications) | revision 10.0 (the format is unchanged since 1.0) | header, sector chains, FAT / DIFAT / mini FAT, directory entries, red-black sibling trees, name comparison, version 3 and 4 files |

The crate knows nothing about what the streams hold. Property sets, Office records, encryption
and AAF objects belong to the crates that read those formats.

## Reading

```rust
let bytes = std::fs::read("book.xls")?;
let cf = craft_cfb::CompoundFile::open(&bytes)?;
let workbook = cf.read_path("Workbook")?;                    // a stream, by path
let small = cf.read_path_capped("\u{5}SummaryInformation", 1 << 20)?; // with a size cap
for id in cf.walk() {                                         // every storage and stream
    let e = &cf.entries[id];
    println!("{:?} {} {}", e.kind, cf.path(id).unwrap_or_default(), e.size);
}
let vba = cf.find("Macros/VBA")?;                             // a storage
for &child in &cf.entries[vba].children { /* its children, in sibling order */ }
```

- Input is a `&[u8]`. Streams are copied out on `read`; nothing is copied before that except the
  FAT, the mini FAT, the directory and the mini stream.
- Version 3 (512-byte sectors) and version 4 (4096-byte sectors).
- Paths are `/`-separated below the root; a leading `/` is allowed. Names compare as [MS-CFB]
  §2.6.4 says siblings do: case-insensitively, by upper-cased UTF-16 code units
  (`compare_names`).
- Lookups do not depend on the sibling trees being sorted, balanced or correctly coloured. Each
  storage's children are indexed by name when the file is opened.

### What the reader tolerates

- A file that ends inside its last sector (some writers do not pad it). The sector reads short;
  a stream that needs bytes the file does not hold is still an error, and a short sector in the
  middle of a chain is an error.
- A mini stream that is damaged or shorter than the root entry says. The file opens and streams in
  regular sectors read; only the streams that need the missing part fail.
- Sibling and child links to directory entries that do not exist. They are treated as no link.
- Directory entries the root does not reach, whatever they point at.
- Spare FAT sectors (writers reserve them for growth) and DIFAT sectors padded with zeros.
- Garbage in the high half of a stream size before version 4.
- A version field that does not match the sector size (the sector shift decides).

### What the reader refuses

`Error::NotCfb` for a missing signature or byte order mark, `Error::Invalid` for the rest:

- a sector shift other than 9 or 12, a mini sector shift other than 6;
- more FAT sectors than the file has sectors, or a FAT sector listed twice;
- a DIFAT, FAT or mini FAT chain that loops or leaves the file, including a loop shorter than the
  stream it belongs to;
- a directory entry reached twice (a cycle or a shared subtree);
- a first directory entry that is not the root.

### Limits

Every size, offset and chain from the file is checked, and every loop is bounded by the number of
sectors the file has. `open` allocates the FAT, the mini FAT, the directory and the mini stream,
each of which is made of distinct sectors of the input, so its memory is a small multiple of the
input's length. Two things are capped by `Limits` (`CompoundFile::open_with`):

| Limit | Default | Exceeded |
|---|---|---|
| `max_stream_size` | 1 GiB | `read` returns `Error::LimitExceeded`; nothing is allocated |
| `max_dir_entries` | 1 048 576 | `open` returns `Error::LimitExceeded` |

`read_capped` and `read_path_capped` take a smaller cap for one call. A stream whose declared
size is larger than the file is refused before any allocation, whatever the limits.

### Files too damaged to open

```rust
let limits = craft_cfb::Limits::default();
let locked = ["EncryptedPackage", "EncryptionInfo"].iter().any(|n| craft_cfb::has_entry_named(&bytes, n, &limits));
```

`directory_names` follows only the header, the DIFAT and the directory chain and returns the name
of every directory entry. `has_entry_named` uses it and, when the directory cannot be followed,
searches the bytes for the name in UTF-16LE. It is meant for choosing an error message ("this
file is password-protected"), not for security decisions: the fallback can match stream contents.

## Writing

```rust
let file = craft_cfb::write_streams(craft_cfb::Version::V3, &[("Workbook", &book), ("Macros/VBA/dir", &dir)])?;

let mut w = craft_cfb::Writer::new(craft_cfb::Version::V4);
w.set_clsid(craft_cfb::Writer::ROOT, root_class);
let h = w.storage(craft_cfb::Writer::ROOT, "Header", header_class)?;
w.stream(h, "properties", bytes)?;
w.stream_at("Header/Content/data", more)?;      // creates "Content"
let file: Vec<u8> = w.finish();
```

Layout: stream sectors (streams of 4096 bytes or more), mini stream, mini FAT, directory, FAT,
DIFAT. Each chain is contiguous. Sibling trees are balanced binary search trees in [MS-CFB] name
order (length first, then upper-cased code units) coloured as valid red-black trees. Names are
checked (1 to 31 UTF-16 code units, none of `/ \ : !` or NUL, unique per storage,
case-insensitively); the duplicate check is a hash lookup, so adding n children is O(n).

## What it does not do

- No `Read + Seek` input and no streaming output: files are read from and written to memory.
- No editing in place. To change a file, read its streams and write a new one.
- No timestamps or state bits on write (they are read).
- No transactions, no sector-level access, no free-sector reuse.
- Version 3 files are not limited to the 2 GB the specification allows them.
- Nothing about stream contents (property sets, `\1CompObj`, encryption).

## Where it came from

The base is FilmCraft's `filmcraft-cfb` (reader, writer and tests, moved as they were). Three
other Craft apps had their own readers; what they did and the base did not was ported, each with
a test in `src/tests/ported.rs`. Where the copies disagreed:

| Question | FilmCraft | GridCraft / DeckCraft | WordCraft (guard in front of the `cfb` crate) | Here |
|---|---|---|---|---|
| Sector shift | 7 to 16 | 9 or 12 | by version: 3 → 512, 4 → 4096 | 9 or 12 (the specification; a smaller sector overlaps the header) |
| Byte order mark | required | not checked | checked by `cfb` | required by `open`; not checked by `directory_names` |
| Where the FAT sector list ends | the header's count | every listed number | the header's count, trailing zeros and free slots trimmed | the header's count: it is what makes zero-padded DIFAT sectors readable |
| A FAT sector listed twice | allowed (FAT bounded by the header's count) | allowed | refused | refused |
| More FAT sectors than the file has sectors | refused | refused | refused; a tighter bound had refused real files with spare FAT sectors | refused; spare sectors accepted |
| File ends inside its last sector | error | reads short, if it is the chain's last sector | (left to `cfb`) | GridCraft's rule, for streams, the directory, the mini FAT and the mini stream; FAT and DIFAT sectors must be whole |
| Chain loop | step count bounded by the file's sectors; a loop inside the stream's size read back as repeated data | a visited flag per FAT entry, allocated per read | (left to `cfb`) | step bound, plus a check for a repeated sector in the chain (memory proportional to the stream, not to the FAT) |
| Damaged or short mini stream | `open` fails | `open` succeeds, the stream read fails | (left to `cfb`) | `open` succeeds |
| Stream size cap | the file's length | 1 GiB | a per-call cap at each caller | `Limits::max_stream_size` (default 1 GiB) and a per-call cap |
| Size before version 4 | high half ignored when the version is 3 | high half ignored when the version is below 4 | (left to `cfb`) | below 4 |
| Link to a missing directory entry | `open` fails | ignored | (left to `cfb`) | ignored |
| Entry reached twice | `open` fails | the lookup fails if it gets that far | (left to `cfb`) | `open` fails |
| Which storages are walked | every storage entry, in directory order (an orphan pointing into the live tree made `open` fail) | the root only | (left to `cfb`) | every storage the root reaches |
| Name lookup | [MS-CFB] upper-casing, sorted index | ASCII case-insensitive, linear, streams only | `cfb` | FilmCraft's; `has_stream` for "a stream of that name" |
| Name of an entry | the code units the length covers, up to a NUL | the code units the length covers | `cfb` | FilmCraft's |
| Directory size | the file's length | 1024 sectors in the scan | (left to `cfb`) | `Limits::max_dir_entries` |
| Largest regular sector number | `<= 0xFFFFFFFA` | `< 0xFFFFFFFA` | `cfb` | `<= 0xFFFFFFFA` (no file can hold either) |

## Migration

FilmCraft (`filmcraft_cfb`):

| Before | After |
|---|---|
| `filmcraft_cfb::{CompoundFile, Entry, EntryKind, Writer, NodeId, Version, Error, sniff, compare_names, validate_name}` | the same names in `craft_cfb` |
| `CompoundFile::open(bytes)` with no stream cap | `CompoundFile::open_with(bytes, Limits { max_stream_size: u64::MAX, ..Limits::default() })` if streams over 1 GiB must read |
| matching on `Error` | `Error` is `#[non_exhaustive]` and has `LimitExceeded` |

GridCraft (`crates/xlsx/src/cfb.rs`) and DeckCraft (`crates/pptx/src/cfb.rs`):

| Before | After |
|---|---|
| `cfb::SIGNATURE`, `bytes.starts_with(&SIGNATURE)` | `craft_cfb::SIGNATURE`, `craft_cfb::sniff(bytes)` |
| `cfb::is_encrypted(bytes)` | `["EncryptedPackage", "EncryptionInfo"].iter().any(\|n\| craft_cfb::has_entry_named(bytes, n, &limits))` |
| `directory_names(bytes)` | `craft_cfb::directory_names(bytes, &limits)` |
| `Compound::open(bytes)` | `CompoundFile::open(bytes)` |
| `cf.has_root_stream(name)` | `cf.has_stream(name)` |
| `cf.root_stream(name)` → `Ok(None)` when absent | `if cf.has_stream(name) { Some(cf.read_path(name)?) } else { None }` |
| `MAX_STREAM` (1 GiB) | the default `Limits::max_stream_size` |
| `testing::compound_file(streams)` | `craft_cfb::write_streams(Version::V3, streams)` (a different sector layout: tests that cut or patch the file at fixed offsets need new offsets) |

WordCraft (`wordcraft_doc::cfb_guard` and the `cfb` crate):

| Before | After |
|---|---|
| `cfb_guard::check(bytes)?` then `cfb::CompoundFile::open(Cursor::new(bytes))` | `CompoundFile::open(bytes)` (the guard's checks are part of `open`) |
| `comp.is_stream(path)` | `cf.has_stream(path)` |
| `comp.open_stream(path)` then `take(cap + 1).read_to_end(..)` | `cf.read_path_capped(path, cap)` (`Error::LimitExceeded` when larger) |
| `CompoundFile::create_with_version(v, Cursor::new(Vec::new()))`, `create_storage_all`, `create_stream(..).write_all(..)`, `flush`, `into_inner().into_inner()` | `Writer::new(v)`, `stream_at(path, bytes)`, `finish()`, or `write_streams(v, &[(path, bytes)])` |
| `cfb::Version::{V3, V4}` | `craft_cfb::Version::{V3, V4}` |
| `cfb_guard::repeating_difat_sample()` (test input) | not exported; the regression is `a_repeating_difat_is_refused` here |

## Tests

`src/tests.rs` (from FilmCraft): round trips for both versions with empty, mini, boundary
(63/64/65, 4095/4096/4097 bytes) and multi-sector streams and up to 40 siblings; red-black
invariants of every sibling tree; a 7.5 MB stream that needs DIFAT sectors; Unicode and
case-insensitive names; random mutations; DIFAT and directory chain loops.

`src/tests/ported.rs`: files from a second, independent test writer with another layout and an
unsorted sibling list; every behaviour in the table above; each limit; truncation at every length
and every single-bit flip of two small files, and every special sector value in every header, FAT
and directory field, none of which may panic or hang.

`tests/writer_scale.rs`: 40 000 children in one storage in under 3 s; duplicate-name rules.
