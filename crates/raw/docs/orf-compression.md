# Olympus compressed ORF: withdrawal record

Moved from LightCraft (`docs/orf-compression.md`) with the ORF reader.

LightCraft once decoded compressed ORF sensor data (its PR #3775) and then removed that decoder while its provenance remained unresolved. The earlier clean-room and derivation claims are withdrawn; this note makes no legal determination and is not a format specification.

The decoder withdrawn from #3775 remains absent. A different implementation from LightCraft #3964 now reads the measured lossless streams; [its provenance and validation record](orf.md) keeps historical derivation and provenance acceptance separate from sensor-pixel agreement. Unsupported modes retain embedded-preview fallback.

Uncompressed 16-bit, packed 12-bit, two-field 12-bit and 12.8 bpp ORF support remains in `src/orf.rs`. This includes supported uncompressed high-resolution modes; it does not imply every mode on the same camera is verified.

The disputed implementation proposed in storytold/lightcraft#2068 is not ported here. The lossless implementation extracted from #1162 and validated in #3964 is documented separately; output agreement does not resolve its provenance acceptance.
