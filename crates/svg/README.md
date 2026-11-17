
### Simple stylesheet compatibility subset

`simple_css` preserves EffectCraft's compound tag/class/id stylesheet grammar and indexed
matching budget, rather than substituting the richer selector semantics of `paths`.
Property values remain text; color precision, path-prefix behavior and geometry policy
stay with the importer. Call `parse` for each sheet, then `StyleSheet::index`; discard the
sheet after a parse error and reject the import when its matching-work callback refuses.

The parser/matcher and structural tests were ported from EffectCraft
`60e2cbbbe3d73c0a9074c25a1a615ccfec18721c`, originally by FredApps. Nested at-rule skipping
and its regression test preserve NetterSilver's
`10e173c0e49696ad335e3da8a069889c089b09a6`; both contributors retain primary Git authorship
for their respective port commits. Source and destination use MIT OR Apache-2.0.
The shared adapter adds an aggregate 64 MiB stylesheet-text limit and idempotent indexing.

### Transform compatibility
`transform_compat::parse_transform` preserves EffectCraft transform-list recovery and exact composition. It retains six numeric arguments per operation; scanning is not time/byte capped and matrix overflow behavior is unchanged. The strict geometry reader is unchanged. Source: Brandon Thomas, EffectCraft94e1c86d89a866771f80ad1610afda9aa12e85d8; bounded safe argument matching by Altynzhan Karybai,4035d181329e847d7a57bf3d059502ea2c91b560.


### Opt-in validated embedded images

With `tree-render`, `tree::ValidatedImageResolver` owns one drawing's immutable
`Limits`, original child-source work, data-URL normalization and renderer-specific
raster validation. Existing `ImageResolver::new`/`resolver` and legacy parse entry
points retain their behavior. Opt in explicitly:

```rust
# #[cfg(feature = "tree-render")]
# fn example() -> Result<(), craft_svg::tree::Error> {
use craft_svg::tree::{self, Limits, ValidatedImageResolver, usvg};
let images = ValidatedImageResolver::new(Limits::default());
let checked = tree::preflight(
    "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/>",
    &images.limits(),
)?;
let drawing = images.parse_preflight(checked, usvg::Options::default())?;
let usage = images.usage();
// Importers disclose usage's invalid_* and *_limit_images; strict consumers refuse them.
# let _ = (drawing, usage);
# Ok(())
# }
```

The caller supplies fonts and a sufficient stack/worker for its depth policy.
`parse_preflight` accepts only an opaque preflight made with exactly this context's
limits. It consumes options and installs its own data-only resolver, including for
all children; unrelated callbacks cannot fetch a linked file or bypass admission.
Expanded root ancestor frames (checked height minus the callback leaf) and each
child's complete expanded height share one depth allowance. The callback leaf is
not counted twice; source nesting alone cannot hide expanded reference chains.
An overlapping root conversion on the same context is refused.

Unchanged XML converts from its original checked DOM. A modified data URL requires
one checked replacement DOM. Invalid URLs become bounded resolver-visible image
failures; hidden, unused and foreign elements do not acquire omission counts.
Original UTF-8 child bytes are charged before XML conversion or shorter repairs,
with per-document and aggregate bounds. Failed inflation/text work is not refunded;
separate bounded retry-classification work never admits another child. Encoded
raster callbacks spend at most `limits.bytes + limits.embedded_bytes` cumulatively,
without retaining their sources. Pixel work and decoder allocation are admitted
before decoding; attempted failures keep their charge. An oversized unattempted
image does not prevent a later smaller image from fitting.

`resolve_text` accepts an already-decoded child SVG through the same admission
context, rather than an unchecked DOM or a precharged-budget override. `usage()`
reports omissions and work after conversion. The context is for one drawing, not a
global rendering cache.

The renderer has no public decoded-pixel or decode-result hook. Validation therefore
still decodes raster pixels before later renderer painting. The shared validator
owns the backend dependencies; locked dependency-edge tests in both the shared
workspace and consumers must keep them aligned with the selected resvg/usvg graph.
This is a fail-fast maintenance check, not automatic access to private renderer
code or a decode-once performance claim. Direct zune-jpeg use selects its MIT option.
