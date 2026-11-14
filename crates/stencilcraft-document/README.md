# craft-stencilcraft-document

Shared StencilCraft document model, validation, geometry helpers, and native file codec.
Applications re-export these model types rather than defining a second set of serializable
document types. The `.stencilcraft` header and typed wire schema are provided by
`craft-stencilcraft-format`.

The crate's `Document::load_json` and `Document::save_json` APIs read and write the versioned
native format. File path I/O helpers preserve application paths in errors and use atomic writes.
