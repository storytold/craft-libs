# craft-kiwi

A clean-room decoder and encoder for the kiwi binary schema format: an open, MIT-licensed message
format with a public specification. No dependencies beyond `std`, no `unsafe`.

A binary schema lists definitions (enums, structs and messages, each with typed fields). A message
is decoded with a schema into a `Kv` tree keyed by field name, and encoded back from one. The
schema usually travels with the data (`.fig` files carry theirs), so nothing here is generated
from a schema at build time: a reader looks fields up by name and takes what it understands.

## Provenance

Written by FigCraft's contributors for its `.fig` import and export, from the format's public
description only: the published specification of the encoding (variable-length integers, zigzag
signed integers, floats with the exponent byte first, NUL-terminated strings, messages as
`field id, value, …, 0`, structs as every field in order). No code of the reference
implementation or of any other product was copied. Moved here from
`figcraft/crates/format/src/fig/kiwi.rs`.

## Reading

```rust
let schema = craft_kiwi::Schema::decode(&schema_bytes)?;
let root = schema.find("Message").ok_or("no Message definition")?;
let message = schema.decode_message(root, &message_bytes)?;
for node in message.get("nodeChanges").map(craft_kiwi::Kv::list).unwrap_or_default() {
    let name = node.get("name").and_then(craft_kiwi::Kv::str);
}
```

`Kv` is `Bool`, `Int`, `Uint`, `Float`, `Str`, `Bytes` (a `byte[]` field), `Enum` (the value's
name; an unknown value stays a `Uint`), `Array` and `Object` (a struct or message: its fields in
the order read). Field and enum names borrow from the schema.

## Writing

```rust
let schema = craft_kiwi::Schema::parse("struct Point float x, float y\nmessage Message Point[] points, string name")?;
let bytes = craft_kiwi::encode::message(&schema, root, &value)?;
let schema_bytes = craft_kiwi::encode::schema(&schema);
```

`Schema::parse` reads a compact text form, one definition per line: `enum Name A B …` (values
from 0), `struct Name type field, type field[] …` or `message Name …` (field ids from 1); `A=3`
or `type field=7` gives a value or a field its own number. The schema limits below apply to the
text form too, and it refuses two fields of one name and a message field id that is 0 or repeated. The encoder checks values against the
schema and fails on what doesn't fit: unknown fields or enum names, missing struct fields, wrong
types, numbers out of range, NaN.

## Limits

Input is hostile. Every read is bounds-checked and nothing panics.

| Limit | Value |
|---|---|
| Nesting of structs and messages (`MAX_DEPTH`) | 64 |
| Definitions in a schema (`MAX_DEFS`) | 10 000 |
| Fields in one definition (`MAX_FIELDS`) | 10 000 |
| Fields in a schema (`MAX_SCHEMA_FIELDS`) | 200 000 |
| Values in one message (`DEFAULT_MAX_VALUES`; `decode_message_within` takes another budget) | 20 000 000 |

A value is every scalar, array element, array field and nested definition. An array count larger
than the bytes left is refused before anything is allocated, a `u32` varint that overflows is
refused, and a message definition with two fields of one id is refused as ambiguous.
