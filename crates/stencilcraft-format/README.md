# craft-stencilcraft-format

Reader and writer for the versioned `.stencilcraft` JSON file. The JSON header is
`{"stencilcraft":{"name":"stencilcraft","version":1}}`; the document fields share the
top-level object with the header. `DocumentFields` owns field names, flattening, required fields,
version 1 defaults, and omission of default canvas members. Its type parameters let consumers use
their typed model fields directly; `File::<DocumentFields<...>>::read_typed` and
`to_json_pretty` handle the shared schema and JSON conversion. StencilCraft's typed model and
domain validation live in `craft-stencilcraft-document`.

The crate does not open files or set a file-size policy. Callers perform bounded file I/O before
calling `File::read`.

```toml
craft-stencilcraft-format = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

`File::read` parses JSON into the default JSON-valued schema. For typed models, use
`File::<DocumentFields<...>>::read_typed`; both readers validate the name and exact supported
version and ignore unknown members for version 1 compatibility. `File::new` and `to_json_pretty`
write the current header and flattened fields. Absent `next-shape-id` defaults to 1 and absent
`images` defaults to an empty list, matching early version 1 files.

`cargo test -p craft-stencilcraft-format` covers round-trip wire shape, version 1 defaults,
unknown members, wrong and missing headers, unsupported versions, malformed JSON, and missing
required fields.
