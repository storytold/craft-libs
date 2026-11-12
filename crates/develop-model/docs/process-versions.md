# Process versions

> **Changes:** [revision history](#revision-history)

Every photo's develop settings record the **rendering process** they are interpreted by
(`DevelopSettings::process`, a `ProcessVersion` in `crates/develop/src/settings.rs`). Lightroom calls
this the process version. It lets the look of LightCraft improve (a new default tone curve, issue #146;
stronger Light sliders, issue #196) without changing a single photo someone has already edited.

| Process | What it changes |
|---|---|
| **V1** | LightCraft's rendering from before process versions existed. Frozen. |
| **V2** | Highlights and Shadows keep fine detail (issue #632): their base layer (`local::tone_base`) is the edge-aware filter of a separable median of log luminance (radius 0.15 % of the long edge, at most 24 px, `local::BASE_DETAIL`) instead of log luminance itself, so twigs, lines and texture take their surroundings' shift and keep their contrast; region edges stay sharp. CPU (`local::prepare`) and GPU prepare branch on `local::tone_base_detail` (0 = no median under V1); the base plane is cached per process (`local::BaseKey`). Renders without Highlights or Shadows (global or local) are the same as V1's. Tone map unchanged. |
| **V3** | Pixel-scale sharpening (issue #757): Radius uses the original photo’s pixels, Detail controls halo damping, and downsized sources retain a response-matched sharpening band. V1 and V2 keep texture-band sharpening exactly. The sharpen plane is keyed by radius and process and invalidated after noise-reduction changes. The Highlights/Shadows base and tone map are unchanged from V2. |
| **V4** | V3 plus calibrated global Contrast (per channel) and Blacks (per-channel/luminance mix) on raws with a camera tone curve. Whites behaves exactly as V2, both signs; its calibration is a planned follow-up. Other sources and local adjustments retain V2 operators. See [calibrated Light operators](light-v3.md). |
| **V5** | V4 plus: a raw's tone curve rolls off into white in stops instead of reaching it within about a stop of the sensor's white, so raising Exposure keeps the highlights the sensor recorded (issue #1632). **Camera curve** (the look fitted to the embedded JPEG, or a DNG's `ProfileToneCurve`; `CameraTone`): past its last knot `CameraTone::apply_rolloff` closes the gap to white by `gap / (1 + c·(2^t − 1))` at `t` stops past the knot, with `c` matching the curve's slope there and the gap halving every stop in the end (`tone::ROLLOFF_STOPS`); `ToneMap::camera_rolloff_hdr` builds the table from it. Under V1 to V4 the curve past its last knot (fitted only to camera JPEG tones below 0.85, so 1 to 3 stops below the sensor's white on most photos) closes on white exponentially in linear light: the sensor's white renders at 8-bit 246 to 254 at Exposure 0 and 254 to 255 from Exposure +1. On the 11 CC0 corpus files with a fitted look (8 Sony, 2 Nikon, 1 Fujifilm) it now renders at 239 to 250 at Exposure 0 and 247 to 253 at +1. A DNG curve that reaches white at the sensor's white renders as before. **Generic curve** (raws without a camera curve, `ToneMap::rolloff_hdr`): from half a stop below its white point (`tone::GENERIC_KNEE_STOPS`) the same shoulder takes over, with the extended Reinhard curve's value and slope at the join, where V1 to V4 clip at the white point (scene 1.18 at default settings, ¼ stop above the sensor's white). The sensor's white renders at about 235, 248 and 252 at Exposure 0, +1 and +2 (V1 to V4: 239, 255, 255; the CC0 Canon R8 file and 6 CC0 DNGs without a camera curve). Content past the old clip point, such as a clipped sky, now renders up to 15 codes below white at Exposure 0 and reaches 255 about 5 stops past the sensor's white. Below the last knot or the join, JPEGs and other display-referred sources, and the camera fit itself are unchanged; CPU and GPU share the tone LUT. |
| **V6** | V5 plus: colour noise reduction filters the linear colour differences R−Y and B−Y with the joint method (issue #815, `docs/nr-color-joint.md`): edge-aware weights from a denoised guide shared by both differences, anisotropic diffusion over scales, a brake for isolated colour specks; Y is kept exactly. V1 to V5 keep the blurred chromaticity (`local::denoise`). CPU (`local::denoise`) and GPU (`render::denoise`) branch on `local::joint_colour_nr`. Renders with Color 0 are the same as V2's. |
| **V7** | V6 plus: film grain from crystal layers ([grain.md](grain.md), `crates/pipeline/src/crystal.rs`), after Aurélien Pierre's model: resolution independent (a preview is the export's grain box-downscaled), fading out at black and white, calibrated to Lightroom Classic's strength and width at mid grey. V1 to V6 keep their two-octave value noise exactly (`finish::ValueGrain`; on the GPU `F_GRAIN_MODEL` 0); `finish::GrainModel::new` picks the model per process. Renders without grain don't depend on this. |
| **V8** (the latest) | V7 plus: Highlights and Shadows take their edge-aware filter of log luminance without its fine detail (`local::detail_free`: each value clamped between its separable median at the V2 radius and the median of a 3× wider neighbourhood) instead of V2's plain median (`local::base_keeps_gaps`), so the gaps in dark lettering on a bright panel are not lifted into bright blocks (issue #3460). CPU (`local::tone_base`) and GPU (`render::tone_base`) branch on it; the base plane is keyed by process. Renders without Highlights or Shadows (global or local) are the same as V7's. |

## The rule

**A change that alters how existing settings render ships as a new process. V1 is frozen.**

- A deliberate change of look (tone model, slider response, default curve, how a stage interprets a
  value) goes into a new process. Photos stay on the process they have until someone runs Update to
  Current Process (or Reset).
- Bug fixes that make output correct where it was broken (a crash, NaN or black output, a decoder
  error, CPU and GPU disagreeing) are not looks and can land in place, as before.
- Changes to how a file is decoded or which camera colour it gets change the source, not how settings
  are interpreted; the process doesn't reach that stage today.

**Neutral display tone map** (JPEG, PNG and other display-referred sources with Contrast, Whites and
Blacks at 0) is the identity up to white: the SDR shoulder no longer pulls 1.0 down to 0.9875 (8-bit
255 to 254, issue #1945). It landed in place as a bug fix, since the tone map promised identity at
neutral settings; edited curves keep the shoulder and their V1 look.

**AI mask edges** (Object and Describe selections stored as a model's grid) follow the photo's edges
when they are enlarged to the output (issue #3272): the plain enlargement spilled 1–2 px onto the
background, which a raised Exposure turned into a bright outline around the subject in previews and
exports. Existing AI masks become 1–2 px tighter at a subject's edges, on every process; their
inside, their outside and edges without contrast in the photo are unchanged. It landed in place as
a bug fix: the outline is output that isn't in the photo nor in the edit, not a look anyone chose.

**A mask whose first component is Intersect** starts from that component, on the CPU and the GPU
(issue #1889). It used to start from nothing, so alone it selected nothing, with a later Add it gave
only what was added, and with the mask's Invert it gave the whole photo. Such masks now include the
first component, on every process. It landed in place as a bug fix: intersecting with nothing is not
a look anyone chose.

**Lettering under Highlights and Shadows** (V8) no longer grows blocks of the wrong tone. V2's base
is the edge-aware filter of a separable median alone. Dark lettering on a bright panel (a bus
number, a sign) has strokes and gaps about as wide as the median's radius, so each row and column
window was close to half letter and half panel, and the median picked one tone or the other in
rectangles: it filled the gaps between the letters and their counters with the letters' tone.
Shadows then lifted those patches of panel by over 1 EV, into bright blocks beside the letters. From V8 each
value moves toward that median only as far as the median of a 3× wider neighbourhood agrees
(`local::detail_free`). A twig against a wall has both medians on the wall's side and takes its tone,
as before (#632); the gap between two strokes sits in a wider neighbourhood that is mostly panel, the
two medians disagree, and the gap keeps its own tone. The letters themselves count as fine dark detail
on the panel and keep their contrast, as twigs do. V8 differs from V7 under Highlights or Shadows only in detail
about as fine as the radius, mostly where it is packed closely (lettering, grilles). V2 to V7
are released and keep the blocks (`tests_legacy` locks V1 and V2); Update to Current Process moves a photo past them.

## Adding a process

1. Add a variant to `Process` (`V2`), append it to `Process::ALL` (oldest first), give it its number in
   `Process::version` and make it `Process::LATEST`.
2. The compiler then points at every `match` on `Process`. The base tone map is chosen in one place,
   `lightcraft_pipeline::finish::base_tone` (both the CPU and the GPU renderer take it from there): give
   V2 its own arm and leave V1's alone. A process that changes another stage branches there the same
   way, on `s.process.process()`; if that stage is cached (`Plan::lin_key`, the planes in `local.rs`),
   add the process to its key.
3. Keep `settings_saved_before_process_versions_load_as_v1` (`crates/develop/src/tests_process.rs`)
   passing: settings saved without the field must stay V1, not become the new latest. That is the
   field's `#[serde(default = "ProcessVersion::legacy")]`; the struct-level `Default` gives new
   settings the latest instead.
4. Render a few procedural scenes on both processes and compare, and update `docs/parity-checklist.md`.

## What gets which process

| | Process |
|---|---|
| A newly imported, merged or Lightroom-imported photo | the latest |
| Settings saved without the field (catalogs and their snapshots, versions and history, `lc:settings` in XMP sidecars, browser libraries: everything from before process versions), or with a damaged value | V1 |
| Reset (`develop.reset`) | the latest |
| Reset of a section or a slider | unchanged |
| Update to Current Process (`develop.updateProcess`) | the latest, for photos on an older one; one undo step. The menu item follows the selection; a call with `ids` / `id` acts on those photos whatever is selected |
| Virtual copy, duplicate | the source's |
| Copy / paste / sync / Auto Sync / presets | unchanged (see below) |
| `crs:` edits (XMP from other raw developers, Lightroom catalogs, imported presets) | unchanged; a new photo has the latest |
| Restoring a version or history step | the one it had |

**Copy, paste, sync and presets** never carry the process: it is in no settings group, so the photos
they change keep their own and interpret the pasted values with it. (Lightroom offers the process
version as a separate checkbox when syncing; LightCraft's groups are coarser and keep it out.) Partial
settings JSON that names `process` itself (`develop.merge`, a hand-written preset) sets it.

**Lightroom edits** are re-interpreted by LightCraft's mapper (`crates/engine/src/crs.rs`), which is
written against the current rendering. `crs:ProcessVersion` numbers Adobe's renderer, not ours, and is
ignored; a photo imported with Lightroom edits is on LightCraft's latest process, and an existing
LightCraft photo that receives them keeps its own.

**A number above this build's latest** (a library or sidecar written by a newer LightCraft) loads, is
kept when saved, and renders with the newest process this build has. Update to Current Process never
moves a photo to an older process; Reset does move it to this build's latest.

**A damaged value** (anything but a whole number from 0 to 2^32 - 1; `2.0` counts as 2) reads as V1,
like a missing field, and costs nothing else: the rest of the settings load from the catalog, a
snapshot, the op log or an `lc:settings` sidecar as usual. The damaged value isn't kept: the photo is
saved as V1.

## Storage

The process is saved as `"process": <number>` in the settings JSON, except V1, which is left out.
Settings made before process versions existed are therefore saved exactly as before, and their
preview-cache keys (`DevelopSettings::hash64`) don't change. `develop.get` (MCP `get_develop`) always
states the process.

From process V3 on, persistent thumbnail, view-render and variant keys also include the
resolved rendering process. A future number can keep the same settings hash across builds
while resolving to different operators, so those keys must differ. V1 and V2 omit this
extra field and retain their existing cached renders. No cache-version bump is needed.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-08 | minor | V8: the Highlights/Shadows base keeps the gaps in dark lettering (#3460) |
| 2026-11-07 | minor | V7: film grain from crystal layers ([grain.md](grain.md)); V1 to V6 keep their value noise |
| 2026-11-05 | minor | V5: a raw's tone curve rolls off into white in stops, so raising Exposure keeps recorded highlights (#1632) |
| 2026-10-12 | minor | Calibrated Contrast and Blacks, V2 Whites, resolved-process cache keys and number coordination |
