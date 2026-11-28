# craft-stencilcraft-document

Shared StencilCraft document model, validation, geometry helpers, and native file codec.
Applications re-export these model types rather than defining a second set of serializable
document types. The `.stencilcraft` header and typed wire schema are provided by
`craft-stencilcraft-format`.

The crate's `Document::load_json` and `Document::save_json` APIs read and write the versioned
native format. File path I/O helpers preserve application paths in errors and use atomic writes.

The v2 model is geometry-once: every path is stored a single time in `Document.shapes`, and
each layer references its shapes by id through `Layer.members` while carrying its production
pass as `Layer.operation` with a full `OperationSettings` snapshot. A shape may be a member of
many layers, and dangling or duplicate memberships are pruned on load and save rather than
rejected. Version 1 files migrate automatically in memory (per-shape `style`, per-layer `kind`
and inline layer geometry become shapes plus operation layers); the file is rewritten as v2 on
the next save, and v2 is never downgraded.
