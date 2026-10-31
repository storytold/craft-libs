# Local Krilla 0.8.2 fork

This is the crates.io Krilla 0.8.2 package (MIT OR Apache-2.0), upstream commit
`3ffdf0588cf98050aad6edba51ca70162e1fb5b5`, with the patches listed below. Registry
bookkeeping files, `Cargo.lock` and the redundant original manifest are omitted. Upstream
MIT/Apache license texts are retained under `../../licenses/pdf-writer`; the registry package
itself omits those texts. Its examples and four CC0 ICC profiles are retained without changes.

## Patch ledger

| File | Patch | Touches |
|---|---|---|
| `patches/01-unwind-safe-drop.patch` | Unwind-safe drop | `document.rs`, `error.rs`, `lib.rs`, `page.rs`, `serialize.rs`, `surface.rs` |
| `patches/02-group-color-space.patch` | Blending color space | `error.rs`, `graphics/{graphics_state,image,mask,tiling_pattern,xobject}.rs`, `lib.rs`, `page.rs`, `pdf.rs`, `resource.rs`, `serialize.rs`, `text/type3.rs` |
| `patches/03-overprint.patch` | Overprint | `configure/{PDF_A2.md,PDF_A4.md,validate.rs}`, `content.rs`, `graphics/{blend,graphics_state}.rs`, `surface.rs` |
| `patches/04-knockout.patch` | Knockout groups | `content.rs`, `graphics/{blend,xobject}.rs`, `surface.rs` |
| `patches/05-soft-mask-backdrop.patch` | Soft-mask backdrop and transfer function | `configure/{PDF_A2,PDF_A4}.md`, `graphics/mask.rs` |
| `patches/06-device-n.patch` | DeviceN colors | `configure/{PDF_A1.md,PDF_A2.md,PDF_A4.md,validate.rs}`, `graphics/{color,device_n,mod,paint}.rs`, `serialize.rs` |
| `patches/07-device-n-images-shadings.patch` | DeviceN images and shadings | `graphics/{image,paint,shading_function}.rs` |
| `patches/08-print-output-intent.patch` | Print output intent and PDF/X markers | `chunk_container.rs`, `configure/{PDF_A1.md,PDF_A2.md,PDF_A4.md,validate.rs}`, `interchange/metadata.rs`, `lib.rs`, `serialize.rs` |
| `patches/09-rendition-class.patch` | Master rendition in XMP | `chunk_container.rs` |
| `patches/10-mesh-gradients.patch` | Mesh gradients | `content.rs`, `error.rs`, `graphics/{mask,paint,shading_function}.rs`, `serialize.rs`, `surface.rs` |
| `patches/11-png-decode-cap.patch` | PNG decode cap | `graphics/image.rs` |
| `patches/12-optional-content.patch` | Optional content | `chunk_container.rs`, `configure/{PDF_A1,PDF_A2,PDF_A4,PDF_UA1}.md`, `configure/validate.rs`, `content.rs`, `error.rs`, `graphics/{mod,optional_content}.rs`, `lib.rs`, `resource.rs`, `serialize.rs`, `surface.rs` |
| `patches/13-annotation-bounds.patch` | Annotation and destination bounds | `error.rs`, `interactive/{annotation,destination}.rs`, `page.rs`, `serialize.rs` |
| `patches/14-devicen-pdf-version.patch` | DeviceN attributes by PDF version | `graphics/device_n.rs` |
| `patches/15-pdfx-conformance.patch` | PDF/X conformance key | `serialize.rs` |
| `patches/16-modification-date.patch` | Modification date | `interchange/metadata.rs` |

Each patch applies with `patch -p1` from this directory, in numeric order, to the pristine
crate. `SOURCE.json` records, for every retained file, the upstream hash (`upstream_sha256`)
and the hash after all patches (`sha256`), plus the hash of the `.crate` download
(`crate_sha256`).

