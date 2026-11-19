# craft-pst

A reader of Outlook personal folders files (`.pst`), written from Microsoft's published
specification, [MS-PST], and nothing else. No dependencies, no `unsafe`.

- `Pst::open(reader)` / `Pst::open_with(reader, Limits)` read over any `Read + Seek`. The file is
  never held in memory: a page or a block is read when it is needed.
- ANSI and Unicode files, with data blocks unencoded, permuted or cyclic-encoded. A file with a
  password answers only after `Pst::unlock`.
- The layers are the specification's: the node database (B-trees of nodes and blocks, data
  trees, subnode trees), the heap on a node, its B-tree, property contexts and table contexts,
  and the messaging layer: `store`, `walk_folders`, `items`, `associated_items`, `message`
  (properties, recipients, attachments, attached messages), `attachment_reader`,
  `read_property` for large values, and `named_properties`.
- `recur` reads the appointment recurrence pattern and time zone blobs of [MS-OXOCAL];
  `list_stream` reads the entry stream of a distribution list.
- A damaged file is salvaged: what cannot be read is left out and named (`Pst::damage`,
  `Message::left_out`).

```rust
use craft_pst::Pst;

let mut pst = Pst::open(std::fs::File::open(path)?)?;
for entry in pst.walk_folders()?.folders {
    for item in pst.items(entry.folder.id)?.items {
        let message = pst.message(&craft_pst::MessageRef::item(item.id))?;
    }
}
```

## Hostile input

Nothing panics on any bytes. Every offset and length is checked against the file, every CRC,
signature and block id against what it guards; trees are bounded in depth and read each part
once. The caps of `Limits` are checked before anything is read or allocated; past a cap a part
is left out and named (`Error::Cap`).

| Limit | Default |
|---|---|
| `max_file_size` | 100 GiB |
| `max_items` | 2,000,000 |
| `max_folders` / `max_folder_depth` | 10,000 / 64 |
| `max_item_bytes` | 256 MiB |
| `max_attachment_bytes` | 150 MiB |
| `max_recipients` / `max_attachments` | 1,000 / 1,000 |
| `max_embedded_depth` / `max_embedded_messages` | 8 / 10,000 |
| `max_btree_pages` | 4,000,000 |
| `max_subnodes` | 200,000 |
| `max_inline_value` | 4 MiB (larger values are `PropValue::Large`, read on their own) |
| `max_listing_bytes` | 1 GiB per folder walk or item listing |
| `max_summary_value` | 64 KiB |
| `read_multiple` / `read_floor` | all reads together: 64 times the file size plus 256 MiB |

## What is not here

- Writing. The `test-writer` feature has a writer of small synthetic files, for tests only.
- Offline folders files with 4 KB pages (version 36) and files protected by Windows Information
  Protection (version 37): `Error::UnsupportedVersion`.
- Text in a code page: `PropValue::String8` carries the bytes, and the caller decodes them in
  the code page the message names.
- Compressed RTF bodies (`PidTagRtfCompressed`, [MS-OXRTFCP]): the property's bytes are
  returned as they are.
- What an item means (mail, contact, appointment): that mapping is the caller's.

## Origin

MailCraft `crates/pst` at
[a44a450](https://github.com/UCS-Care-Group/mailcraft/commit/a44a45013d1c6396229c40ab60172d1882b368d5),
by Martin Ball and the MailCraft contributors (MIT OR Apache-2.0), with its tests. Left behind:
the RTF reading (`rtf.rs`) and the code-page name table, which are not part of the .pst format.
