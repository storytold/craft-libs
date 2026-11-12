# craft-psd

A standalone reader and writer for Adobe Photoshop **PSD** (version 1) and **PSB** (version 2, "large document format") files. It keeps everything it reads, so an unmodified file writes back byte for byte. It came from PhotoCraft (`photocraft-psd`) and depends on no app.

* Clean-room implementation from Adobe's public *Photoshop File Formats Specification*. Where the spec is silent, behavior follows MIT-licensed psd-tools / ag-psd documentation, and the code comments say so.
* `#![forbid(unsafe_code)]`. Builds for `wasm32-unknown-unknown`. The core API works on byte slices; the file helpers only exist on native targets.
* Parsing never panics on malformed input. Limits: dimensions ≤ 300 000; allocations are checked against the remaining input; a single decode produces at most `MAX_DECODED_BYTES` (8 GiB on 64-bit targets, 2 GiB on 32-bit targets); descriptor nesting depth ≤ 64.

## Guarantees

| Property | How |
|---|---|
| `PsdFile::from_bytes(b)?.to_bytes()? == b` for unmodified files | Channel data keeps its original encoding. Unknown blocks and resources are stored raw. Exact padding is recorded wherever it can vary: tagged blocks, layer info, section trailers. |
| `from_bytes(to_bytes(f)) == f` for any model `f` | Canonical defaults: `padding: None` means "zero-pad to even", and the parser yields `None` whenever the file used that default. |
| Lazy decoding | The parser only copies encoded bytes. Channels are decoded when asked (`ChannelData::decode`, `ImageData::decode`). |
| Lazy reading | `PsdFile::from_source` reads only the structure. Channel data and the merged image stay in the source as `Stored` ranges and are read when decoded, so opening a 2 GB file doesn't need 2 GB for the file itself. The model equals what `from_bytes` builds, and it writes back the same bytes. |
| No compositing | `PsdBuilder` takes the merged image from the caller. Without one, it writes a white placeholder and sets `has_real_merged_data = false` in resource 1057. |

## Public API (overview)