Every change to `src/` updates its patch file, this ledger and `SOURCE.json` in the same
commit. To take a new Krilla release: unpack the crates.io package, apply the patches in
order, drop the ones the release makes unnecessary, regenerate `SOURCE.json`, and rerun the
craft-pdf and plate-test suites.

## 01: unwind-safe drop

The patch changes two destructors: `Surface::drop` and `Page::drop` return early
when `std::thread::panicking()` is true. Drawing can panic with an unfinished
transform, clip, isolated group or tagged section. Finalizing that state during
unwind can panic a second time and abort before an outer catch can recover.
Discarding the unfinished state permits native unwind recovery. Cleanup marks the
serialization context poisoned until it completes successfully, preserving any prior
poison. Document::finish returns KrillaError::Poisoned after failed cleanup, including
explicit surface finalization, even if a caller retains the writer after a narrow
catch. A caller must still discard other captured app state after a panic. Normal
finalization, including checks for unbalanced state, remains unchanged.

Public marker constants, `CRAFT_UNWIND_SAFE_DROP` and `CRAFT_UNWIND_POISONING`, let craft-pdf reject an accidentally
unpatched dependency at compile time. Consumers must copy the Krilla root patch
from the craft-pdf README because Cargo ignores dependency workspaces' patches.

The shared writer's native subprocess regression covers the previously aborting
cleanup and a successful fresh export afterward. Native and WASM export tests
cover the unchanged successful path. WASM aborts, aborting panic configurations
and process-wide allocation failures remain outside unwind recovery.

## 02: blending color space

Upstream gives every transparency group an RGB `/CS` and writes no page group, so
transparency over CMYK content blends in RGB and comes back with all four process inks
(K50 over C100 separates as roughly C84 M34 Y33 K37 instead of C50 K50).

`SerializeSettings::group_color_space` chooses the blending space. `GroupColorSpace::Rgb`,
the default, writes the same groups and page dictionaries as upstream. With
`GroupColorSpace::Cmyk`:

- Content groups (`draw_masked`, `draw_opacified`, `draw_isolated`, `Graphic`) write no `/CS`
  and inherit the page group.
- A page that uses transparency gets `/Group <</S/Transparency/CS …>>`: DeviceCMYK, or the
  `cmyk_profile` ICC space when `no_device_cs` is set. Opaque pages get no page group, so
  they stay valid for PDF/X-1a and X-3.
- Soft-mask source groups have no parent, so they name their own space: DeviceCMYK, or
  sGrey when `no_device_cs` is set. Through a CMYK profile K 100% is not black, so an ICC
  CMYK mask group would give the default black backdrop a luminosity above 0, and the masked
  object would show where the mask content paints nothing. The XObject `Repr` carries a
  `GroupRole` (content or mask source) for this.
- Pages placed as XObjects (`draw_pdf_page`) get an isolated group without `/CS`; the RGB
  color space is not registered. The source page's own group is dropped, as before;
  krilla #410 (unreleased, needs hayro-write 0.8) keeps it.
- `no_device_cs` without a `cmyk_profile` fails at `Document::finish` with
  `KrillaError::MissingGroupCmykProfile`.

Which pages blend is tracked by object reference, because cached objects are serialized
once: transparent graphics states (alpha below 1, a blend mode other than Normal, a soft
mask), group XObjects, images with alpha and embedded pages are recorded when written,
and so are forms, tiling patterns and Type 3 fonts whose resources include one of these.
A page blends when its resources name a recorded object.

Tests: `crates/pdf/tests/print_color.rs`, and the plate-test scenes S06, S07, S08, S10,
S15 and D04.

## 03: overprint

Upstream writes no overprint entries, so separated output knocks out every plate under an
object; a [Black] rect over C100 prints K100 instead of C100 K100.

`Surface::push_overprint(Overprint { fill, stroke, nonzero })` pushes overprint settings
like `push_blend_mode`: no group, popped with `Surface::pop`. Paths, text and images drawn
in between get an ExtGState with `/op` (fill), `/OP` (stroke) and `/OPM` (1 when `nonzero`,
otherwise 0). All three entries are always written, so an inner push replaces an outer one
completely and a RIP never inherits OPM 1 from an enclosing state. Groups pushed inside
inherit the settings through the `Do` that paints them. Overprint is not a field of `Fill`
or `Stroke`, because apps build those as struct literals.

