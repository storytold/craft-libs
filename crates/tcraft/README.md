# craft-tcraft

Streaming neutral `TCTX` binary payloads used inside TextureCraft `.tcraft` projects. Sparse
paint data stores 64×64 tiles of little-endian `u16` components; baked maps store dense
little-endian `f32` values. This crate does not yet define the project JSON schema or ZIP
envelope and does not depend on TextureCraft's app types.

`read_tiles` validates the expected dimensions and components, bounds tile count, rejects
out-of-range and repeated tile positions, and delivers one owned tile at a time. `read_baked`
validates dimensions and declared length and decodes in bounded chunks. Both consume the same
caller-owned `craft_zip::Budget` used for the surrounding project and reject trailing data.
`write_tiles` and `write_baked` stream their bodies rather than copying an entire image; a caller
must discard an incomplete output if a write fails. Limits and ownership rules beyond this
binary format remain with the project reader.