```rust
// Top level
PsdFile::from_bytes(&[u8]) -> Result<PsdFile>
PsdFile::from_bytes_and_merged(&[u8], want) -> Result<(PsdFile, Option<Result<Vec<u8>>>)>
                                            // + decode_merged() when want(&file): ZIP inflated once
PsdFile::from_bytes_salvaging(&[u8]) -> Result<(PsdFile, Option<PsdError>)>  // damaged layers: flattened, with the skipped error
PsdFile::composite_f32() / layer_f32(i, canvas) -> Result<FloatImage>   // straight RGBA f32, every mode and depth, within a budget
PsdFile::from_source(Source) -> Result<PsdFile>   // channel data and merged image left in the source
PsdFile::from_source_and_merged_recovering(Source, want, recover)
PsdFile::to_bytes(&self) -> Result<Vec<u8>>
PsdFile::open(path) / save(path)            // native only
PsdFile { header, color_mode_data, resources, layer_info, layer_info_placement,
          global_layer_mask, global_blocks, layer_mask_trailing, image_data }
PsdFile::layers() -> &[LayerRecord]         // file order = bottom-most first
PsdFile::layers_mut() -> &mut Vec<LayerRecord>
PsdFile::layer(i) -> Option<Layer<'_>>;  iter_layers()
PsdFile::layer_tree() -> Vec<LayerNode>     // nested groups, children bottom-to-top
PsdFile::layer_tree_max_depth(n) -> Vec<LayerNode> // same, groups nested n deep keep no children
PsdFile::decode_merged() -> Result<Vec<u8>> // planar big-endian samples
PsdFile::composite_rgba8() -> Result<RgbaImage>
PsdFile::resource(id), global_block(key), icc_profile(), resolution(),
        has_real_merged_data(), merged_has_alpha(), validate()

// Head only: header, colour mode data and image resources; no layers, no pixels
Head::from_bytes(&[u8]) -> Result<Head>     // the file or a prefix reaching the end of the resources
Head::from_source(&Source) -> Result<Head>  // reads nothing after the resources
Head { header, color_mode_data, resources, layer_section_offset };  Head::resource(id)
Header::from_bytes(&[u8]) -> Result<Header> // the first 26 bytes
head::sniff(&[u8]) -> bool

// Sources (lazy reading)
trait ReadAt { len(), read_at(offset, buf) }   // [u8], Vec<u8>, Arc<T>, FileSource (pread / seek_read)
Source::new(impl ReadAt) / read_vec(offset, len)   // shared handle; equal when the same source
Stored { source, offset, len }::read() / read_part(at, len)

// Header
Header { version: Version::{Psd,Psb}, channels, width, height, depth, color_mode, reserved }
ColorMode::{Bitmap, Grayscale, Indexed, Rgb, Cmyk, Multichannel, Duotone, Lab, Unknown(u16)}

// Layers
LayerInfo { merged_alpha, layers: Vec<LayerRecord>, padding }
LayerRecord { rect, channels: Vec<ChannelData>, blend_mode, opacity, clipping, flags,
              filler, mask: MaskData, blending_ranges, name, blocks, extra_trailing }
LayerRecord::name()            // prefers `luni`
LayerRecord::section_divider(), section_type(), layer_id(), fill_opacity(), is_visible(),
             block(key), block_mut(key), channel(id), channel_rect(id), decode_channel(id, depth, version)
ChannelData { id, compression: Option<Compression>, data, stored: Option<Stored> }   // encoded bytes kept verbatim
ChannelData::new(id, compression, data); encoded() -> Cow<[u8]>; encoded_len(); load()
ChannelData::encode(id, compression, decoded, w, h, depth, version)
ChannelData::decode(w, h, depth, version); set_decoded(...)
MaskData::{None, Mask(LayerMask), Raw(Vec<u8>)}
LayerMask { rect, default_color, flags, parameters: Option<MaskParameters>, real: Option<RealMask>, trailing, real_first }
BlendMode (27 modes + PassThrough + Unknown([u8;4])); BlendMode::key()/from_key()
Layer<'_>::rgba8() -> Result<RgbaImage>; user_mask() -> Option<Result<GrayImage>>; channel_bytes(id)

// Tagged blocks
TaggedBlock { signature /* 8BIM | 8B64 */, key, data, padding: Option<Vec<u8>> }
TaggedBlock::parsed() -> Option<Result<BlockData>>
BlockData::{UnicodeName, SectionDivider, LayerId, NameSource, BlendClippedAsGroup,
            BlendInteriorElements, Knockout, TransparencyShapes, LayerMaskHidesEffects,
            VectorMaskHidesEffects, Protection, SheetColor, FillOpacity, MetadataSetting}
constructors: unicode_name, section_divider, layer_id, name_source, blend_clipped_as_group,
              blend_interior_elements, knockout, transparency_shapes, layer_mask_hides_effects,
              vector_mask_hides_effects, protection, sheet_color, fill_opacity
TaggedBlock::check_structure() -> Result<()>   // strict inner re-parse of PlLd, SoLd, SoLE, lnk2/3/D, lfx2
tagged::uses_long_length(version, key)   // PSB 8-byte length keys (the spec's 13 + lnk3, lnkE, pths, extd, extn, FELS, cinf, artd)
LayerInfo::unpadded_len(version), LayerInfo::pad_to(version, 4)   // Photoshop pads the layer info to 4

// Image resources
ImageResource { signature, id, name, data };  ImageResource::parsed() -> Option<Result<ResourceData>>
ResourceData::{ResolutionInfo, LayerState, LayerGroupInfo, Thumbnail, GlobalAngle, IccProfile,
               GlobalAltitude, VersionInfo, Exif, Xmp, TimelineInfo, Animation, LayerSelectionIds}

// Legacy animation syntax (no playback/document-model dependency)
animation::AnimationResource::from_bytes / to_bytes  // maniIRFR; ordered raw records
animation::AnimationRecord::descriptor()              // AnDs; version-16 descriptor
animation::{frame_info, frame_sets, layer_frame_states} // borrowed sparse views, opaque ids
animation::parse_descriptor                          // descriptor + up to 3 zero alignment bytes
metadata::MetadataItem::animation_descriptor()       // mlst / tmln; leaves other metadata alone

// Compression
Compression::{Raw, Rle, Zip, ZipPrediction, Unknown(u16)}
compression::{encode_planes, decode_planes, PlaneLayout, packbits::{encode, decode}, predict, unpredict}
pixels::{samples_u16, samples_f32, u16_to_bytes, f32_to_bytes, unpack_bits, plane_to_u8}

// Descriptors
descriptor::{Descriptor, VersionedDescriptor, Value, Id, Class, ReferenceItem, UnicodeString, ObjectArray}
Descriptor::from_bytes / to_bytes;  VersionedDescriptor::from_bytes / to_bytes (version 16)

// Text engine data (`EngineData` in `TySh`, and the document's `Txt2`)
engine_data::parse(&[u8]) -> Result<Value, ParseError>;  parse_lossy(&[u8]) -> Value;  write(&Value);  write_bare(&[(String, Value)])
engine_data::Value::{Dict, Array, String, RawString, Int, Real, Bool, Name}   // RawString is written back as the same bytes
engine_data::first_style(&Value) -> TextStyle   // font, size, fill, justification, tracking, leading, faux styles, box text
engine_data::minimal(text, font, size, rgb, justification) -> Vec<u8>

// Layered TIFF (tags 37724 and 34377)
tiff::ImageSourceData::from_bytes / to_bytes(ByteOrder) / into_psd(header, resources, image_data);  tiff::resources_from_bytes / resources_to_bytes
tiff::layered::{TAG_IMAGE_SOURCE_DATA, TAG_IMAGE_RESOURCES, FILTER_MASK_DEFAULT}
ImageSourceData::for_tiff(&mut PsdFile, ifd_alpha)   // Photoshop's conventions: Patt, FMsk, MTrn, LMsk, per-layer blocks
ImageSourceData::drop_photoshop_padding()            // on open, so the padding does not pile up
tiff::layered::ifd_channel_resources(&mut resources, IfdChannels { color, merged_alpha, write_alpha, extra, kept })
tiff::layered::{interleaved_to_planar, planar_to_interleaved, swap_plane, strip_foreign_order_blocks, alpha_identifiers}

// Builder
PsdBuilder::new(w, h).color_mode(..).depth(8|16).version(..).compression(..).resolution(dpi).icc_profile(..)
builder.push_layer(LayerSpec) / begin_group(GroupSpec) / end_group()? / composite(PixelData)
builder.build() -> Result<PsdFile>;  builder.to_bytes()
PixelData::{Rgba8, Rgba16, GrayA8, GrayA16, Cmyka8, Cmyka16}   // CMYK given as ink, stored inverted

// Test generator (feature `testgen`)
testgen::{all_cases, merged_only, layered, small, sample_descriptor, pattern_plane}
```