The ExtGState `Repr` carries `overprint_fill`, `overprint_stroke` and `overprint_mode`;
`empty`, `combine` and `serialize` handle them like the other entries. Overprint is
not transparency: it marks no page as blending.

Validation: ISO 19005-2 and 19005-4 (6.2.4.2) forbid OPM 1 when an ICCBased CMYK color
space is used for a side that overprints. Validated PDF/A export writes all CMYK as
ICCBased, and the graphics state doesn't know the colors painted under it, so OPM 1 with
`/op` or `/OP` true under `no_device_cs` registers `ValidationError::OverprintModeIccCmyk`,
which PDF/A-2, A-3 and A-4 prohibit. PDF/A-1 and PDF/UA allow it. ISO 32000-1 (8.6.7)
applies OPM 1 to DeviceCMYK only, so under `no_device_cs` it has no effect in a PDF 1.7
reader; `Overprint::nonzero` says so.

Tests: `crates/pdf/tests/print_color.rs`, and the plate-test scenes S01, S03, S04, S12,
S16, D06 and D10.

## 04: knockout groups

Upstream can only push isolated groups, so objects overlapping inside a group always
composite with each other; a knockout group of a 50% cyan and a 50% magenta circle over
yellow separates as C25 M50 Y25 where they overlap instead of M50 Y50.

`Surface::push_group(GroupOptions { isolated, knockout })` pushes a group with `/I` and
`/K` set independently; `push_isolated` is shorthand for `isolated` alone. A knockout
group is a transparency group even when it is not isolated, so it gets patch 02's `/CS`
rules: krilla's RGB `/CS` under `GroupColorSpace::Rgb`, none under `Cmyk`, and it marks
its page as blending. A group with neither attribute is a plain form XObject, as before.

The XObject `Repr` carries `knockout`; `XObject::group` builds a content group from
`GroupOptions`.

Tests: `crates/pdf/tests/print_color.rs`, and the plate-test scene S09.
## 05: soft-mask backdrop and transfer function

Upstream writes soft masks with `/S` and `/G` only. A luminosity mask then has the default
black backdrop, so the masked object is hidden wherever the mask content paints nothing, and
there is no transfer curve.

- `Mask::with_backdrop(color)` writes `/BC` on luminosity masks, in the mask group's color
  space: RGB under `GroupColorSpace::Rgb` (DeviceRGB or sRGB); under `GroupColorSpace::Cmyk`,
  four components (DeviceCMYK), or one (sGrey) with `no_device_cs`. Colors from another space
  are converted with the device formulas; luma becomes equal RGB components or black ink only.
  On an alpha mask, where `/BC` has no meaning, the call returns the mask unchanged.
- `Mask::with_transfer(TransferFunction)` writes `/TR`: `/Identity`, or a Type 0 sampled
  function (domain and range 0 to 1, 16-bit samples, linear interpolation) for
  `TransferFunction::Sampled`. An empty curve is written as `/Identity`; one sample is
  repeated to span the domain.

Standards: PDF/A-2 and PDF/A-4 6.2.5 forbid `TR` in graphics states (veraPDF rules test
`PDExtGState` only); nothing restricts the soft-mask dictionary's `/TR`, so no validation
error is added and the PDF/A notes say so. PDF/X (ISO 15930-7) forbids graphics-state transfer
functions; whether it or a PDF/X-4 preflight also flags a soft-mask `/TR` could not be checked
against the standard's text.

Tests: `crates/pdf/tests/print_color.rs`, and the plate-test scenes S11, M1, M2 and D08.
## 06: DeviceN colors

Upstream has only Separation as a special color space, so a color that sets several
inks at once (C60 K40 without touching M and Y, or a process ink with a spot) has to be
written as CMYK, which knocks out or replaces the other plates.

