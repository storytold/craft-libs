# craft-mcmat

A bounded reader and writer for portable MaterialCraft `.mcmat` ZIP packages. A package
contains `material.json`, named texture maps and a source package. Applications interpret
parameter values and cook or render maps themselves; this crate does not depend on an app's
model. It supports callers that read either version 1 or version 2 of the manifest, or both.

`is_material` checks for a manifest with a bounded ZIP index; `Material::open` takes caller-provided version, entry-count, map-count, per-entry and total
expanded-byte limits. Each subsequent `read` and `read_source` shares that budget, including
repeated references to the same ZIP member. Duplicate member names are refused. The manifest
preserves unknown fields at the top level and on maps; parameter, input and preset objects are
kept as JSON values so their unknown fields survive too. `write` preserves the caller's source
entry name and map names, and rejects duplicate archive member names through `craft-zip`.

The source package is opaque. Version 1 commonly names `source.mcraft`; MaterialCraft's version 2
writer names `source.sbs`. Callers that invoke another application must preserve the source's
format and extension rather than infer it from `.mcmat` alone.
