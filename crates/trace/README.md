# craft-trace

Image Trace for the Craft suite's 2D vector kernel (RFC-0030): raster in, `craft_geom::PathData` out. Carved from VectorCraft's `crates/trace` and re-synced from storytold/vectorcraft@9de706507c30eda3b956862875a0a1655170063f, with its tests. VectorCraft's copy is retired by its RFC-0030 adoption PR.

A clean-room tracer built from the published ideas behind bitmap tracers (the potrace paper and classic colour quantisation), not from any existing implementation. The pipeline:

1. **Quantise** the image into a small palette: luminance threshold (black and white), 1-D k-means over the luminance histogram (grayscale), or median cut plus weighted k-means over a 15-bit colour histogram (colour), as the `Palette` asks. Blend pixels that anti-aliasing leaves along edges take no palette entry. Continuous-tone images are smoothed within their tones first, edges kept.
2. **Remove noise**: whole 4-connected same-colour regions merge, smallest first, into their most common neighbouring colour until no mergeable region is smaller than the noise setting.
3. **Follow boundaries**: each colour layer's pixel-crack edges link into closed loops, outer loops clockwise and holes counter-clockwise (y down), so results fill correctly under either fill rule.
4. **Sub-pixel edges**: each crack moves to where the image says the edge really is, so the outline is right to a fraction of a pixel before any fitting. Outlines are cut at junctions and each piece fitted the same way from either side, so abutting regions share one curve.
5. **Curves**: corners the pixels rounded are put back, straight runs become lines, a round outline becomes a true circle, and the rest becomes as few cubic Béziers as the tolerance allows.

Centre-line (stroke) tracing, abutting or overlapping colour layers, and gradient detection ride on the same pipeline; see the crate docs.

## Dependencies

`craft-geom`, `kurbo`, `craft-image-codecs` (decoding bounded, hostile-input limits applied before any pixel is read), `serde` and `thiserror`. The quality harness under `tests/quality` measures fidelity and cost and writes its own report.
