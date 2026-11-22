# SQLite reader comparison

This comparison addresses the LightCraft and Craft Anything SQLite item in [#778](https://github.com/storytold/craft-libs/issues/778#issuecomment-7022490467).

## Readers and results

- LightCraft's Lightroom importer checks the WAL and passes the validated committed prefix to its file-backed page reader. The probe calls that reader directly with a complete, committed WAL produced by SQLite. [LightCraft baseline `78b57565`](https://github.com/storytold/lightcraft/blob/78b575658a61d17bd392b4b81b64c92c8beecbac/crates/engine/src/lightroom_catalog.rs#L512-L530)
- Craft Anything's `db_sqlite_show` reads the main-file image and checks for a WAL sidecar so it can disclose a potentially older view. The probe calls the same parser and row-pagination method as the tool. [Craft Anything baseline `df227090`](https://github.com/storytold/craft-anything/blob/df2270904c36d6904883ba4442fb80fb383ed350/crates/engine/src/tools/database_tools.rs#L49-L93)

The fixture is one table with 512-byte pages. SQLite queries verified the seven stored values: row ID 1, integer −8,388,607, real 1.25, null, label `Päris`, 6,000 ASCII `x` characters, and bytes `00 ff`. The text spans overflow pages. An update changes the label to `committed`.

| Input | LightCraft import reader | Craft Anything inspection reader |
| --- | --- | --- |
| Same pre-update main image | `Päris` | `Päris` |
| Same main image with SQLite's committed WAL available | `committed` after the caller supplies the committed prefix | Main-file view remains `Päris` |
| Same checkpointed main image | `committed` | `committed` |
| Overflow text in every tested view | All 6,000 bytes | First 4,096 bytes, marked truncated |
| BLOB in every tested view | Exact `00 ff` bytes | Length 2 and SHA-256 prefix `06eb7d6a69ee19e5` |

The scalar values other than the updated label agreed across every view. Both pinned reader modules compiled and all five probe calls passed on Windows with Rust 1.99.0 and Python 3.14.4 using SQLite 3.50.4. The WAL was 7,536 bytes. These are module-level checks. The production WAL checksum scanner, app frontends, malformed database recovery, and schema-default edge cases were outside this run.

## Candidate shared boundary

Start with bounded varint decoding and serial-type widths over borrowed bytes. The reader sources already have independently implemented versions of these mechanics. Before adopting a helper, add boundary fixtures for ninth-byte varints, failed reads and offset handling, reserved serial types, and truncated values. Both readers assemble overflow data, but their caps and treatment of partial data need separate parity evidence before that code moves. [LightCraft reader](https://github.com/storytold/lightcraft/blob/78b575658a61d17bd392b4b81b64c92c8beecbac/crates/engine/src/lightroom_sqlite.rs) · [Craft Anything reader](https://github.com/storytold/craft-anything/blob/df2270904c36d6904883ba4442fb80fb383ed350/crates/formats/src/sqlite.rs)

Keep snapshot selection and caller validation with the Lightroom importer. Keep schema defaults, pagination, text previews and BLOB presentation with Craft Anything. Coordinate a consumer change against the current catalog bounds and old-row default work. [LightCraft #3674](https://github.com/storytold/lightcraft/pull/3674) · [Craft Anything #1349](https://github.com/storytold/craft-anything/pull/1349)

## Reproduce

Place the three repositories under one parent directory and check out the two app revisions above. In a scratch directory at `craft-libs/target/sqlite-reader-parity`, save the three blocks below as `generate.py`, `Cargo.toml` and `src/main.rs`. Run from that scratch directory. Use a new fixture path on each run.

```text
python generate.py fixtures
cargo run --quiet -- fixtures
```

### Fixture generator

```python
from pathlib import Path
import sqlite3
import sys

root = Path(sys.argv[1])
root.mkdir(parents=True, exist_ok=False)
live = root / "live.db"
with sqlite3.connect(live) as connection:
    connection.execute("PRAGMA page_size=512")
    assert connection.execute("PRAGMA journal_mode=WAL").fetchone() == ("wal",)
    connection.execute("PRAGMA wal_autocheckpoint=0")
    connection.execute(
        "CREATE TABLE samples(id INTEGER PRIMARY KEY, n INTEGER, r REAL,"
        " missing, label TEXT, overflow TEXT, binary BLOB)"
    )
    expected = (1, -8388607, 1.25, None, "Päris", "x" * 6000, b"\0\xff")
    connection.execute("INSERT INTO samples VALUES (?,?,?,?,?,?,?)", expected)
    connection.commit()
    assert connection.execute("SELECT * FROM samples").fetchone() == expected
    assert connection.execute("PRAGMA page_size").fetchone() == (512,)
    assert connection.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone() == (0, 0, 0)
    (root / "base.db").write_bytes(live.read_bytes())

    connection.execute("UPDATE samples SET label='committed'")
    connection.commit()
    committed = expected[:4] + ("committed",) + expected[5:]
    assert connection.execute("SELECT * FROM samples").fetchone() == committed
    wal = live.with_name(live.name + "-wal").read_bytes()
    assert len(wal) >= 32 + 24 + 512 and (len(wal) - 32) % (24 + 512) == 0
    assert int.from_bytes(wal[-(512 + 20):-(512 + 16)], "big") > 0
    (root / "committed.wal").write_bytes(wal)
    assert connection.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone() == (0, 0, 0)
    (root / "checkpoint.db").write_bytes(live.read_bytes())
    assert connection.execute("SELECT * FROM samples").fetchone() == committed
print("SQLite", sqlite3.sqlite_version, "| 512-byte pages |", len(wal), "WAL bytes")
```

### Runner manifest

```toml
[package]
name = "sqlite-parity-probe"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = { version = "=1.0.229", features = ["derive"] }
sha2 = "=0.10.9"

[workspace]
```

### Runner

```rust
#[path = "../../../../lightcraft/crates/engine/src/lightroom_sqlite.rs"]
mod light;
#[path = "../../../../craft-anything/crates/formats/src/sqlite.rs"]
mod inspect;

use inspect::{CellValue, SqliteDb};
use light::{Database, FileSource, Value};
use std::fs;
use std::path::Path;

#[allow(dead_code)]
#[derive(Debug)]
enum FormatError {
    TooLarge(String),
    Unsupported(String),
    Truncated(String),
    Malformed(String),
}

fn light_row(main: &Path, wal: &[u8], label: &str) {
    let database = Database::open_file_with_wal(FileSource::open(main).unwrap(), wal).unwrap();
    let table = database.live_tables().unwrap().into_iter().find(|table| table.name == "samples").unwrap();
    let rows = database.read_table(table.rootpage, 7).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].values,
        [
            Value::Integer(1),
            Value::Integer(-8388607),
            Value::Real(1.25),
            Value::Null,
            Value::Text(label.to_owned()),
            Value::Text("x".repeat(6000)),
            Value::Blob(vec![0, 255]),
        ]
    );
    database.ensure_unchanged().unwrap();
    println!("LightCraft: {label}; all 7 values; full 6000-byte text and 2-byte BLOB");
}

fn inspector_row(main: &Path, label: &str) {
    let bytes = fs::read(main).unwrap();
    let database = SqliteDb::parse(&bytes).unwrap();
    let table = database.find_object("samples").unwrap();
    let page = database.read_rows(table.rootpage, 0, 2);
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].rowid, Some(1));
    assert_eq!(
        page.rows[0].values,
        [
            CellValue::Int(1),
            CellValue::Int(-8388607),
            CellValue::Real(1.25),
            CellValue::Null,
            CellValue::Text { text: label.to_owned(), truncated: false },
            CellValue::Text { text: "x".repeat(4096), truncated: true },
            CellValue::Blob { len: 2, sha256_prefix: "06eb7d6a69ee19e5".into() },
        ]
    );
    println!("Craft Anything: {label}; all 7 values; 4096-byte projection and BLOB digest");
}

fn main() {
    let fixture_arg = std::env::args_os().nth(1).expect("fixture directory");
    let fixtures = Path::new(&fixture_arg);
    let base = fixtures.join("base.db");
    let checkpoint = fixtures.join("checkpoint.db");
    let wal = fs::read(fixtures.join("committed.wal")).unwrap();
    println!("Main image");
    light_row(&base, &[], "Päris");
    inspector_row(&base, "Päris");
    println!("Committed WAL overlay");
    light_row(&base, &wal, "committed");
    println!("Checkpointed control");
    light_row(&checkpoint, &[], "committed");
    inspector_row(&checkpoint, "committed");
}
```
