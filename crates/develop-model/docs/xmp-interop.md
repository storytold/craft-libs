# XMP sidecars and interchange

LightCraft keeps its catalog as the source of truth, and can also store each photo's metadata and develop settings in
an XMP sidecar next to the original. Sidecars let edits travel with the files (backups, another LightCraft library,
other tools), and let LightCraft pick up edits made elsewhere.

## Sidecar files

| | |
|---|---|
| Name | `<stem>.xmp` by default (`IMG_0001.CR3` → `IMG_0001.xmp`); `<file>.xmp` (`IMG_0001.CR3.xmp`) with `naming: "full"`. Reading accepts either (preferred first) and `.XMP`. With stem naming, files sharing a stem don't share a sidecar: see *Shared names* below. |
| Write | `photo.saveMetadataToFile {ids?}` (Photo ▸ Save Metadata to File, ⌘S), or automatically after every change with `library.xmpPreferences {autoWrite: true}` (File ▸ Automatically Write Changes into XMP). Slider drags are written once, when the drag ends; undo/redo rewrite the sidecar. An existing sidecar is **merged into, never replaced** (see *Saving into an existing sidecar*). Written atomically (temp file, fsync, rename). The result lists `written`, `merged` and `backups`. |
| Read | On import (`library.import`, the report counts `sidecars`), and `photo.readMetadataFromFile {ids?}` (one undo step). Manual rereading uses embedded XMP in raw/DNG and JPEG files when no sidecar exists. A discovered sidecar that cannot be read or parsed is reported in `failed`, without applying embedded values. `photo.readMetadataFromFile` never writes the sidecar, even with auto-write on: writing back what it has just read would add `lc:settings`, which later reads prefer over another app's `crs:` edits (issue #934). A file arriving in an import is ranked as if it were catalogued already: it reads its own `<file>.xmp` when another file of its stem (a raw first) owns the shared stem sidecar (issue #1905). |
| Preferences | `library.xmpPreferences {autoWrite?, naming?: stem\|full}`, stored in the library's `prefs.json`. |

What we write (standard namespaces, so other tools can read the metadata):

| Field | XMP property |
|---|---|
| Rating 0–5 | `xmp:Rating` |
| Colour label | `xmp:Label` (`Red`, `Yellow`, `Green`, `Blue`, `Purple`) |
| Title / caption / copyright / creator | `dc:title` / `dc:description` / `dc:rights` / `dc:creator` |
| Copyright status | `xmpRights:Marked` (`True` copyrighted, `False` public domain, absent = unknown) |
| Rights usage terms / copyright info URL | `xmpRights:UsageTerms` / `xmpRights:WebStatement` |
| Keywords | `dc:subject` (names: `Places`, `Italy`, `Rome`) and, when any keyword has levels, `lr:hierarchicalSubject` (paths: `Places\|Italy\|Rome`, with the flat keywords too) |
| Capture time, GPS | `exif:DateTimeOriginal`, `photoshop:DateCreated`, `exif:GPSLatitude`/`GPSLongitude` |
| Pick / reject flag | `lc:flag` (`pick`, `reject`, `none`) |
| Location | `lc:location` |
| Develop settings | `lc:settings` — our complete `DevelopSettings` as JSON (exact round trip, incl. masks, spots, crop and the rendering process; settings without `process` are V1, see [process-versions.md](process-versions.md)) |

`lc:` is `http://ns.lightcraft.app/lc/1.0/`.

Reading merges into the catalog with the **sidecar winning** for every field it states; fields it doesn't state are
kept. The one exception is the **capture time**: a valid time embedded in the file (EXIF/IPTC) wins. Invalid EXIF
clock values are skipped, allowing DateTimeDigitized, IFD0 DateTime or embedded XMP to supply the time. If the file
has no valid capture time, the sidecar's `exif:DateTimeOriginal`, else `photoshop:DateCreated`, else `xmp:CreateDate`
(ISO 8601) is used — on import (it then also files a copied photo in its date folder) and by
`photo.readMetadataFromFile` (undoable). An incomplete or malformed XMP timestamp is not projected as a capture time,
remains available in the raw property map, and does not prevent the next valid capture-time property from being used.
`xmp:Rating="-1"` (the XMP convention for rejected) sets the reject flag. Develop settings come from
`lc:settings` when present (exact); otherwise from the `crs:` fields below (approximate).

When foreign `crs:` settings are only partly transferable, `library.import` includes per-photo `warnings` in its
report. Warnings identify unsupported profile names, saved Upright geometry that will be recalculated, and other
unmapped adjustments; they do not change the source packet or become catalog edits.