`color::device_n` adds `SpecialColor::DeviceN`:

- `DeviceNSpace::new(colorants)` takes an ordered list of `Colorant::Process(Process)` and
  `Colorant::Spot { name, alternate: cmyk::Color }`. It returns `None` for an empty list,
  more than 32 colorants, a repeated name, or a spot named `All`, `None` or like a process
  ink. `device_n::Color::new(tints, space)` takes one tint per colorant.
- The space is written as `[/DeviceN [names] alternate tint attributes]` and cached like a
  Separation space. The alternate is DeviceCMYK, or the `cmyk_profile` ICC space with
  `no_device_cs`.
- The tint transform is a Type 4 (PostScript calculator) function. Each colorant
  contributes its tint times its CMYK alternate (process inks: 1 on their own component);
  contributions combine as `1 - (1 - a)(1 - b)…`, so process tints pass through unchanged.
  It registers `ValidationError::ContainsPostScript`, which krilla's PDF/A tables forbid for
  PDF/A-1, -2 and -3.
- From PDF 1.6, a space with spots gets an attributes dictionary whose `/Colorants` maps
  each spot to its Separation space. When spots and process inks mix, it is
  `/Subtype /NChannel` with `/Process << /ColorSpace alternate /Components [/Cyan /Magenta
  /Yellow /Black] >>`. Patch 14 changes which of these each PDF version gets.
- Validation: a spot's CMYK alternate counts as its Separation fallback, so one colorant
  name keeps one alternate across Separation and DeviceN spaces
  (`ValidationError::InconsistentSeparationFallback`). `/All` and `/None` are Separation-only
  and checked as before. The PDF/A ledgers state that DeviceN fails export under PDF/A-1, -2
  and -3 and is written under PDF/A-4.
- `Color::to_regular` (annotations, tags) gives the alternate CMYK; `Paint::as_rgb` gives
  `None`, as for CMYK.

Images and shadings use the same `DeviceNSpace` (patch 07).

Tests: `crates/pdf/tests/print_color.rs` (`device_n_*`), and the plate-test scene N1.

## 07: DeviceN images and shadings

Upstream images are RGB, gray or CMYK, so a duotone or a spot-channel image can't name its
inks. Gradient stops must share one color space: a stop that mixes a spot with process
inks, or two spots, has no space to go in, and stops from different spaces fail an
assertion while writing.

- `ImageColorspace::DeviceN` for `CustomImage`, with the space from the new
  `CustomImage::device_n_space` (default `None`): one sample per colorant, in the space's
  colorant order, 0 = no ink. `Image::from_custom` refuses a DeviceN image without a space.
  The image's `/ColorSpace` is the cached DeviceN space from patch 06; an ICC profile is
  ignored. `ImageColorspace` stays `Copy`, so the new variant breaks only exhaustive
  matches.
- Gradient stops (linear, radial, sweep) in different ink-based spaces are rewritten into
  one space when the shading function is created. Ink-based: CMYK, gray (as black), a
  Separation with a named colorant, DeviceN. A process colorant takes part when a stop has
  it at a nonzero tint or a DeviceN stop names it; a spot takes part when a stop names it.
  - One spot and no process colorant: a Separation shading in the spot's own space. A spot
    to paper (CMYK 0 0 0 0, white, or tint 0) stays a Separation shading.
  - Process colorants only (gray to CMYK, for example): DeviceCMYK.
  - Anything else: a DeviceN shading, process colorants in CMYK order, then spots in the
    order the stops name them; the function has one output per colorant. A spot whose
    fallback isn't CMYK can't join a DeviceN space.
  - Stops that already share a space, and mixes with RGB, `All` or `None`, are written as
    upstream does.
- PostScript (sweep) gradient functions get one `/Range` pair per output component;
  upstream always wrote three.

The conversion lives in `shading_function::common_color_space`, which takes a list of
colors, so other shading types (such as mesh gradients) can call it for their colors.

