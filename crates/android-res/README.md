# craft-android-res

Read-only wire decoding for Android compiled XML (AXML) and Android resource
tables (ARSC). XML and resource tables share the bounded chunk, string-pool,
and typed-value readers. APK container selection, manifest inventory policy,
device/security interpretation, and presentation remain consumer-owned.

## Provenance

The decoders, checked-reader subset, and synthetic test fixture subset were
moved from [Craft Anything at 8cec25d](https://github.com/storytold/craft-anything/tree/8cec25d02007db604b57ce6735ad4eb44a1a9948/crates/formats).
The original implementation is Brandon Thomas (echelon), [f64ae3bf](https://github.com/storytold/craft-anything/commit/f64ae3bf3c8863a982af1304e06de20653803968).
Later parser bounds and resource identity/qualifier work came from Afloat
and BFlat; typed-attribute handling also includes RRFZ's work. The original
[fixture generator](https://github.com/storytold/craft-anything/blob/8cec25d02007db604b57ce6735ad4eb44a1a9948/crates/testkit/src/android.rs)
remains the source of the test data. No external assets or added dependencies.