## Feature matrix

| Area | Read | Write | Typed | Notes |
|---|:-:|:-:|:-:|---|
| Header, PSD v1 / PSB v2 | ✓ | ✓ | ✓ | depth 1/8/16/32; all 8 color modes plus `Unknown` |
| Color mode data | ✓ | ✓ | raw | indexed palette is used by `composite_rgba8` |
| Image resources | ✓ | ✓ | partial | 1005, 1024, 1026, 1033/1036 (raw), 1037, 1039, 1049, 1057, 1058, 1060, 1069 |
| Layer records | ✓ | ✓ | ✓ | flags, clipping, filler, blending ranges (raw + accessor) |
| Layer masks | ✓ | ✓ | ✓ | 20 / 36-byte forms, mask parameters, real mask; unparseable masks stay raw |
| Channel ids -1 / -2 / -3 | ✓ | ✓ | ✓ | per-channel rect via `channel_rect` |
| Raw / RLE / ZIP / ZIP+prediction | ✓ | ✓ | – | prediction at 8/16/32 bits (32: byte-plane shuffle + delta); RLE counts are u16 in PSD, u32 in PSB |
| Merged image data | ✓ | ✓ | – | validated at parse time (sizes, RLE counts, zlib stream incl. checksum); a zlib stream declaring more than `MAX_DECODED_BYTES` is kept verbatim without inflating it, and decoding it reports the limit |
| Tagged blocks (raw passthrough) | ✓ | ✓ | – | keeps `8BIM` / `8B64` and exact padding |
| PSB 8-byte-length keys | ✓ | ✓ | – | LMsk Lr16 Lr32 Layr Mt16 Mt32 Mtrn Alph FMsk lnk2 FEid FXid PxSD |
| Lr16 / Lr32 / Layr layer info | ✓ | ✓ | ✓ | lifted into `layer_info`; re-inserted at its original position |
| luni lsct lsdk lyid lnsr clbl infx knko tsly lmgm vmgm lspf lclr iOpa shmd | ✓ | ✓ | ✓ | `TaggedBlock::parsed()` |
| Layer XMP (`shmd` › `cust` › `layerXMP`) | ✓ | ✓ | ✓ | `metadata::layer_xmp` / `take_layer_xmp` / `put_layer_xmp`, laid out as Photoshop 2026 writes it |
| ActionDescriptor (all OSTypes) | ✓ | ✓ | ✓ | including references, `ObAr` (best effort), `UnFl`, `Pth `, version-16 wrapper |
| Legacy frame animation syntax | ✓ | ✓ | partial | `mani` / `AnDs`, `mlst`, `tmln`, resource 1075; raw-preserving descriptor views, not playback or keyframe evaluation |
| Layer tree (groups) | ✓ | ✓ | ✓ | open/closed folders plus bounding dividers; tolerant of malformed nesting |
| RGBA8 extraction | ✓ | – | – | RGB / gray / CMYK layers; merged image also supports indexed / bitmap / duotone |
| Builder | – | ✓ | – | RGB / gray / CMYK at 8/16 bit, masks, groups, blend modes, opacity, fill, visibility, clipping |
| Patterns | ✓ | ✓ | ✓ | `Patt` / `Pat2` / `Pat3`, `.pat`, and `.abr` embedded patterns; non-empty tiles allow in-bounds channel subrectangles, decoded size is capped and PackBits expansion is input-bounded |
| Actions (`.atn`) | ✓ | ✓ | ✓ | `atn::parse` / `atn::write`: version-16 sets (Photoshop 6+): actions with function key, modifiers and colour; steps with event code or string name, panel name, enabled/dialog flags and their descriptor. Exact round trip for version-16 files, except that flag and Boolean bytes other than 0 or 1 are written back as 1 and trailing bytes after the last action are dropped (with a parse warning); version-12 (Photoshop 5) files are read too and written back as version 16 |