Manual rereading and undo leave the JPEG original unchanged. Manual rereading also leaves sidecar files unchanged,
even with automatic XMP writing on. With automatic XMP writing off (the default), undo/redo leave sidecars unchanged;
with it enabled, subsequent edits and undo/redo can trigger the existing sidecar writer. `photo.allMetadata` uses
its already loaded file bytes for embedded XMP, with an existing sidecar taking precedence.

Manual JPEG rereading uses the first standard XMP APP1 before the first SOS (scan) or EOI (end) marker.
Scanning is limited to a 16 MiB header span, including SOI, segment headers and skipped payloads, and 4096
marker/fill steps after SOI. Non-APP1 payloads are skipped without loading them. Each APP1 payload must be
complete and is at most 65,533 bytes. Header extraction succeeds for the first complete
standard XMP packet within both limits, including a packet ending exactly at either limit; otherwise it
fails without a whole-file fallback, including on I/O errors or an incomplete APP1 whose available bytes
form parseable XMP. A valid JPEG with
XMP beyond those limits therefore cannot be manually reread through this path. Extended XMP assembly and
packets between scans are not supported by manual JPEG rereading.

Packet completeness is stricter than the shared best-effort JPEG extractor; it is not strict whole-JPEG
validation. A complete early XMP packet can be returned without examining the remaining file. Import and
`photo.allMetadata` retain their existing extraction paths. Command regressions in
`crates/engine/src/tests/xmp.rs` cover metadata/undo and sidecar precedence, an over-nested XMP packet and a
truncated APP1 without an XMP signature. Separate reader cases in `sidecar::jpeg_header_tests` use small
counted readers and injected limits for span/step boundaries, complete versus truncated APP1, early stopping,
short reads and I/O errors.

## Saving into an existing sidecar

A sidecar may already hold another application's data — e.g. its `crs:` develop settings and `xmpMM:History` — often
the only copy of those edits outside that application's catalog. Saving never drops it:

- LightCraft **owns** the properties in the table above: `xmp:Rating`, `xmp:Label`, `dc:title`, `dc:description`,
  `dc:rights`, `dc:creator`, `dc:subject`, `lr:hierarchicalSubject`, `Iptc4xmpCore:Location`, `Iptc4xmpCore:AltTextAccessibility`,
  `Iptc4xmpCore:ExtDescrAccessibility`, `photoshop:City`/`State`/`Country`, `xmpRights:Marked`/`UsageTerms`/
  `WebStatement` and everything in `lc:`. They are replaced on every save, and removed when LightCraft has no value
  (clearing a title in LightCraft clears it in the file). All of them are read back on import, so the library starts
  from what the sidecar said.
- The **capture time** (`exif:DateTimeOriginal`, `photoshop:DateCreated`) and **GPS** (`exif:GPSLatitude`,
  `exif:GPSLongitude`) are replaced only when LightCraft has a value; otherwise the file's stay.
- **Everything else is kept byte for byte**: other namespaces (`crs:`, `xmpMM:`, the rest of `lr:`, unknown
  ones), `xmp:CreatorTool`, comments, the packet wrapper and padding. LightCraft's properties go into one
  `rdf:Description` of their own; owned properties written by another application (in attribute or element form) are
  removed from its description, which otherwise stays as it was.
- Ownership follows the namespace URI, not the prefix spelling. A foreign namespace bound to `lc:` or `xmp:`
  stays foreign: its properties are preserved and do not become LightCraft settings or ratings. Genuine namespace
  aliases remain interchangeable, including namespace declarations containing XML character references.
- A sidecar that can't be read as XMP (not well-formed, not UTF-8, no `rdf:RDF`) is first copied to
  `<name>.xmp.bak-<date><time>` (`-1`, `-2`… if taken — a backup is never overwritten), then replaced. A sidecar that
  can't be read at all (permissions) is left alone and the save fails.

Keywords are written the way Lightroom Classic and darktable write them, so other applications can search them: names
in `dc:subject`, paths in `lr:hierarchicalSubject` (`keywords::to_file`; read back by `keywords::from_file`, which gives
the same keywords). No export option applies to a sidecar, so no keyword is left out and no synonym added. Sidecars
from LightCraft 0.13 and older have paths in `dc:subject`; they still read as paths, and the next save writes them this
way.

Limits: LightCraft doesn't write `crs:`: the other application's
develop settings stay as that application left them, next to LightCraft's own (`lc:settings`, which LightCraft reads
first).

## Shared names

