# craft-usd

Bounded wire primitives for USD Crate, independent of application scene and material models.

Currently provides Crate-framed LZ4 decompression only: a single unprefixed block or up to 127 length-prefixed blocks, under a caller-provided output-byte cap with fallible output allocation. Trailing bytes after counted chunks retain the original reader's ignored-tail policy. This is not a complete USD reader/writer; scene interpretation, integer packing and export remain outside this first extraction.

Adapted from TextureCraft `crates/formats/src/mesh_import/usd/codec.rs`, introduced by bflatastic in `bdbb52c9b66b221a55b370105b42f485a76e6fc7`. The LZ4 backend is MIT-licensed `lz4_flex`.