Tests: `crates/pdf/tests/print_color.rs` (`device_n_image_*`, `device_n_shading_*`), and
the plate-test scene S14.
## 08: print output intent and PDF/X markers

Upstream writes an output intent only for PDF/A, always sRGB, and has no `/Trapped` or PDF/X
version. DesignCraft and VectorCraft added these by rewriting the serialized bytes.

- `SerializeSettings::output_intent: Option<OutputIntent>` writes an `/OutputIntents` entry
  with `/S /GTS_PDFX`: the CMYK profile as `/DestOutputProfile`, `/OutputConditionIdentifier`,
  and optional `/OutputCondition`, `/RegistryName` and `/Info`. With a PDF/A validator, the
  `GTS_PDFA1` entry names the same profile object and condition instead of sRGB, because
  PDF/A requires every output intent to share one destination profile.
- `SerializeSettings::trapped: Option<Trapped>` writes `/Trapped` to the document
  information dictionary and `pdf:Trapped` to XMP, `Unknown` included (the Adobe PDF schema
  allows it, and PDF/A-1 requires the two to match).
- `SerializeSettings::pdfx_version: Option<String>` writes `/GTS_PDFXVersion` to the
  dictionary and `pdfxid:GTS_PDFXVersion` to XMP. PDF/A-1, -2 and -3 require an extension
  schema for the XMP property, which this patch does not write, so the setting registers
  `ValidationError::PdfxVersionWithoutExtensionSchema`, which they prohibit. PDF/A-4 has no
  extension schema requirement and allows it.
- The dictionary entries follow the dictionary's PDF 2.0 deprecation: from PDF 2.0 only XMP
  carries them. The dictionary is written without `Metadata` when it has print markers.
- All three default to `None`, which keeps upstream's output.
- The PDF/A ledgers state the two output intents and the CMYK destination profile; krilla
  checks only that the profile has four components.

krilla does not check PDF/X conformance; callers do. `CRAFT_PRINT_COLOR` marks the fork's
print color patches (02 to 08) so craft-pdf fails to build against an unpatched Krilla.

Tests: `crates/pdf/tests/print_color.rs` and the plate-test print preset over every scene.

## 09: master rendition in XMP

From storytold/craft-libs#183 by Hamza (@Hamza0101). Upstream writes
`xmpMM:RenditionClass` as `proof` on every document, which tells a prepress system that a
print deliverable is a proof copy. XMP Part 2 (Media Management) defines `default` as the
master rendition, so the patch writes `default`. Every document's XMP differs from
upstream's output in this value, whatever the other settings.

## 10: mesh gradients

From storytold/craft-libs#184 by @fastmikey99.

Krilla 0.8.2 paints only flat color, linear, radial and sweep gradients and tiling patterns, so
an app writing a gradient mesh had to cut each patch into flat-color pieces: a VectorCraft
document holding an Illustrator file's 22,850 mesh patches became a 197 MB PDF with stepped
blends. The patch adds `paint::MeshGradient` (and `MeshPatch`): Coons patches with a color and
an opacity at each corner, written as one PDF Type 6 shading (ISO 32000-1, 8.7.4.5.7) through
pdf-writer's `StreamShading`, so the viewer or RIP blends them at its own resolution. Each patch
is an 8-bit flag (no shared edge), its twelve points as 32-bit coordinates over the mesh's
bounds (`Decode`) and its four corner colors at 8 bits a component, Flate-compressed. It goes
through the same path as the other gradients (`GradientProperties::MeshGradient`): a shading
pattern for the fill, and for corner opacities below 1 a luminosity soft mask holding the same
mesh in gray (`use_opacities`), as cairo and Illustrator write transparent meshes.