With stem naming, `IMG_0001.CR3` and `IMG_0001.JPG` map to the same `IMG_0001.xmp`. When two or more files in the
library share a stem, the stem sidecar belongs to one of them — a raw first, otherwise the first by file name — and
the others write and read `<file>.xmp` (`IMG_0001.JPG.xmp`), falling back to the stem sidecar for reading when they
have none. So saving one photo never overwrites the other's metadata (`Session::sidecar_naming`).

An import ranks the files it is adding the same way, before they are catalogued: a TIFF or JPEG arriving beside a
raw of its stem reads its own `<file>.xmp` and takes nothing from the raw's `IMG_0001.xmp`, whether it arrives
with the raw or after it (issue #1905).

## Face regions (MWG)

LightCraft reads face, pet, focus and barcode regions in the Metadata Working Group's region schema
(`mwg-rs:Regions`, `http://www.metadataworkinggroup.com/schemas/regions/`), as Lightroom, digiKam, Picasa and others
write them, from sidecars and from XMP embedded in the file. Named faces become people in the People view
(LR-LIB-PEOPLE in [`parity-checklist.md`](parity-checklist.md)), and every region is drawn as a box in the loupe.

- **Read only.** LightCraft never writes `mwg-rs:Regions`; saving a sidecar keeps another application's regions byte
  for byte. Removing or resizing a box in the loupe changes the library only.
- **Areas**: `stArea:x`/`y` are the box's centre, `w`/`h` its size, normalized to the photo (`stArea:unit="pixel"`
  areas are divided by `mwg-rs:AppliedToDimensions`, and dropped without it). Boxes are clipped to the photo; one
  that misses it entirely, or has a non-finite, zero or negative size, is skipped (the rest of the list is kept).