Pattern channel subrectangles are padded with zeroes to the tile rectangle. All channel
metadata is checked before any plane is decoded, against one `PatternBudget` shared by every
pattern of a parse (a block, a `.pat` file): `MAX_DECODED_BYTES` decoded and 512 MiB of zero
padding in total. `parse_pattern_block_with` lets several blocks of one document share a budget;
`.abr` patterns are charged to the file's sample budget. All channels of a pattern must have the
same depth. Fully populated channels reuse their decoded buffer without a second copy;
allocations for padded channels return an error on failure.

### Preserved raw, not yet typed (parsers planned for M8)

* Layer effects: `lfx2`, `lrFX`, `lmfx`. The descriptors inside can be parsed on demand with `VersionedDescriptor::from_bytes(&block.data[4..])`.
* Type layers: `TySh`, `tySh`. The EngineData inside them has a parser and writer (`engine_data`); the blocks themselves are not typed.
* Smart objects: `SoLd`, `PlLd`, `SoLE`, `lnk2`, `lnkD`, `lnk3`.
* Shapes: `vogk`, `vscg` (vector masks, `vmsk` / `vsms`, are typed by `path`).
* Adjustment and fill layers: `curv`, `levl`, `hue2`, `SoCo`, `GdFl`, `PtFl`, …, plus `Txt2`, `FMsk`, 3D and video blocks.
* Global layer mask info (raw, with accessors), thumbnails (raw), guides, and every other resource id except the ones `ImageResource::parsed()` types (resolution info, layer state, layer group info, global angle and altitude, version info, Exif, XMP, layer selection ids, timeline info, `mani` animation), paths (`path`) and slices (`slices`).

