# Craft Font Info

`craft-font-info` reads what one font face offers for fine typography, for type panels: variable
font controls, OpenType feature toggles and their previews:

- **Variation axes** (`fvar`): tag, the font's own name ("Parametric Thick Stroke"), range,
  default, and whether the font asks to hide it.
- **Named instances**: name ("Bold Condensed") and coordinates.
- **OpenType layout features** (`GSUB` and `GPOS`): every tag once, sorted; the font's UI name for
  stylistic sets (`ss01`–`ss20`) and character variants (`cv01`–`cv99`); and the characters each
  substitution changes, for a preview that shows exactly those.

Extracted from PhotoCraft (`photocraft-text`, storytold/photocraft#218).

```rust
let bytes = std::fs::read("Inter.ttf")?;
let details = craft_font_info::read(&bytes, 0)?; // face 0; the face index in a .ttc
if let Some(f) = details.feature("cv11") {
    println!("{:?} changes {}", f.name, f.sample); // Some("Single-story a") changes a
}
```

## What stays in each app

Finding the face for a text style (family, weight, italic), caching, and labels: registered
feature and axis names are each app's, translated in its own catalogs.

## Guarantees

Fonts are untrusted input. Data that isn't a font (or has no face at the index) is an error; any
malformed table is left out of the result, never a panic. Features, lookups per feature and
glyphs per coverage table are capped. Takes bytes, so callers don't share a `skrifa` version with
it. Pure Rust, no `unsafe`, builds for `wasm32-unknown-unknown`.

Tests build their font with `write-fonts` in code; no font file is committed.