- **Orientation**: regions are stored on the upright (EXIF-oriented) photo. A `mwg-rs:Rotation` of −π/2, +π/2 or ±π
  (how Lightroom marks a box given in the sensor's frame for Exif orientation 6, 8 or 3) is turned back into the
  upright frame; MWG can't say "mirrored", so Lightroom's boxes on mirrored photos (orientations 2, 4, 5, 7) are
  taken as written. Other applications mark boxes drawn on the photo as stored only by their size: when
  `mwg-rs:AppliedToDimensions` has the shape of the photo as stored rather than upright (a photo that Exif orientation
  5–8 turns a quarter turn, and no `mwg-rs:Rotation`), the boxes are turned with the file's own Exif orientation,
  mirror included, when imported and when read again (#2674). The image's own pixel size decides, not a size the
  metadata states, and the orientation is found wherever the file keeps its Exif (WebP after the image data, for
  one). The exception is a raw with a sidecar that has no `mwg-rs:Regions`: its boxes come from the file and the size
  its Exif states decides, and with no such size they are taken as written. A square photo or size looks the same both
  ways and is taken as written. Boxes follow LightCraft's own Rotate Left/Right and flips.
- **Re-reading**: a sidecar that has `mwg-rs:Regions` — even with an empty list — replaces the photo's regions, so
  regions removed in another application go away here too. A sidecar without `mwg-rs:Regions` (an application that
  doesn't do regions) leaves the photo's regions as they are.

## Reading `crs:` develop fields

Many raw developers store edits as `crs:` properties (`http://ns.adobe.com/camera-raw-settings/1.0/`) in sidecars, in
DNG files and in XMP presets. LightCraft reads the common ones and maps them to its own controls. We implemented this
from the public XMP specification and by observing what each field does; no third-party code or preset files were used.
Our pipeline renders differently, so **values carry over but the look is approximate**.

We read these fields; we never write them. Only fields in the packet are applied: the result is a partial settings
object that gets merged like a preset, so everything else keeps its current or default value. Packets marked
`crs:AlreadyApplied="True"` are skipped, because those pixels already contain the edit. Only process-version 2012+ field
names are read (e.g. `Exposure2012`, not the older `Exposure`). `crs:ProcessVersion` itself numbers the other
application's renderer and is ignored: the photo keeps LightCraft's own process, the latest for a newly imported photo
([process-versions.md](process-versions.md)).

| `crs:` field(s) | LightCraft control | Notes |
|---|---|---|
| `Exposure2012` | `light.exposure` | EV, 1:1 |
| `Contrast2012`, `Highlights2012`, `Shadows2012`, `Whites2012`, `Blacks2012` | `light.contrast` … `light.blacks` | −100..100, 1:1 |
| `HDREditMode` | `hdr.enabled` | `1` / `True` = HDR editing on, `0` / `False` off |
| `WhiteBalance` | `wb.mode` | `As Shot`, `Auto`, `Daylight`, `Cloudy`, `Shade`, `Tungsten`, `Fluorescent`, `Flash`; other names → custom |
| `Temperature`, `Tint` | `wb.temp`, `wb.tint` (+ `wb.scale: kelvin`) | Kelvin / tint as written for raws with colour matrices — in the file (DNG) or measured for the camera (`lightcraft_raw::spectral`: ILCE-7M3, D850, EOS 5D Mark III…, issue #730) or read from its white-balance presets (`lightcraft_raw::kelvin_presets`: ILCA-99M2, ILCA-77M2…) — and for presets; the mapped settings say `"scale": "kelvin"` so the value stays Kelvin wherever it lands. Raws LightCraft develops relative to their as-shot look (ARW, NEF, RW2, RAF, CR3, CR2, PEF, SRW of other cameras: 6500 K / 0 means as shot) take the same mired shift from the packet's `AsShotTemperature` / `AsShotTint` instead; without that reference a custom Kelvin stays As Shot and is reported (`Temperature, Tint (custom white balance without AsShotTemperature: kept As Shot)`), because read on the relative scale it would be a large colour cast (issue #510). Note that LightCraft's Kelvin for a camera comes from its measured matrices, Lightroom's from Adobe's: the two scales differ (about 7–10 % on the ILCE-7M3), so the number is carried over, not the exact look |
| `IncrementalTemperature`, `IncrementalTint` | `wb.temp`, `wb.tint` | rendered files: −100..100 on our relative scale (mired shift around 6500 K, same as the Temp slider); a rendered file with only `Temperature` shifts from `AsShotTemperature` like the raws above when it is present |
| `Vibrance`, `Saturation` | `color.vibrance`, `color.saturation` | 1:1 |
| `Texture`, `Clarity2012`, `Dehaze` | `effects.texture`, `effects.clarity`, `effects.dehaze` | 1:1 |
| `HueAdjustment<Band>`, `SaturationAdjustment<Band>`, `LuminanceAdjustment<Band>` | `mixer.<band>.hue/sat/lum` | bands Red, Orange, Yellow, Green, Aqua, Blue, Purple, Magenta |
| `ConvertToGrayscale` | `treatment` | `True` → B&W |
| `GrayMixer<Band>` | `bw_mix.<band>` | |
| `ParametricShadows`, `ParametricDarks`, `ParametricLights`, `ParametricHighlights` | `curve.shadows/darks/lights/highlights` | |
| `ParametricShadowSplit`, `ParametricMidtoneSplit`, `ParametricHighlightSplit` | `curve.split_shadows/split_mid/split_highlights` | |
| `CurveRefineSaturation` | `curve.refine_saturation` | 0..100 (100 = curve saturation unchanged) |
| `ToneCurvePV2012`, `ToneCurvePV2012Red/Green/Blue` | `curve.master/red/green/blue` | `"x, y"` points in 0..255 → 0..1; a straight 0→255 line = no curve; the red / green / blue curves are read only when `ToneCurvePV2012` and all three channel curves are present (Lightroom Classic ignores them otherwise: a master + red-only packet renders without the red curve) |
| `SplitToningShadowHue/Saturation`, `SplitToningHighlightHue/Saturation` | `grading.shadows/highlights.hue/sat` | |
| `ColorGradeShadowLum`, `ColorGradeHighlightLum` | `grading.shadows/highlights.lum` | |
| `ColorGradeMidtoneHue/Sat/Lum`, `ColorGradeGlobalHue/Sat/Lum` | `grading.midtones/global.*` | |
| `ColorGradeBlending`, `SplitToningBalance` | `grading.blending`, `grading.balance` | |
| `Sharpness`, `SharpenRadius`, `SharpenDetail`, `SharpenEdgeMasking` | `detail.sharpen_*` | |
| `LuminanceSmoothing`, `LuminanceNoiseReductionDetail`, `LuminanceNoiseReductionContrast` | `detail.nr_luminance/nr_detail/nr_contrast` | |
| `ColorNoiseReduction`, `ColorNoiseReductionDetail`, `ColorNoiseReductionSmoothness` | `detail.nr_color/nr_color_detail/nr_color_smoothness` | |
| `PostCropVignetteAmount/Midpoint/Roundness/Feather/HighlightContrast` | `vignette.amount/midpoint/roundness/feather/highlights` | |
| `PostCropVignetteStyle` | `vignette.style` | 1 highlight priority, 2 colour priority, 3 paint overlay |
| `GrainAmount`, `GrainSize`, `GrainFrequency` | `grain.amount`, `grain.size`, `grain.roughness` | |
| `LensProfileEnable`, `AutoLateralCA` | `optics.lens_profile`, `optics.remove_ca` | lens profiles are the file's own (DNG-embedded) corrections, which Lightroom applies whatever its profile switch says (our inference): only `LensProfileEnable=1` carries over, `0` leaves the switch as it is |
| `LensManualDistortionAmount`, `VignetteAmount`, `VignetteMidpoint` | `optics.distortion`, `optics.vignetting`, `optics.vignetting_midpoint` | |
| `DefringePurple/GreenAmount/HueLo/HueHi` | `optics.defringe_*` | |
| `ShadowTint`, `RedHue/Saturation`, `GreenHue/Saturation`, `BlueHue/Saturation` | `calibration.shadows_tint`, `calibration.red_hue/red_sat`, … | Calibration panel, 1:1 |
| `PerspectiveVertical/Horizontal/Rotate/Scale/Aspect/X/Y` | `geometry.vertical/horizontal/rotate/scale/aspect/offset_x/offset_y` | |
| `PerspectiveUpright` | `geometry.upright` | 0 off, 1 auto, 2 level, 3 vertical, 4 full, 5 guided; saved `UprightTransform_*` matrices are not imported, so photo import reports that Lightroom geometry was not preserved and the selected mode will be recalculated |
| `HasCrop`, `CropLeft/Top/Right/Bottom`, `CropAngle` | `crop.geometry` | normalized edges → rect; angle in degrees; `HasCrop="False"` → no crop |

Values pass through our control specs, so anything outside our slider ranges gets clamped.

`crs:CameraProfile` is kept as the selected name in the photo's import warning and the active profile is left as it is: the name carries no rendering data and never selects an Adobe profile. Import reports such a profile as not applied (colours may differ from Lightroom), except Lightroom's default profiles (`Adobe Standard`, `Adobe Color`), which every sidecar names and which LightCraft's own starting look stands in for.

**Profiles named by presets (`Look`, issue #1001).** A preset keeps the profile it names (`crs:Look`: `Name`, `Group`,
`UUID`, `Amount`; in `.lrtemplate` files the `Look` table) as `profile.look` in its settings. When the user has imported
a profile of that name (`profile.import`; the name compared ignoring case and repeated spaces, the group breaking
ties), applying the preset selects it at the look's amount (`Amount` 1 = 100 %). Lightroom's basic profiles map to our
own (`Adobe Color` → Color, `Adobe Neutral`, `Adobe Vivid`, `Adobe Landscape`, `Adobe Portrait`, `Adobe Monochrome`
→ Monochrome). Any other `Adobe …` / `Camera …` profile and the numbered creative ones (`Artistic 01`, `B&W 05`,
`Modern 10`, `Vintage 02`) can't be used here, and a third-party profile not imported yet leaves the photo's profile as it
is; import reports either per preset (`Profile “Name” (Lightroom's own)` / `(not imported yet)` in `unmapped`).
References are resolved again whenever profiles are imported or deleted, so a pack's profiles and presets can come in
either order. The look's embedded `Parameters` are not read.

**Not mapped:** camera profiles (`CameraProfile`; we have our own profile set), red eye, lens blur,
process-version 2010 field names, and AI features. Local adjustments and spot removal are read as described below.

## Spot removal

Heal and clone spots stored as `crs:RetouchAreas` (one structure per spot, a mask component inside) and the older
`crs:RetouchInfo` list (one `key = value, …` line per spot; read on its own when it is all a packet has) become our
spots (`crates/engine/src/crs_masks.rs`, `spots`). Implemented from the public data model and by observing what a
sidecar written for a healed photo contains.

| Field | Ours | Notes |
|---|---|---|
| `SpotType` | `mode` | `heal` (default) or `clone`; the removal method (`Method`) is ours |
| `Masks` → `Mask/Ellipse` (`X`, `Y`, `SizeX`) | a circle spot | centre in image fractions; `SizeX` is a fraction of the width, our size of the long edge |
| `Masks` → `Mask/CircularGradient` (`Top/Left/Bottom/Right`) | a circle spot | the box's centre and half-width |
| `Masks` → `Mask/Paint` (`Dabs`, `Radius`) | a brushed spot | the dabs are the path |
| `SourceX`, `OffsetY` | `source_offset` | the source's **absolute** x and y (despite the name); ours is the offset from the target. Only with `SourceState="sourceSetExplicitly"` (or no state); anything else leaves the source automatic |
| `Opacity`, `Feather` | `opacity`, `feather` | 0..1 → 0..100 |

A spot whose mask is any other kind is reported as `Spot: <kind>`. Sizes are clamped to our spot range. A sidecar's
spots replace the photo's, as its masks do.

## Local corrections (masks)

Masks stored as `crs:` structures are read from sidecars, DNG-embedded XMP, XMP presets and `.lrtemplate` files
(`crates/engine/src/crs_masks.rs`). Four containers hold them: `MaskGroupBasedCorrections` (current) and the older
`GradientBasedCorrections`, `CircularGradientBasedCorrections` and `PaintBasedCorrections`. Each correction becomes one
mask: `CorrectionName` → name, `CorrectionAmount` → Amount, inactive corrections are skipped.

| Field | Ours | Notes |
|---|---|---|
| `LocalExposure2012` | `adjust.exposure` | stored as a fraction, ×4 EV |
| `LocalContrast2012`, `LocalHighlights2012`, `LocalShadows2012`, `LocalWhites2012`, `LocalBlacks2012`, `LocalClarity2012`, `LocalTexture`, `LocalDehaze`, `LocalTemperature`, `LocalTint`, `LocalSaturation`, `LocalHue`, `LocalSharpness`, `LocalLuminanceNoise`, `LocalMoire`, `LocalDefringe`, `LocalToningSaturation` | the matching `adjust.*` | stored −1..1, ×100 |
| `LocalToningHue` | `adjust.color_hue` | degrees |
| other `Local*` sliders (`LocalGrain`…) | — | no counterpart in our masks: reported as `Mask: LocalGrain` when set (`LocalCurveRefineSaturation` 100 is neutral) |
| `Mask/Gradient` (`FullX/Y`, `ZeroX/Y`) | linear gradient | full effect at the Full point |
| `Mask/CircularGradient` (`Top/Left/Bottom/Right`, `Angle`, `Feather`, `Flipped`) | radial gradient | box fractions → long-edge radii (presets assume 3:2; sidecars use the photo's shape). `Flipped="true"` means effect inside: in `MaskGroupBasedCorrections` with `Version` 2 (what Lightroom writes for an ordinary radial) and in the legacy `CircularGradientBasedCorrections` Radial Filter, whose default (no `Flipped`) is outside. A `MaskGroupBasedCorrections` radial without `Version` is inside whatever `Flipped` says, and its `Feather` is halved (Lightroom rescales it that way when it upgrades one). Inverting is `MaskInverted` |
| `Mask/Paint` (`Dabs` "d x y", `Radius`, `Flow`, `CenterWeight`, `MaskValue`) | brush | one brush component per correction; `MaskValue` ≤ 0 erases |
| `Mask/Image` `MaskSubType` 1 / 2 | Subject / Sky | other AI selections are reported, not guessed |
| `Mask/RangeMask` `Type` 2 / 3 (`LumRange`, `DepthRange`) | luminance / depth range | lightness converted to our range scale; a range at full strength at white (third `LumRange` value 1) stays open above our scene white; colour ranges are reported |
| `MaskBlendMode` 0 / 1 / 2, `MaskInverted` | add / subtract / intersect, invert | |

Applying a preset adds its masks to the photo's own (masks the photo already has are not added twice), and the Amount
slider scales them. A sidecar's masks replace the photo's, because a sidecar holds the whole edit. Components we can't
carry over are listed in `preset.import`'s `unmapped` as `Mask: <kind>`.

## Local adjustment presets (mask effects)

`mask.importEffects {path | paths, group?, dryRun?}` (and `preset.import`, File ▸ Import Profiles & Presets…, drag & drop)
reads these into the Masking panel's Effect menu (`crates/engine/src/cmd/mask_effect_files.rs`):

| File | How it is read |
|---|---|
| Lightroom Classic `.lrtemplate` (`%APPDATA%\Adobe\Lightroom\Local Adjustment Presets`, `~/Library/Application Support/Adobe/Lightroom/Local Adjustment Presets`) | a Lua table with `type = "LocalizedAdjustmentPreset"`; `title` (else `internalName`, else the file name) → name; `value` holds the `Local*` field names in lower camel case: `exposure2012` → `LocalExposure2012`, mapped as in the table above (`exposure`, `contrast`, `clarity` are the older process version's; the 2012 fields win); `luminanceNoise` → Noise, `toningHue` / `toningSaturation` → Color |
| Camera Raw local-correction `.xmp` (`CameraRaw/LocalCorrections`) | top-level `crs:Local*` properties (or, in a file with `crs:PresetType="Local"` without them, its first correction's); `crs:Name` → name, `crs:Group` → group. Develop presets (no `crs:Local*`) stay develop presets |
| `.lceffect` | ours: `{"format": "lightcraft.maskEffects", "version": 2, "effects": [{name, adjust, group?, favorite?, fields?, with_amount?, brush?, extra?}]}` (a bare array or one effect also reads); `mask.exportEffects {path, names?, group?}` writes it |
| `.zip` | the presets above inside it; its folders give groups |

Groups: a file's folder (relative to the imported folder; names that only say "local adjustment presets" don't count)
names its group, unless the file names one. Lightroom users mark sections of the flat preset menu with **divider
presets**: presets that set nothing, named like `10 ----- DETAIL ENHANCERS ----- 10` (a run of three or more `-`, `=`,
`_`, `*`, `#`, `~`, `—`, `.`). These become groups instead of effects: a preset whose name starts with a number goes to
the divider with the largest number not above it (`12 Sky` under `10 ----- DETAIL ENHANCERS ----- 10` → *Detail
Enhancers*); presets without a number keep their folder's group. An earlier import's divider effects are removed.
Same-named effects are replaced (a star is kept).

Settings with no local slider here are listed per preset in `unmapped` (and the toast) and **kept** with the effect in
`extra` (an `.lceffect` export carries them), but not applied: `grain` (Grain; LightCraft has no local grain),
`pointColors` (Point Color: `SrcHue/Sat/Lum`, `HueShift`, `SatScale`, `LumScale`, `RangeAmount`, `Hue/Sat/LumRange`
`{LowerNone, LowerFull, UpperFull, UpperNone}`; there is global Point Color only), `toningLuminance`. Zero values aren't
listed (`refineSaturation = 0` in most files). Lightroom keeps brush size, feather, flow and density per brush, not in
presets, so there is nothing to import for them; an effect saved here can carry them (`brush`).

Applying (`mask.applyEffect {name, id? | ids? | all?, photos?: "selected"}`) sets the sliders the effect carries —
all of them for an imported preset (unnamed ones to 0, as Lightroom does), or only the ones chosen when it was saved
(`mask.saveEffect {fields: "changed" | [..]}`) — and Amount only if it carries it. The effect is remembered for the
mask's kind: the next brush / linear / radial / … mask starts with it (`mask.toolEffect`).

## Preset files

| | |
|---|---|
| Ours: `.lcpreset` | JSON `{"format": "lightcraft.preset", "version": 1, "presets": [{id, name, group, settings}]}`, where `settings` is a partial develop-settings object (only the groups the preset includes). A file can hold one preset or many, and every preset keeps its group. Import also accepts a bare preset object or an array of them. |
| Export | `preset.export {path, ids?, group?}`: all user presets by default, or the given ids or one group. In the app: File ▸ Export Presets…, Presets panel ▸ ⋯ ▸ Export User Presets…, or right-click a group ▸ Export Group…. |
| Import | `preset.import {paths, group?, dryRun?}`: files or folders (recursive): `.lcpreset`, `.xmp`, classic `.lrtemplate` (a Lua table: `value.settings` holds the same field names as `crs:`), photos that carry their edits in XMP ("DNG presets" from mobile apps; their crop, geometry and custom white balance are left out) and `.zip` bundles of any of these. Local adjustment (brush) presets — Lightroom Classic's `.lrtemplate` with `type = "LocalizedAdjustmentPreset"`, Camera Raw's local-correction `.xmp`, our `.lceffect`, also inside zips — become mask effects (`mask.effects`) instead (see *Local adjustment presets* below); the result lists them under `effects`, and divider presets under `effectDividers`. Presets in a folder (or a folder inside a zip) go to a group named after it, unless the file names its own group. The result lists, per preset, the settings that couldn't be carried over (`unmapped`, e.g. `CameraProfile`, a profile named by `Look` that isn't imported, local masks), and the app's toast names them; fields that only describe the preset or the photo (`Cluster`, `SortName`, `Description`, `SupportsAmount2`, `RequiresRGBTables`, `CropConstrainToWarp`, `OverrideLookVignette`, `AsShotTemperature` / `AsShotTint`, empty `PointColors` slots) aren't listed. Older (process version 2010) fields — `Exposure`, `Contrast`, `FillLight`, `HighlightRecovery`, `Shadows`, `Brightness`, `Clarity`, `ToneCurve` — are approximated with today's sliders when a preset has no 2012-era fields. Dropping preset files on the window imports them too. In the app: File ▸ Import Presets…, or Presets panel ▸ ⋯ ▸ Import Presets…. A preset that's already there (same name, group and settings) is skipped. If an id clashes, the import gets a fresh `user.*` id, and built-in presets are never replaced. |
| XMP presets | Read with the `crs:` table above, with `crs:Name` as the name (falling back to the file name) and `crs:Group` as the group (falling back to "Imported Presets"). Only the fields the preset sets are included, so applying it leaves everything else alone and the Amount slider scales it like any other preset. We only read XMP presets; we don't write them. |

LightCraft ships no third-party presets. Its built-in presets are its own values (`crates/engine/src/presets.rs`).

## Luminar looks (`.lmp`, `.mplumpack`)

`preset.import` (and drag & drop, File ▸ Import Profiles & Presets…) also reads Luminar looks: an `.lmp` file is an
XML property list of adjustment layers (each with effects and named sliders, mostly on a −100..100 scale); newer looks
may be a bundle folder `Name.lmp/Contents/preset.lmp`. An `.mplumpack` collection is a zip of `.lmp` files whose
`PresetsInfo.plist` names the group (`GroupName`); the pack's file name is the fallback. Looks get their own name
(`Name`) or their file / bundle name. The sliders with a clear counterpart are carried over; everything else (AI tools,
Orton, glow, LUT layers, colour balance…) is listed in `unmapped` as `Tool.Slider` (e.g. `OrtonFilter.Amount`).
Disabled layers are ignored, layer opacity scales the sliders (and fades curves toward a straight line), and layers
with a blend mode other than Normal or with a mask are reported, not applied. Binary property lists are not read.

| Luminar tool.slider | Ours |
|---|---|
| Develop / Light / Exposure `Exposure` (±100) | `light.exposure` (±4 EV — the scale is our reading, not documented) |
| `Contrast`, `Highlights`, `Shadows`, `Whites`, `Blacks` | `light.*` (same scale) |
| `Temperature`, `Tint` (relative) | white balance shift as `crs:IncrementalTemperature` / `IncrementalTint`; values above 1000 as Kelvin |
| `Saturation`, `Vibrance` | `color.saturation`, `color.vibrance` |
| Clarity `Clarity`, Structure / AI Structure `Amount` | `effects.clarity` (summed) |
| Dehaze `Amount` | `effects.dehaze` |
| HSL (`MIPLChannelsEffect`) `h`/`s`/`l` + Red…Magenta | `mixer.<band>.hue / sat / lum` |
| Curves `RGB` / `Red` / `Green` / `Blue` (x, y points in 0..1) | `curve.master / red / green / blue` |
| Vignette `Amount`, `Vignette Size` | `vignette.amount`, `vignette.midpoint` |
| Grain `Amount` | `grain.amount` |

## Profiles: 3D LUTs (`.cube`)

`profile.import {paths}` (File ▸ Import Profiles & Presets…, drag & drop) reads `.cube` 3D LUTs — single files, folders
or `.zip` bundles — as creative profiles: they appear in the profile browser under their folder's (or zip's) name and
take the Amount slider (0–200 %) like the built-in looks. The LUT is applied to the finished, display-encoded colour
(trilinear; `DOMAIN_MIN` / `DOMAIN_MAX` honoured; 1D LUTs are not supported); photos with a LUT profile render on the
CPU. A library on disk keeps a copy of each file in its `Profiles/` folder. XMP camera/creative profiles with embedded
tables are not read.

## Profiles: camera profiles (`.dcp`)

The same command (and File ▸ Import Profiles & Presets…, drag & drop) reads DNG Camera Profiles — the user's own `.dcp`
files, e.g. made with a colour-checker tool — as camera profiles (`dcp:<camera>-<file>` ids, group *Camera Profiles*,
listed by `profiles.camera`). Read per DNG specification chapter 6 (`crates/raw/src/dcp.rs`): `ProfileName`,
`UniqueCameraModel`, `CalibrationIlluminant1/2`, `ColorMatrix1/2`, `ForwardMatrix1/2`, `ProfileHueSatMap*`,
`ProfileLookTable*`, `ProfileToneCurve` and `BaselineExposureOffset`. Choosing one (`develop.profile {id}`) replaces a raw
file's colour model when it is decoded — the file's own white balance, analog balance and baseline exposure stay, and a
profile without a `ProfileToneCurve` (most chart-made ones) keeps the photo's own contrast: the DNG's tone curve, else the
tone of the look fitted to its camera JPEG — so the
photo's source is decoded again when its profile changes; rendered (non-raw) photos ignore it. Smart previews and the
web build's render worker keep the file's own colour. A library on disk keeps a copy of each file in `Profiles/`;
`profile.deleteCamera` (or `profile.deleteImported`) removes one. No profile ships with LightCraft, and the repository
holds none: tests build theirs in memory (`lightcraft_raw::dcp::write_dcp`).
