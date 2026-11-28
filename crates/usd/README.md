# craft-usd

Bounded wire primitives and neutral `.usda`/`.usdc` scene, prim, and layer read/write for USD, independent of application mesh and material models.

Provides:
- Bounded integer-delta array decoding/encoding and Crate-framed LZ4 compression/decompression (`compress`, `integers`, `decompress`) under caller-provided output-byte caps with fallible allocation.
- Shared Crate wire constants and type metadata (`codec`).
- Neutral `.usda` (text) and `.usdc` (Crate binary) layer reader (`scene::read_layer`, `scene::read_layer_with_budget`) preserving `def`/`over`/`class` specifiers, internal references, coordinate metadata (`metersPerUnit`, `upAxis`), payload/sublayer/variant warnings, and decoded byte budgets (`scene::Budget`).
- Neutral `.usda` and `.usdc` scene/prim/layer writer (`scene::write_usda`, `scene::write_usdc`).

Adapted from TextureCraft's `mesh_import/usd` and `mesh_export/{usd,usdc}.rs`, originally introduced by bflatastic in `bdbb52c9b66b221a55b370105b42f485a76e6fc7`. The LZ4 backend is MIT-licensed `lz4_flex`.