The corner colors of every patch go through `common_color_space` from patch 07, the rule
gradient stops follow, before the shading is written: process colors alone become
DeviceCMYK, one spot alone a Separation space, and any other mix of inks a DeviceN space with
the process colorants in CMYK order, then the spots. A mesh mixing CMYK and a spot is a
DeviceN mesh with one component per colorant at each corner. Colors the rule can't bring into
one space (RGB with CMYK, `All` or `None` with anything else) would be written with another
space's component count, so the document fails with `KrillaError::MixedMeshColorSpaces`
instead. The soft mask is written in gray from the opacities and is unaffected.

Tests: `crates/pdf/tests/print_color.rs` (`mesh_*`, `rgb_mesh_*`) and the plate-test scene S17.

## 11: PNG decode cap

A PNG whose header implies more than 1 GiB of decoded output is refused by
`Image::from_png` and by the deferred decoder, and the decoded buffer is reserved
fallibly, so a small file (or a bitmap glyph in a font) cannot request an allocation that
ends the process. The color and alpha buffers that PNG, GIF and WebP decoding split an
image into are reserved fallibly too, and a failure there is an image error.
`Image::from_custom` computes the expected channel lengths with checked `usize` arithmetic and
returns an error when they overflow or don't match the data, where upstream multiplied in
`u32` and asserted while writing (32 DeviceN components at 16 bits overflow at about 67
megapixels).

## 12: optional content

Krilla 0.8.2 can't write optional content groups (layers), and it has no hook for raw content
operators or extra catalog and resource entries, so CADCraft's drawing layers can't be added
from outside. The new module `graphics/optional_content.rs` holds the feature; the other files
get small hooks.

- `Document::add_optional_content_group(name, visible) -> OcgId` writes
  `<< /Type /OCG /Name (…) >>` once (a text string, so any Unicode name works).
- `Surface::start_optional_content(OcgId)` and `Surface::end_optional_content()`, both
  returning `KrillaResult<()>`, write `/OC /ocN BDC` … `EMC`, with `/ocN` in the current
  stream's `/Properties` resources (a new resource kind beside `/ExtGState`). Inside a group,
  opacity or mask, the entry goes in that form's resources.
- Marks nest, and nest independently of tagging, whose single marked-content level is
  unchanged. An end with nothing open, or one that would close a tagged section started inside
  the mark, returns `KrillaError::UnbalancedOptionalContent` and writes nothing. An `OcgId` index
  the document doesn't have returns `KrillaError::UnknownOptionalContentGroup`. Marks still open
  when their stream finishes (surface finish, or a pop of a group, opacity or mask), and a
  tagged section ended while a mark started inside it is open, are closed with `EMC` and make
  `Document::finish` fail with `KrillaError::UnbalancedOptionalContent`. Nothing panics.
- When any group exists, the catalog gets `/OCProperties << /OCGs [all] /D << /Name (Layers)
  /Order [all] /OFF [hidden] >> >>`, listing groups that no page uses too; `/OFF` is omitted
  when every group is shown. `Document::set_optional_content_page_mode(true)` adds
  `/PageMode /UseOC`, written only when groups exist.
- Validation: adding a group registers `ValidationError::OptionalContent`, which PDF/A-1
  prohibits (6.1.13), and below PDF 1.5 also
  `ValidationError::RequiresNewerPdfVersion(VersionedFeature::OptionalContent)`, which PDF/UA-1
  prohibits. Without a validator a PDF 1.4 document still gets the groups, as with krilla's
  other versioned features. The PDF/A-2, A-4 and UA-1 ledgers record that the one
  configuration has a `Name`, an `Order` listing every group and no `AS`.

`CRAFT_OPTIONAL_CONTENT` marks the patch; craft-pdf requires it at compile time.

Tests: `crates/pdf/tests/optional_content.rs`.

## 13: annotation and destination bounds

Upstream panics on link geometry that has no finite place on the page, which aborts a WASM
app. `Document::finish` returns an error instead and writes nothing for the object:

| Input | Upstream | Error |
|---|---|---|
| `LinkAnnotation::new_with_quad_points` with no quads, a non-finite point, or a bounding box whose width or height overflows | `assert!` or `unwrap` | `InvalidAnnotationGeometry(location)` |
| A link rect or quad bounding box that overflows when flipped into PDF coordinates (`page_height - y`) | `unwrap` | `InvalidAnnotationGeometry(location)` |
| A media, crop, bleed, trim or art box that overflows the same way | `unwrap` | `InvalidPageBox(page_index)` |
| An `XyzDestination` page index past the last page | `panic!` | `InvalidDestination(page_index)` |
| An `XyzDestination` point that maps to a non-finite coordinate | writes `inf` or `NaN` | `InvalidDestination(page_index)` |

The constructors keep their signatures: `LinkAnnotation` holds an `Option<Rect>` and the check
runs at serialization, before the annotation object starts. Refusing a single annotation
instead of skipping it keeps the indices of tagged annotations in the structure tree valid.

Tests: `crates/pdf/tests/link_annotations.rs`.

## 14: DeviceN attributes by PDF version

Patch 06 writes the DeviceN attributes dictionary only from PDF 1.6. A PDF/X-1a file is PDF
1.3 or 1.4, so its spots then had no `/Colorants`, and a file written at 1.6 or later and
relabeled 1.4 kept `/Subtype /NChannel` and `/Process`, which preflight tools flag in a 1.4
file.

ISO 32000-1, 8.6.6.5 (Table 71) marks the attributes dictionary and its `/Colorants` entry
PDF 1.3, and `/Subtype`, `/Process` and `/MixingHints` PDF 1.6; the PDF Reference for 1.3 and
1.4 lists `/Colorants` as the dictionary's only entry. The spec was checked from memory, not
against the text.

- Every version krilla writes (1.4 and later): a space with spots gets an attributes
  dictionary with `/Colorants`, each spot mapped to its Separation space.
- From PDF 1.6, a space that mixes spots and process inks also gets `/Subtype /NChannel` and
  `/Process`, as patch 06 writes them.
- A space of process inks only gets no attributes, as before.

Tests: `crates/pdf/tests/print_color.rs` (`device_n_attributes_follow_the_pdf_version`) and
`crates/prepress/tests/flatten_document.rs` (`pdf_x1a_output_is_pdf_14_without_nchannel`).

## 15: PDF/X conformance key

Patch 08 writes only `GTS_PDFXVersion`. PDF/X-1a and X-3 also need `GTS_PDFXConformance` in
the document information dictionary: a PDF/X-1a:2003 file carries `GTS_PDFXVersion
(PDF/X-1:2003)` and `GTS_PDFXConformance (PDF/X-1a:2003)` (ISO 15930-4). Later parts (X-4
and on) have no conformance key.

- `SerializeSettings::pdfx_conformance: Option<String>` writes `/GTS_PDFXConformance` to the
  dictionary, under the same PDF 2.0 rule as patch 08's entries. It defaults to `None`.
- There is no XMP property for it: the `pdfxid` schema (ISO 15930-7) defines only
  `GTS_PDFXVersion`. A PDF 2.0 file therefore omits the key.

Tests: `crates/pdf/tests/print_color.rs` (`trapped_and_pdfx_version_are_written_to_info_and_xmp`)
and `crates/prepress/tests/flatten_document.rs` (`pdf_x1a_output_is_pdf_14_without_nchannel`).

## 16: modification date

Upstream has only a creation date and writes it as the modification date too (`/ModDate` and
`xmp:ModifyDate`), so a document rewritten from another, such as a flattened copy, can't keep
its source's modification date.

- `Metadata::modification_date(DateTime)` sets the date written as `/ModDate` and
  `xmp:ModifyDate`. Without it, the creation date is written there, as upstream does.
- The XMP history events keep the creation date.

Tests: `crates/pdf/tests/writing.rs` (`writes_pages_metadata_and_selected_version_without_a_renderer`,
`the_creation_date_is_the_modification_date_when_none_is_given`) and
`crates/prepress/tests/flatten_document.rs`
(`a_document_flattens_page_by_page_and_keeps_boxes_metadata_and_shared_fonts`).
