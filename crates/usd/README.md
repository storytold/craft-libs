# craft-usd

Bounded wire primitives for USD Crate, independent of application scene and material models.

Provides bounded integer-delta array decoding and Crate-framed LZ4 compression/decompression: a single unprefixed block or up to 127 length-prefixed blocks, under a caller-provided output-byte cap with fallible output allocation. Trailing bytes after counted chunks retain the original reader's ignored-tail policy. This is not a complete USD reader/writer; scene interpretation, integer packing and full scene export remain outside this extraction. Compression uses a caller cap on worst-case reserved output and fallible allocation. Integer decoding uses explicit 32/64-bit widths, signed wrapping arithmetic, a caller output-byte cap and fallible allocation.

Adapted from TextureCraft `crates/formats/src/mesh_import/usd/codec.rs`, introduced by bflatastic in `bdbb52c9b66b221a55b370105b42f485a76e6fc7`. The LZ4 backend is MIT-licensed `lz4_flex`.
