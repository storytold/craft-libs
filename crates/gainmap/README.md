# craft-gainmap

HDR gain maps for the Craft apps, from LightCraft's `lightcraft-codecs`: an SDR base image plus a
small map of gains that rebuilds the HDR rendition on a display with headroom. This crate is the
metadata and the arithmetic. It does not encode or decode images and does not know a container.
No `unsafe`, no panics on malformed input; the only dependency is `thiserror`.

```toml
craft-gainmap = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

| | |
|---|---|
| `GainMapMeta::from_iso` / `to_iso` | ISO 21496-1 binary metadata (the JPEG APP2 payload after `ISO_URN`; the same bytes HEIF and AVIF carry) |
| `GainMapMeta::from_hdrgm_xmp` / `to_gain_map_xmp` | Adobe `hdrgm` XMP (Ultra HDR), attributes or `rdf:Seq` lists; the writer adds Apple's description of the same map |
| `compute` | measures a 1 or 3-channel 8-bit gain map from SDR and HDR renditions in linear light |
| `apply` | rebuilds the HDR rendition from a base and its map, at a weight for the display's headroom |
| `primary_iso_marker`, `primary_xmp` | what the primary image carries to say the file has a gain map |
| `apple::apply`, `apple::Metadata` | Apple's legacy single-channel gain maps (a linear ratio, headroom from the maker note) |

```rust
use craft_gainmap::{GainMapOptions, apply, compute};

// `sdr` and `hdr`: the same picture, linear RGB in the same primaries.
let (map, meta) = compute(&sdr, &hdr, width, height, [0.2126, 0.7152, 0.0722], &GainMapOptions::default())?;
let mut rendition = sdr.clone();
apply(&mut rendition, width, height, &map, &meta, 1.0)?;
```

Metadata readers return `None` for anything a decoder could not use: unknown versions, an HDR
base, zero denominators, non-finite values, gammas that are not positive. `compute` and `apply`
check every size (with overflow) against the buffers they are given and return `Error` instead
of indexing past them.

What stays with the caller: encoding the base and the map, the container (for a JPEG, the MPF
index that locates the second image), and colour management of the base.
