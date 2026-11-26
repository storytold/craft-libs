
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
