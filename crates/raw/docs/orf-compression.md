# Olympus compressed ORF: not decoded

Moved from LightCraft (`docs/orf-compression.md`) with the ORF reader.

LightCraft once decoded compressed ORF sensor data (its PR #3775) and then removed that decoder while its provenance remained unresolved. The earlier clean-room and derivation claims are withdrawn; this note makes no legal determination and is not a format specification.

Compressed ORF files are unsupported for sensor decoding (`RawError::Unsupported`). Their embedded JPEG preview is still returned when there is one.

Uncompressed 16-bit, packed 12-bit, two-field 12-bit and 12.8 bpp ORF support remains in `src/orf.rs`. This includes supported uncompressed high-resolution modes; it does not imply compressed-mode support on the same camera.

Any future compressed ORF decoder requires independently established provenance and validation before support is claimed again. An open LightCraft pull request (storytold/lightcraft#2068) proposes one; its provenance has not been established, so it is not ported here.