## Legacy animation inspection

```sh
cargo run -p craft-psd --example animation_inspect -- /path/to/input.psd
cargo test -p craft-psd --test it animation::
```

The inspection example accepts an explicit local PSD/PSB (up to 128 MiB), reports frame counts,
stored delays and descriptor counts, and checks byte-exact raw round-trips without modifying the
input. It does not print layer names, text or descriptor values. No personal file is bundled;
committed animation tests construct synthetic payloads and PSD/PSB containers.

`mani` is detected by its payload signature within the plug-in resource range 4000–4999, not by
assuming that resource 4003 always belongs to animation. Unknown records and unsupported descriptor
versions stay raw. `FrIn` definitions and `FSts` playback order are separate; ids are opaque stored
`long` values. Missing frame delays remain absent, and sparse `mlst` properties are not filled with
guessed defaults. Timeline tracks, loops, inheritance and property evaluation are left to higher
layers. This parser alone does **not** make PSD animations play.

## Testing

`cargo test -p craft-psd --all-features` runs the unit tests, integration tests and proptests:

* Byte stability and model round-trips for every generated case: all depths, color modes and compressions, PSD and PSB.
* PackBits edge cases.
* Known vectors for 8/16/32-bit prediction.
* PSD vs PSB length handling.
* Truncation sweeps: every prefix of several small files must return `Err`.
* Random-mutation and random-byte proptests: never panic.
* Layer tree cases.
* Builder → `rgba8` correctness.
* Unknown blend modes and unknown blocks pass through unchanged.

`tests/it/corpus.rs` and `tests/it/corpus_atn.rs` (feature `corpus`) walk a real-file corpus that is not in this repository: `corpus/psd`, `corpus/psd-tools` and `corpus/atn` at the workspace root, or under `CRAFT_PSD_CORPUS` (a PhotoCraft checkout's `corpus` directory). For each file they assert that parsing succeeds and the file round-trips byte for byte. With the feature on and the corpus missing they fail. The one known-bad upstream file, `group-divider-blend-mode.psd`, is reported but does not fail the test (`KNOWN_BAD`). Add `-- --nocapture` to see the per-file results.

A `cargo-fuzz` skeleton lives in `fuzz/`. It is its own workspace and is excluded from the craft-libs workspace. Run it with `cargo +nightly fuzz run parse` (or `descriptor`) from `crates/psd`.

## Spec ambiguities and decisions

* **Tagged block padding.** The spec says lengths are "rounded up to an even byte count". In practice writers either include the padding in the length or append it after the data, and pad to 2 or 4. The parser detects the padding that actually follows each block: the smallest k ≤ 3 after which the next signature or the end of the region appears. It stores that padding unless it matches the default.
* **Layer info padding** (2 vs 4) is recorded the same way.
* **16/32-bit layers** are read from global `Lr16` / `Lr32` blocks when the main layer info is empty. The block data is a layer-info body with no inner length field.
* **Mask record layout.** The order is: rect, default color, flags, parameters (when flag bit 4 is set), then the real flags, background and rect whenever at least 18 bytes remain. Anything left over (for example the 2 padding bytes of the 20-byte form) is kept in `trailing`.
* **Mask channel rects.** Channel -2 uses the mask rect. Channel -3 uses the real mask rect.
* **Merged ZIP data** is a single zlib stream over all planes. Prediction is applied per row across all planes.
* **ZIP prediction at depth 1** is treated as byte-wise delta.
* **`ObAr`** is read as a u32 prefix followed by a descriptor-shaped body, following ag-psd's reading. The spec does not document it.
* **Descriptor `bool`** bytes other than 0/1 are normalized to 1 on write.
* **Pascal names** have their pad bytes written as zeros. A layer & mask section that contains only a zero layer-info length is normalized to an empty section. These are the only known non-byte-exact cases, and both are pathological.
* **Header limits.** The parser accepts up to 300 000 × 300 000 for both versions. `PsdFile::validate()` enforces the spec's 30 000 limit for PSD.
