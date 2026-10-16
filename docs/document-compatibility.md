# Creative Application File Format, Compatibility & Conversion Reference

> **Author:** Zach Shallbetter (`Zach Shallbetter <zachshallbetter@icloud.com>`)  
> **Status:** Authoritative Ecosystem Specification & Compatibility Reference  
> **Scope:** Raster Imaging, Vector Geometry, Multipage Publications, and Temporal/Animation Media  
> **Ecosystem Alignment:** PhotoCraft, VectorCraft, DesignCraft, PdfCraft, FilmCraft, EffectCraft, and `mediacraft`/`craft-libs`

---

## Executive Summary & Suite Mapping

This specification establishes the authoritative file format, compatibility, conversion, and structural preservation reference across the four core creative document paradigms. Within the Storytold/ArtCraft suite, these paradigms map directly to our 13 clean-room applications and shared libraries:

| Paradigm | Domain Reference & Format Specification | ArtCraft Application | Core Shared Crates | Primary Document Invariant |
| :--- | :--- | :--- | :--- | :--- |
| **Raster Imaging** | Layered Raster Imaging (`.psd`/`.psb`) | **PhotoCraft** (`storytold/photocraft`) | `craft-psd`, `craft-media`, `craft-color` | Pixel fidelity, layer graph, non-destructive masks, and compositing |
| **Vector Geometry** | Precision Vector Artwork (`.ai`/`.eps`/`.svg`) | **VectorCraft** (`storytold/vectorcraft`) | `craft-geom`, `craft-path`, `craft-pdf` | Cubic Bézier geometry, appearance stacks, and resolution-independent paths |
| **Multipage Publishing** | Document Layout & Preflight (`.indd`/`.idml`/`.pdf`) | **DesignCraft** (`storytold/designcraft`), **PdfCraft** (`storytold/pdfcraft`) | `craft-pdf`, `craft-archive`, `craft-text` | Page geometry, typography, story flow, linked assets, and pagination stability |
| **Temporal Media & Animation** | Multi-Track Timeline & Vector Animation (`.fla`/`.xfl`/`.swf`) | **EffectCraft** (`storytold/effectcraft`), **FilmCraft** (`storytold/filmcraft`) | `craft-time`, `craft-media`, `craft-isobmff`, `craft-matroska` | Hierarchical timelines, nested symbols, property tweens, and causal monotonicity |

---

## Part I: Raster Imaging & Compositing Reference (Raster Domain)

Raster imaging pipelines support a broad range of file formats, but capabilities vary significantly depending on the format revision, installed components, and operating system.

A complete inventory needs to distinguish six operations:

1. **Open**: Read a file into the raster application.
2. **Import**: Place external content into an existing document.
3. **Edit**: Modify the imported content, including layers, vectors, metadata, or pixels.
4. **Save**: Write the document back to its original format.
5. **Export**: Generate another format from the document.
6. **Convert**: Transform content between formats, potentially changing color models, bit depth, layers, transparency, or other properties.

Importantly, being able to open a format does not mean the application can save it, preserve its structure, or round-trip it without losing information.

The following is a comprehensive working inventory for layered raster workflows. Exact support can vary by engine and platform; this is not a version-specific, experimentally verified compatibility matrix.

---

### 1. Native Layered Raster Formats

These formats provide the highest levels of layered document fidelity.

| Format | Extension | Open | Save | Layers | Transparency | Key Details |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **PSD Document** | `.psd` | Yes | Yes | Yes | Yes | Primary editable format |
| **Large Document Format** | `.psb` | Yes | Yes | Yes | Yes | Large documents exceeding PSD limits |
| **PSD Template** | `.psdt` | Yes | Yes | Yes | Yes | Reusable document templates |
| **Raw Raster** | `.raw` | Yes | Yes | No | Limited | Headerless raster data |
| **Raster EPS** | `.eps` | Yes | Yes | No | Limited | Encapsulated PostScript |
| **DCS 1.0 Prepress** | `.eps` | Yes | Legacy/version-dependent | No | No | Prepress color separations |
| **DCS 2.0 Prepress** | `.eps` | Yes | Legacy/version-dependent | No | No | Multichannel prepress separations |

#### PSD: Layered Document Specification

**Extension:** `.psd`

PSD is the primary layered interchange and working format. It can preserve:

* Raster layers
* Layer groups
* Adjustment layers
* Fill layers
* Layer masks
* Vector masks
* Smart Objects
* Smart Filters
* Layer effects
* Blend modes
* Opacity
* Text layers
* Shape layers
* Alpha channels
* Spot channels
* Paths
* Clipping masks
* Color profiles
* Guides
* Metadata
* Artboards
* Timeline-related document information

##### Technical Characteristics

| Property | PSD |
| :--- | :--- |
| Maximum dimensions | 30,000 × 30,000 pixels |
| Maximum file size | Approximately 2 GB |
| Color depth | 8, 16, 32 bits/channel, subject to mode |
| RGB | Yes |
| CMYK | Yes |
| Grayscale | Yes |
| Lab | Yes |
| Indexed Color | Yes |
| Multichannel | Yes |
| Bitmap | Yes |
| Layers | Yes |
| Compression | RLE (PackBits), ZIP, ZIP prediction, uncompressed variants |
| ICC profiles | Yes |
| EXIF/XMP | Supported |
| Lossless editing | Yes, depending on operations |

PSD is also a container format. Its internal structure includes a file header, color-mode data, image resources, layer and mask information, and composite image data.

A PSD reader therefore has several independent responsibilities: decoding pixels, reconstructing the layer graph, interpreting resource blocks, and reproducing standard blend-mode compositing behavior.

A decoder that displays the composite image is not necessarily a full PSD implementation.

#### PSB: Large Document Format

**Extension:** `.psb`

PSB extends PSD for very large documents.

| Property | PSB |
| :--- | :--- |
| Maximum dimensions | 300,000 × 300,000 pixels |
| Maximum file size | Approximately 4 exabytes, theoretical format limit |
| Layers | Yes |
| Smart Objects | Yes |
| Masks | Yes |
| Adjustment layers | Yes |
| Color profiles | Yes |
| 16/32-bit workflows | Yes |

PSB is particularly relevant for large photographic composites, scientific imagery, panoramic images, and high-resolution production artwork. Although its structure resembles PSD, PSB uses 64-bit field lengths in place of 32-bit offsets in key sections of its binary structure.

---

### 2. Common Raster Image Formats

These are the formats most frequently encountered in everyday image editing.

| Format | Extensions | Open | Save/export | Alpha | Layers | Typical Bit Depth |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **JPEG** | `.jpg`, `.jpeg`, `.jpe` | Yes | Yes | No | No | 8-bit |
| **PNG** | `.png` | Yes | Yes | Yes | No | 8/16-bit |
| **TIFF** | `.tif`, `.tiff` | Yes | Yes | Yes | Sometimes | 1/8/16/32-bit |
| **GIF** | `.gif` | Yes | Yes | 1-bit transparency | Limited animation | 8-bit indexed |
| **BMP** | `.bmp`, `.dib` | Yes | Yes | Variant-dependent | No | 1/4/8/16/24/32-bit |
| **WebP** | `.webp` | Yes | Yes | Yes | No | Usually 8-bit |
| **AVIF** | `.avif` | Version-dependent | Version-dependent | Yes | No | 8/10/12-bit source variants |
| **HEIF** | `.heif`, `.heic` | Platform/version-dependent | Platform/version-dependent | Supported by format | No | 8/10-bit common |
| **JPEG 2000** | `.jp2`, `.j2k`, `.jpf`, `.jpx` | Component/version-dependent | Component/version-dependent | Supported by format | No | Variable |
| **JPEG XL** | `.jxl` | Version/plugin-dependent | Version/plugin-dependent | Yes | No | Variable |
| **Targa** | `.tga`, `.vda`, `.icb`, `.vst` | Yes | Yes | Yes | No | 8/16/24/32-bit |
| **OpenEXR** | `.exr` | Yes | Yes | Yes | No PSD layer round-trip | 16/32-bit float |
| **Radiance HDR** | `.hdr`, `.rgbe` | Yes | Yes | No conventional alpha | No | HDR |
| **Portable Bit Map** | `.pbm` | Yes | Yes | No | No | 1-bit |
| **PICT** | `.pct`, `.pict` | Legacy | Legacy | Limited | No | Variable |
| **PCX** | `.pcx` | Legacy/version-dependent | Legacy/version-dependent | Limited | No | Variable |

Bit depths describe the underlying format’s capabilities or common source variants, not a guarantee that an editor preserves every variant when saving.

#### JPEG

**Extensions:** `.jpg`, `.jpeg`, `.jpe`

JPEG uses lossy compression optimized for photographic imagery.

* DCT-based compression in conventional JPEG.
* RGB, grayscale, and CMYK JPEG workflows.
* No standard alpha channel.
* No editable layers.
* EXIF metadata and ICC color profiles.
* Progressive and baseline encoding.
* Quality-controlled compression.

**Conversion Behavior:**

* **JPEG → PSD**: Pixels become an editable raster layer. Compression artifacts remain; original compression information is not recoverable.
* **PSD → JPEG**: Layers are composited; transparency must be resolved (matted against background color); high-bit-depth data downsampled; unsupported color modes converted.

Repeated JPEG encoding introduces progressive generational loss.

#### PNG

**Extension:** `.png`

PNG is a lossless raster format supporting RGB, RGBA, grayscale, grayscale with alpha, and indexed color. Supports 8-bit and 16-bit channel data with DEFLATE compression.

| Variant | Description |
| :--- | :--- |
| PNG-8 | Indexed palette, up to 256 colors |
| PNG-24 | 8-bit RGB |
| PNG-32 | 8-bit RGBA |
| PNG-48 | 16-bit RGB |
| PNG-64 | 16-bit RGBA |
| APNG | Animated PNG; not equivalent to standard single-frame PNG support |

PNG → PSD preserves appearance and transparency. PSD → PNG flattens/composites the editable layer structure.

#### TIFF

**Extensions:** `.tif`, `.tiff`

TIFF is one of the most flexible image containers. Capabilities include RGB, CMYK, Grayscale, Lab, Bitmap, alpha channels, multi-page directories, embedded ICC profiles, uncompressed data, LZW, ZIP, PackBits, and JPEG compression variants.

##### Layered TIFF

Layer graphs can be preserved inside TIFF using ImageSourceData tags. However, generic TIFF decoders will only read the baseline composite image. Support must be distinguished between baseline TIFF, extended TIFF, BigTIFF, and layered TIFF.

---

### 3. Camera RAW Formats

Camera RAW files represent unprocessed sensor data:

* Sensor mosaic data (Bayer, X-Trans, etc.)
* Camera-specific black levels and white balance
* Color calibration matrices and lens profiles
* Proprietary metadata and compression tables

Opening a RAW file involves demosaicing and rendering into the working color space via a camera RAW demosaicing engine.

| Manufacturer | Formats | Typical Support Engine |
| :--- | :--- | :--- |
| **Digital Negative** | `.dng` | Camera RAW / Open DNG |
| **Canon** | `.cr2`, `.cr3`, `.crw` | Camera Raw |
| **Nikon** | `.nef`, `.nrw` | Camera Raw |
| **Sony** | `.arw`, `.srf`, `.sr2` | Camera Raw |
| **Fujifilm** | `.raf` | Camera Raw |
| **Olympus/OM System** | `.orf` | Camera Raw |
| **Panasonic** | `.rw2`, `.raw` | Camera Raw |
| **Leica** | `.rwl`, `.dng` | Camera Raw |
| **Pentax** | `.pef`, `.dng` | Camera Raw |
| **Samsung** | `.srw` | Camera Raw |
| **Hasselblad** | `.3fr`, `.fff` | Camera Raw |
| **Phase One** | `.iiq` | Camera Raw |
| **Kodak** | `.dcr`, `.k25`, `.kdc` | Camera Raw |
| **Sigma** | `.x3f` | Camera Raw |
| **Apple** | `.dng` | ProRAW through Camera Raw |

#### DNG: Digital Negative

A documented RAW container based on TIFF/EP containing RAW mosaic sensor data, linear RAW data, embedded previews, opcode lists, and lossless/lossy compression.

---

### 4. Vector and Document Formats in Raster Applications

| Format | Extension | Open | Place/Import | Save/Export | Vector Preservation |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **PDF** | `.pdf` | Yes | Yes | Yes | Partial/workflow-dependent |
| **AI Vector** | `.ai` | Yes | Yes | Vector source encapsulated | Embedded as smart layer when placed |
| **EPS** | `.eps` | Yes | Yes | Yes | Usually rasterized on direct open |
| **SVG** | `.svg` | Workflow/version-dependent | Yes | Export supported in some workflows | Partial |
| **PostScript** | `.ps` | Yes | Limited | No general PS authoring | Rasterized |

Opening PDF/AI/EPS directly in a raster editor rasterizes the document at a chosen resolution, crop box, and color mode. Placing them encapsulates the vector source within a smart layer, preserving scalability.

---

### 5. High-Dynamic-Range (HDR) and Scientific Imaging

| Format | Extension | HDR | Floating Point | Alpha | Multichannel |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **OpenEXR** | `.exr` | Yes | 16/32-bit float | Yes | Yes, at format level |
| **Radiance HDR** | `.hdr` | Yes | RGBE encoding | No standard alpha | Limited |
| **TIFF** | `.tif` | Yes | Yes | Yes | Yes |
| **PSD/PSB** | `.psd`, `.psb` | Yes | Yes (32 bpc) | Yes | Yes |

OpenEXR supports half-float, full-float, arbitrary channel naming, multiple parts, tiled images, and deep data. Standard raster OpenEXR readers do not guarantee preservation of multipart or deep-data channels.

---

### 6. Legacy & Specialized Image Formats

| Format | Extension | Purpose | Status |
| :--- | :--- | :--- | :--- |
| **Targa** | `.tga` | Games, rendering, video | Supported |
| **BMP / DIB** | `.bmp`, `.dib` | Windows bitmap | Supported |
| **PCX** | `.pcx` | Legacy PC graphics | Legacy |
| **PICT** | `.pict`, `.pct` | Classic Mac QuickDraw | Legacy |
| **IFF / ILBM** | `.iff`, `.ilbm` | Amiga graphics | Legacy / Plugin-dependent |
| **Pixar** | `.pxr` | 3D rendering | Legacy |
| **Scitex CT** | `.sct` | High-end prepress | Legacy |
| **Kodak Photo CD** | `.pcd` | Photo CD | Legacy |
| **WBMP** | `.wbmp` | Wireless bitmap | Legacy |
| **DDS / KTX / ASTC** | `.dds`, `.ktx`, `.astc` | GPU compressed textures | Plugin-dependent |

---

### 7. Video and Animation Formats

| Format | Extension | Import | Export | Notes |
| :--- | :--- | :--- | :--- | :--- |
| **MP4** | `.mp4` | Yes (supported codecs) | Yes | H.264 / AAC container |
| **MOV** | `.mov` | Yes (supported codecs) | Yes | QuickTime container |
| **AVI** | `.avi` | Platform-dependent | Version-dependent | Legacy Windows video |
| **GIF** | `.gif` | Yes | Yes | Frame animation |
| **Image sequence** | `.png`, `.jpg`, etc. | Yes | Yes | Frame-based workflow |

Video import does not guarantee decoding of every internal codec stream (e.g. ProRes, DNxHD, AV1 require platform codecs).

---

### 8. 3D and Texture Formats

Legacy 3D features (Wavefront `.obj`, Collada `.dae`, 3D Studio `.3ds`, U3D `.u3d`, STL `.stl`, GLB/glTF `.gltf`) have been discontinued in modern releases.

---

### 9. Color Modes & Conversion

Professional raster workflows support RGB, CMYK, Grayscale, Bitmap (1-bit), Indexed Color (8-bit palette), Lab Color ($L^*a^*b^*$), Duotone, and Multichannel modes.

* **RGB → CMYK**: Out-of-gamut colors clipped/compressed via ICC rendering intent.
* **CMYK → RGB**: Irreversible loss of original CMYK separation plates.
* **16/32-bit → 8-bit**: Loss of highlight recovery and dynamic range latitude.

---

### 10. Metadata Compatibility

* **EXIF**: Camera parameters, lens data, shutter, aperture, GPS.
* **IPTC / IIM**: Editorial metadata, copyright, captions, credits.
* **XMP**: Extensible XML metadata packet embedded in JPEG/TIFF/PSD/PNG.
* **ICC Profiles**: Color management input/output transform profiles.
* **C2PA**: Content Credentials provenance records.

---

### 11. Raster Conversion Compatibility Matrix

Legend: `✓` Commonly supported; `◐` Conditional or lossy; `-` Not a general supported workflow.

| Source | PSD | PNG | JPEG | TIFF | PDF | WebP | EXR |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| **PSD** | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| **PSB** | ◐ | ◐ | ◐ | ◐ | ◐ | ◐ | ◐ |
| **PNG** | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| **JPEG** | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| **TIFF** | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| **GIF** | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| **WebP** | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| **HEIC** | ◐ | ◐ | ◐ | ◐ | ◐ | ◐ | ◐ |
| **Camera RAW** | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| **PDF** | ◐ | ◐ | ◐ | ◐ | ◐ | ◐ | ◐ |
| **AI** | ◐ | ◐ | ◐ | ◐ | ◐ | ◐ | ◐ |
| **SVG** | ◐ | ◐ | ◐ | ◐ | ◐ | ◐ | ◐ |
| **EXR** | ✓ | ◐ | ◐ | ◐ | ◐ | ◐ | ✓ |

---

### 12. Structural Preservation Matrix (Raster Domain)

| Property | PSD | TIFF | PNG | JPEG | PDF | EXR |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: |
| **Raster pixels** | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| **Layer graph & masks** | ✓ | ◐ | - | - | ◐ | - |
| **Layer groups** | ✓ | ◐ | - | - | ◐ | - |
| **Adjustment layers** | ✓ | ◐ | - | - | ◐ | - |
| **Smart Objects** | ✓ | ◐ | - | - | ◐ | - |
| **Editable text** | ✓ | ◐ | - | - | ◐ | - |
| **Vector paths** | ✓ | ◐ | - | - | ✓ | - |
| **Alpha transparency** | ✓ | ✓ | ✓ | - | ✓ | ✓ |
| **HDR float** | ✓ | ✓ | ◐ | - | ◐ | ✓ |
| **ICC profiles** | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| **EXIF metadata** | ◐ | ✓ | ◐ | ✓ | ◐ | - |
| **Lossless compression** | ✓ | ✓ | ✓ | - | ◐ | ✓ |
| **Animation** | ◐ | - | - | - | ◐ | - |

---

### 13. Architectural Families & Implementation Problems

| Family | Formats | Core Implementation Problem |
| :--- | :--- | :--- |
| **Native Layered Documents** | PSD, PSB | Layer graph, non-destructive masks, blend modes, compositing |
| **Conventional Raster** | JPEG, PNG, BMP, GIF, WebP | High-performance SIMD pixel decoding, colorspace transform |
| **Extended Containers** | TIFF, HEIF, AVIF, JPEG 2000 | Container directory trees, tile/strip streams, auxiliary alpha |
| **Camera RAW** | DNG, CR3, NEF, ARW, RAF | CFA mosaic decoding, black/white calibration, demosaicing |
| **Vector Documents** | PDF, AI, EPS, SVG | Path rasterization, antialiasing, clipping hierarchy |
| **HDR & Scientific** | EXR, HDR, 32-bit TIFF | IEEE 754 half/single float, tone mapping, wide gamuts |
| **Video & Temporal** | MP4, MOV, GIF, sequences | Demuxing, timestamp alignment, keyframe seeking |
| **Specialized & Legacy** | PCX, PICT, TGA, DDS, KTX | Byte packing, palette expansion, hardware block decompression |

---

### 14. Recommended Capability Model (Raster Domain)

```typescript
interface FormatCapability {
  format: string;
  extensions: string[];
  mimeTypes: string[];
  operations: {
    detect: boolean;
    inspect: boolean;
    decode: boolean;
    encode: boolean;
    import: boolean;
    export: boolean;
    convert: boolean;
  };
  image: {
    raster: boolean;
    vector: boolean;
    animated: boolean;
    hdr: boolean;
    alpha: boolean;
    layers: boolean;
    multichannel: boolean;
  };
  color: {
    modes: string[];
    bitDepths: number[];
    iccProfiles: boolean;
  };
  preservation: {
    layers: boolean;
    text: boolean;
    vectors: boolean;
    metadata: boolean;
    colorProfile: boolean;
    alpha: boolean;
  };
  limitations: string[];
}
```

---

## Part II: Vector Geometry & Illustration Reference (Vector Domain)

Vector illustration differs fundamentally from raster imaging because its primary document model is geometry-based rather than pixel-based. Raster engines manipulate pixels, while vector engines operate on paths, appearance stacks, typography, and structured artwork.

---

### 1. Vector Format Inventory

Legend: `✓` Supported; `◐` Partial/conditional; `-` Not supported natively; `P` Placement/linking; `E` Export-oriented.

#### 1.1 Native and Vector Document Formats

| Format | Extensions | Open | Place | Save/Export | Native Vector Editing |
| :--- | :--- | :---: | :---: | :---: | :---: |
| **AI Artwork** | `.ai` | ✓ | ✓ | ✓ | ✓ |
| **AI Template** | `.ait` | ✓ | ✓ | ◐ | ✓ |
| **ISO 32000 PDF** | `.pdf` | ✓ | ✓ | ✓ | ◐ |
| **Vector EPS** | `.eps` | ✓ | ✓ | ✓ | ◐ |
| **Encapsulated PostScript** | `.eps`, `.epsf` | ✓ | ✓ | ◐ | ◐ |
| **PostScript** | `.ps` | ✓ | ◐ | ◐ | ◐ |
| **SVG** | `.svg` | ✓ | ✓ | ✓ | ◐ |
| **Compressed SVG** | `.svgz` | ◐ | ◐ | ◐ | ◐ |
| **CDR Vector Drawing** | `.cdr` | ◐ (v5–10) | ◐ | - | ◐ |
| **DWG Drawing Database** | `.dwg` | ✓ | ✓ | ✓ | ◐ |
| **DXF Drawing Exchange Format** | `.dxf` | ✓ | ✓ | ✓ | ◐ |
| **Computer Graphics Metafile** | `.cgm` | ✓ | ✓ | ◐ | ◐ |
| **Enhanced Metafile** | `.emf` | ✓ | ✓ | ✓ | ◐ |
| **Windows Metafile** | `.wmf` | ◐ | ◐ | ◐ | ◐ |

#### 1.2 Raster Image Formats in Vector Applications

| Format | Extensions | Open/Place | Export | Transparency | Vector Preservation |
| :--- | :--- | :---: | :---: | :---: | :---: |
| **Layered PSD** | `.psd` | ✓ | ✓ | ✓ | Partial |
| **JPEG** | `.jpg`, `.jpeg`, `.jpe` | ✓ | ✓ | No | No |
| **PNG** | `.png` | ✓ | ✓ | Yes | No |
| **TIFF** | `.tif`, `.tiff` | ✓ | ✓ | Yes | No |
| **GIF** | `.gif` | ✓ | ✓ | Limited | No |
| **BMP** | `.bmp`, `.dib`, `.rle` | ✓ | ✓ | Variant-dependent | No |
| **WebP** | `.webp` | ✓ | ✓ | Yes | No |
| **AVIF** | `.avif` | ✓ | ◐ | Yes | No |
| **HEIC/HEIF** | `.heic`, `.heif` | ◐ | ◐ | Yes | No |
| **Targa** | `.tga` | ◐ | ◐ | Yes | No |
| **JPEG 2000** | `.jp2`, `.j2k` | ◐ | - | Yes | No |
| **OpenEXR** | `.exr` | ◐ | - | Yes | No |
| **Radiance HDR** | `.hdr` | ◐ | - | No | No |

---

### 2. Native Vector Formats

#### 2.1 AI - Vector Artwork Format Specification

Modern AI files utilize a dual-stream architecture when *Create PDF Compatible File* is enabled:

```text
AI Vector Document (.ai)
│
├── Document Metadata (XMP)
│
├── Private Editing Stream
│   ├── Artboards
│   ├── Layers & Sublayers
│   ├── Paths & Compound Paths
│   ├── Appearance Stacks (Multiple Fills/Strokes)
│   ├── Text Objects & Text-on-Path
│   ├── Symbols & Brushes
│   └── Live Effects & Graphic Styles
│
├── Embedded / Linked Assets
│
├── Color Profiles & Swatches (Process & Spot)
│
└── PDF-Compatible Stream
    ├── Page Geometry & Viewports
    ├── Content Stream Drawing Operators
    ├── Embedded Type 1 / TrueType / CFF Fonts
    └── Flattend Transparency Groups
```

* **PDF-Compatible AI**: PDF parsers can render the file via the PDF stream, but this stream does not expose the native vector editing model.
* **Non-PDF-Compatible AI**: Pure PostScript/private stream; generic PDF readers fail completely.

---

### 3. PDF Compatibility in Vector Engines

| Feature | PDF Format Standard | Vector Editing Representation |
| :--- | :---: | :---: |
| **Vector paths** | Yes | Usually |
| **Raster images** | Yes | Yes |
| **Embedded fonts** | Yes | Conditional on local system font |
| **Live text** | Yes | Conditional |
| **Transparency** | Yes | Usually |
| **Gradients** | Yes | Usually |
| **Clipping paths** | Yes | Usually |
| **Spot colors** | Yes | Yes |
| **ICC profiles** | Yes | Yes |
| **Multiple pages** | Yes | Converted to multiple Artboards |
| **Layers / OCGs** | Yes | Partial |

Converting generic PDF to AI may flatten layer structures, expand live brushes, convert text to outlines if fonts are missing, and fragment compound shapes.

---

### 4. SVG & Geometric Math

SVG defines vector artwork via an XML scene graph.

#### 4.1 Cubic Bézier Curve Geometry

The vector geometry engine is fundamentally cubic Bézier:

$$\mathbf{B}(t) = (1-t)^3 \mathbf{P}_0 + 3(1-t)^2 t \mathbf{P}_1 + 3(1-t) t^2 \mathbf{P}_2 + t^3 \mathbf{P}_3, \quad t \in [0, 1]$$

When importing SVG quadratic Bézier curves ($\mathbf{Q}(t)$ with control point $\mathbf{Q}_1$), the degree must be elevated to cubic control points:

$$\mathbf{P}_1 = \mathbf{P}_0 + \frac{2}{3}(\mathbf{Q}_1 - \mathbf{P}_0), \quad \mathbf{P}_2 = \mathbf{P}_3 + \frac{2}{3}(\mathbf{Q}_1 - \mathbf{P}_3)$$

SVG elliptical arcs (`A` commands) are similarly approximated by piecewise cubic Bézier splines (maximum angular span $\le \pi/2$ per segment).

---

### 5. CAD Formats (DWG / DXF)

* **DWG**: Proprietary binary CAD database.
* **DXF**: ASCII/binary drawing interchange format.

| DXF Entity | Description | Vector Representation |
| :--- | :--- | :--- |
| `LINE` | Straight segment | 2-point Path |
| `LWPOLYLINE` | Lightweight 2D polyline | Multi-segment Path (arcs expanded) |
| `POLYLINE` | 2D/3D polyline | Path |
| `ARC` | Circular arc | Cubic Bézier approximation |
| `CIRCLE` | Circle | 4-point Bézier path |
| `ELLIPSE` | Ellipse | 4-point Bézier path |
| `SPLINE` | Non-uniform rational B-spline | Cubic Bézier approximation |
| `TEXT` / `MTEXT` | Text / Multiline text | Text Frame (conditional on font) |
| `HATCH` | Hatch boundary/pattern | Closed Path with Pattern Fill |
| `INSERT` | Block reference | Group or Symbol Instance |
| `DIMENSION` | Associative dimension | Expanded lines and text (loses associativity) |

---

### 6. Appearance System & Conversion Losses

A vector object can possess an arbitrary appearance stack:

```text
Path Object
├── Fill 1: Spot Pantone 185 C (Opacity: 100%)
├── Fill 2: Radial Gradient (Screen Blend Mode, Opacity: 60%)
├── Stroke 1: 2 pt White (Inside Alignment)
├── Stroke 2: 8 pt Black (Outside Alignment)
├── Live Effect: Drop Shadow (Gaussian Blur)
└── Opacity Mask (Inverted Grayscale Texture)
```

Exporting to formats that lack this stack (e.g. DXF, SVG, EPS) forces an appearance strategy:

1. **Translate**: Approximate using multiple duplicate objects.
2. **Expand**: Break down into primitive stroked and filled paths.
3. **Flatten**: Resolve transparency into opaque abutting fragments.
4. **Rasterize**: Convert entire appearance into pixel buffer.

---

### 7. Vector Structural Preservation Matrix

| Property | AI | PDF | SVG | EPS | DXF | PSD |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: |
| **Vector paths** | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ |
| **Editable text** | ✓ | ◐ | ✓ | ◐ | ◐ | ✓ |
| **Layers** | ✓ | ◐ | ◐ | Limited | ✓ | ✓ |
| **Groups** | ✓ | ◐ | ✓ | Limited | ◐ | ✓ |
| **Artboards** | ✓ | Pages | Viewports | Limited | None | ◐ |
| **Clipping paths** | ✓ | ✓ | ✓ | ✓ | Limited | ✓ |
| **Transparency** | ✓ | ✓ | ✓ | Limited | Limited | ✓ |
| **Gradients** | ✓ | ✓ | ✓ | ◐ | Limited | ✓ |
| **Gradient meshes** | ✓ | ◐ | Limited | Limited | - | Rasterized |
| **Live effects** | ✓ | ◐ | Limited | - | - | Rasterized |
| **Brushes** | ✓ | Expanded | Expanded | Expanded | - | Rasterized |
| **Symbols** | ✓ | Limited | ✓ | Limited | Blocks | - |
| **ICC profiles** | ✓ | ✓ | ◐ | ◐ | Limited | ✓ |
| **Spot colors** | ✓ | ✓ | Limited | ✓ | Limited | ◐ |
| **Linked assets** | ✓ | ◐ | ✓ | Limited | ✓ | ◐ |

---

### 8. Recommended Vector Engine Architecture

```text
                  INPUT VECTOR FILE
                          │
                          ▼
               Format Identification
                          │
                          ▼
                 Parser / Decoder
                          │
                          ▼
              Intermediate Document (IR)
                          │
       ┌──────────────────┼──────────────────┐
       │                  │                  │
       ▼                  ▼                  ▼
    Geometry          Appearance         Resources
    (Cubic Paths,     (Fills, Strokes,   (Fonts, Images,
     Transforms)       Shaders, Masks)    ICC Profiles)
       │                  │                  │
       └──────────────────┼──────────────────┘
                          │
                          ▼
                 Conversion Planner
                          │
            ┌─────────────┴─────────────┐
            │                           │
            ▼                           ▼
    Preserve Structure           Expand / Flatten
            │                           │
            └─────────────┬─────────────┘
                          │
                          ▼
                    Output Encoder
                          │
                          ▼
                  OUTPUT VECTOR FILE
```

---

## Part III: Multipage Publishing Reference (Publishing Domain)

Publishing engines operate on structured, multipage publications, coordinating typography, page geometry, linked assets, reusable styles, and publishing output.

---

### 1. Multipage Publishing Format Inventory

| Format | Extension | Open | Save/Create | Purpose |
| :--- | :--- | :---: | :---: | :--- |
| **Binary Publication** | `.indd` | Yes | Yes | Primary native editable document (binary) |
| **Publication Template** | `.indt` | Yes | Yes | Reusable publication template |
| **IDML Package** | `.idml` | Yes | Yes | Open XML-based package interchange |
| **Book Coordinator** | `.indb` | Yes | Yes | Multi-document publication coordinator |
| **Asset Library** | `.indl` | Yes | Yes | Reusable layout asset storage |
| **Layout Snippet** | `.idms` | Place | Create | Reusable layout fragment |
| **Legacy XML Interchange** | `.inx` | Legacy | Legacy | Pre-IDML XML interchange format |
| **InCopy Document** | `.icml` | Place | Export | Editorial content collaboration |
| **InCopy Assignment** | `.icma` | Workflow | Workflow | Editorial story assignments |
| **InCopy Package** | `.icap` | Workflow | Workflow | Compressed editorial assignment handoff |

---

### 2. IDML - Open Publication Markup Architecture

IDML is an open ZIP archive containing XML streams representing the document object model:

```text
publication.idml (ZIP Archive)
├── mimetype (text/plain: "application/vnd.adobe.indesign-idml-package")
├── designmap.xml (Root manifest linking resources and spread files)
│
├── META-INF/
│   └── container.xml
│
├── MasterSpreads/
│   └── MasterSpread_*.xml (Parent page geometry, margins, running heads)
│
├── Spreads/
│   └── Spread_*.xml (Page items, text frames, geometric transforms)
│
├── Stories/
│   └── Story_*.xml (Threaded text flows, paragraph/character runs)
│
├── Resources/
│   ├── Styles.xml (Paragraph, character, object, table, cell styles)
│   ├── Fonts.xml (Font families, PostScript names, font styles)
│   ├── Graphic.xml (Swatches, inks, gradients, stroke styles)
│   └── Preferences.xml (Document units, baseline grid, snap rules)
│
└── XML/
    ├── Tags.xml (Structure tags)
    └── BackingStory.xml (XML structure tree)
```

IDML preserves full styling, story threading, object styles, parent page links, and geometric frames. It represents the primary clean-room interchange format for publishing workflows.

---

### 3. Graphics Placement & Asset Linking

Publishing engines maintain explicit external links rather than embedding heavy raster/vector assets by default:

```text
Document.indd
  ├── cover.psd (Linked: Layer comps, alpha channel clipping)
  ├── illustration.ai (Linked: Artboard 2, crop to bleed box)
  ├── photo01.tif (Linked: 300 DPI, embedded standard RGB profile)
  └── architecture.dwg (Imported & expanded)
```

* **Placing PSD**: Allows toggling individual layer visibility, choosing layer comps, and applying embedded clipping paths.
* **Placing AI**: Allows selecting specific Artboards, crop boxes (Media, Bleed, Trim, Art), and layer overrides.
* **Placing PDF**: Renders selected PDF pages with full passthrough print compatibility.

---

### 4. Editorial Collaboration & Text Import (Word / ICML)

* **Word (.docx)**: The layout engine maps Word styles to publication paragraph/character styles during import. Tables, inline footnotes, and endnotes are converted into native publication constructs.
* **InCopy (.icml)**: Decouples editorial text from page layout. Writers edit stories in InCopy without risking page geometry or spread layout alterations.

---

### 5. Typography, Pagination & Composition Invariants

Text in multipage publication layouts does not merely sit in isolated boxes; it flows across threaded frame sequences:

$$\text{Story} \longrightarrow \text{Frame}_1 \longrightarrow \text{Frame}_2 \longrightarrow \text{Frame}_3 \longrightarrow [\text{Overset}]$$

Composition depends strictly on:

1. Multi-line paragraph composer (Knuth-Plass style global hyphenation and justification optimization).
2. OpenType features (ligatures, small caps, oldstyle figures, stylistic alternates).
3. Baseline grid alignment.
4. Optical margin alignment.
5. Text wrap boundaries around overlapping graphic frames.

> **Critical Composition Invariant**: A single font substitution or metric discrepancy of 0.5 pt can cause a line break to shift, triggering a cascading reflow that pushes headings across column boundaries, causes text overset, and corrupts table-of-contents pagination across a 500-page book.

---

### 6. Publishing Structural Preservation Matrix

| Feature | INDD | IDML | PDF | EPUB | HTML |
| :--- | :---: | :---: | :---: | :---: | :---: |
| **Pages & Spreads** | ✓ | ✓ | ✓ | ◐ | ◐ |
| **Parent Pages** | ✓ | ✓ | - | - | - |
| **Text frames** | ✓ | ✓ | Rendered | ◐ | ◐ |
| **Threaded stories** | ✓ | ✓ | - | ◐ | ◐ |
| **Paragraph / Character styles** | ✓ | ✓ | Rendered | CSS | CSS |
| **Object styles** | ✓ | ✓ | - | - | - |
| **Tables** | ✓ | ✓ | Rendered | ✓ | ✓ |
| **Linked assets** | ✓ | ✓ | Embedded | Packaged | Linked |
| **Editable typography** | ✓ | ✓ | ◐ | ✓ | ✓ |
| **Vector artwork** | ✓ | ✓ | ✓ | ◐ | ◐ |
| **Color profiles & Spot colors** | ✓ | ✓ | ✓ | - | - |
| **Bleed & Slug** | ✓ | ✓ | ✓ | - | - |
| **Bookmarks & Hyperlinks** | ✓ | ✓ | ✓ | ✓ | ✓ |
| **Accessibility tags** | ✓ | ◐ | ✓ | ✓ | ✓ |

---

## Part IV: Temporal Media & Animation Reference (Temporal Animation Domain)

Temporal vector animation systems work with time-dependent visual objects, reusable symbols, animation timelines, and interactive behavior. A static document is a scene evaluated at one moment; an animation document is a scene whose state changes continuously over time.

---

### 1. Temporal Animation Format Inventory

Legend: `✓` Supported; `◐` Conditional/partial; `-` Not native; `Legacy` Historical workflow.

| Format | Extension | Open | Save | Export/Publish | Editable Timeline |
| :--- | :--- | :---: | :---: | :---: | :---: |
| **Binary Animation Archive** | `.fla` | ✓ | ✓ | - | ✓ |
| **Uncompressed XFL Package** | `.xfl` | ✓ | ✓ | - | ✓ |
| **SWF Vector Animation** | `.swf` | ◐ | - | ✓ | Compiled bytecode |
| **ActionScript** | `.as` | Source | ✓ | Compiled into target | Code |
| **HTML5 Canvas** | `.html`, `.js` | - | - | ✓ | Runtime representation |
| **WebGL / glTF** | `.html`, `.gltf` | - | - | ◐ | Runtime representation |
| **Animated GIF** | `.gif` | ✓ | - | ✓ | Frame reconstruction |
| **SVG** | `.svg` | ✓ | - | ✓ | Static / Frame-by-frame |
| **PNG / JPEG Sequence** | `.png`, `.jpg` | ✓ | - | ✓ | Frame reconstruction |
| **Video** | `.mp4`, `.mov` | ◐ | - | ✓ | Flattened video |
| **Standalone Runtime Package** | `.air`, `.apk` | - | - | ✓ | Compiled application |

---

### 2. FLA & XFL Internal Document Architecture

Modern `.fla` projects are zipped packages of the open `.xfl` directory format:

```text
Animation.xfl Directory Structure
├── DOMDocument.xml (Stage dimensions, frame rate, background color, layer manifest)
│
├── LIBRARY/
│   ├── CharacterRig.xml (Nested timeline, bone symbols)
│   ├── ArmSymbol.xml (Graphic symbol with internal keyframes)
│   ├── AudioTrack.xml (Sample rate, sound envelope)
│   └── BackgroundBitmap.xml (Image asset descriptor)
│
├── bin/
│   └── binary image and audio assets
│
└── PublishSettings.xml (HTML5 Canvas / WebGL / AIR configuration)
```

#### Document Components

* `DOMTimeline`: Root timeline structure.
* `DOMLayer`: Layers (Normal, Guide, Mask, Masked, Folder).
* `DOMFrame`: Frame spans, keyframes, motion tween descriptors.
* `DOMSymbolItem`: Reusable library symbols.
* `DOMSymbolInstance`: Transformed placements on the stage.

---

### 3. SWF - Vector Animation Bytecode Specification

SWF is a runtime display list and bytecode container:

* **Headers**: `FWS` (uncompressed), `CWS` (zlib), `ZWS` (LZMA).
* **Tag Stream**: Definition tags (shapes, fonts, bitmaps, sounds) and Control tags (place object, remove object, show frame).
* **Decompilation Boundary**: SWF → FLA is inherently lossy; symbol hierarchies, motion tween curves, and variable names are compiled away into per-frame display-list operations.

---

### 4. Animation Hierarchy, Symbols & Tweens

Timeline animation documents enforce hierarchical evaluation:

$$\text{Root Timeline (Frame } t\text{)} \longrightarrow \text{Symbol Instance (Local Frame } \tau\text{)} \longrightarrow \text{Nested MovieClip (Autonomous Frame } \theta\text{)}$$

#### Symbol Types

1. **Graphic Symbol**: Synchronized strictly to the parent timeline. Playback modes: Loop, Play Once, Single Frame.
2. **Movie Clip**: Autonomous timeline running independently of the parent timeline, scriptable via ActionScript/JavaScript.
3. **Button**: 4-state interactive object (`Up`, `Over`, `Down`, `Hit`).

#### Tweening Systems

* **Motion Tweens**: Property curve interpolation over time (Position, Rotation, Scale, Skew, Color Effect, Alpha).
* **Classic Tweens**: Keyframe-to-keyframe transform interpolation.
* **Shape Tweens**: Vector geometry morphing using contour-matching algorithms and shape hints.

---

### 5. Animation Structural Preservation Matrix

| Property | FLA | XFL | SWF | HTML5 Canvas | GIF | MP4 |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: |
| **Stage Geometry** | ✓ | ✓ | ✓ | ✓ | Pixel dims | Pixel dims |
| **Layers** | ✓ | ✓ | Partial | Partial | No | No |
| **Keyframes** | ✓ | ✓ | Runtime | Runtime | Frames | Frames |
| **Motion Tweens** | ✓ | ✓ | Compiled | Runtime code | No | No |
| **Shape Tweens** | ✓ | ✓ | Compiled | Conditional | No | No |
| **Symbols & Hierarchy** | ✓ | ✓ | Runtime defs | Runtime defs | No | No |
| **Nested Timelines** | ✓ | ✓ | Yes | Conditional | No | No |
| **Vector Geometry** | ✓ | ✓ | ✓ | ✓ | No | No |
| **Audio Tracks** | ✓ | ✓ | ✓ | ✓ | No | Yes |
| **Scripts** | ✓ | ✓ | Bytecode | JavaScript | No | No |
| **Masks** | ✓ | ✓ | ✓ | Conditional | Rendered | Rendered |
| **Character Rigging** | ✓ | ✓ | No | No | No | No |
| **Interactivity** | ✓ | ✓ | ✓ | ✓ | No | No |

---

### 6. Modern Animation Interoperability (Lottie, Rive, Spine)

| Feature | Vector Animation (FLA/XFL) | Lottie (.json) | Rive (.riv) | Spine (.json/.skel) |
| :--- | :---: | :---: | :---: | :---: |
| **Vector Geometry** | Yes | Yes | Yes | Limited |
| **Timeline Animation** | Yes | Yes | Yes | Yes |
| **Nested Symbols** | Yes | Precompositions | Artboards | Attachments |
| **Skeletal Bones** | Specialized | Limited | Yes | Yes |
| **Mesh Deformation** | Specialized | Limited | Yes | Yes |
| **State Machines** | Script-based | External | Yes | Animation state |
| **Runtime Scripting** | AS3 / JS | Host API | Runtime API | Runtime API |
| **Audio Sync** | Event / Stream | Limited | Runtime | Runtime |

---

## Part V: Digital Audio Workstation & Spatial DSP Reference (SoundCraft Domain)

SoundCraft operates on multi-track audio recordings, time-aligned signal processing graphs, sample-accurate DSP automation, and spatial audio mastering.

---

### 1. Audio Format Inventory

| Format | Extension | Open/Import | Record/Export | Channels | Sample Depth | Primary Domain |
| :--- | :--- | :---: | :---: | :--- | :--- | :--- |
| **Broadcast Wave (BWF)** | `.wav` | ✓ | ✓ | 1–64+ | 16/24/32-bit float | Professional production & interchange |
| **RF64 (EBU Tech 3306)** | `.wav`, `.rf64` | ✓ | ✓ | Multichannel | 16/24/32-bit float | Large multi-hour recordings exceeding 4 GB |
| **ADM BWF (ITU-R BS.2076)** | `.wav` | ✓ | ✓ | Up to 128 | 24/32-bit float | Immersive & spatial audio (Dolby Atmos) |
| **AIFF / AIFC** | `.aif`, `.aiff` | ✓ | ✓ | Multichannel | 16/24/32-bit float | Apple audio format |
| **FLAC** | `.flac` | ✓ | ✓ | 1–8 | 16/24-bit | Lossless compressed audio archive |
| **Apple Lossless (ALAC)** | `.m4a` | ✓ | ✓ | 1–8 | 16/24-bit | Apple lossless compression |
| **Opus (RFC 6716)** | `.opus` | ✓ | ✓ | 1–8 | 48 kHz float | Ultra-low latency voice and stream audio |
| **Ogg Vorbis** | `.ogg` | ✓ | ✓ | 1–8 | 16/24-bit equiv | Lossy compressed web/game audio |
| **MP3 (MPEG-1 Layer 3)** | `.mp3` | ✓ | ✓ | Stereo | 16-bit | Legacy lossy distribution |
| **AAC-LC** | `.m4a`, `.aac` | ✓ | ✓ | 1–8 | 16/24-bit | High-efficiency modern streaming audio |

---

### 2. Broadcast Wave & Metadata Chunks (BWF, RF64, iXML)

Standard consumer WAV files (RIFF WAVE) are limited to 4 GB by their 32-bit chunk size headers and lack standardized production timecodes. Professional DAW workflows mandate:

#### 2.1 BWF `bext` Chunk (EBU Tech 3285)

Contains origin information and the SMPTE sample timestamp:

* `Description`: ASCII description of audio content (256 bytes).
* `Originator`: Sound recorder / DAW identity (32 bytes).
* `OriginatorReference`: Unique take identifier (32 bytes).
* `OriginationDate` / `OriginationTime`: Timestamp of recording.
* `TimeReference`: 64-bit integer specifying the exact sample count since midnight (SMPTE alignment).
* `CodingHistory`: Non-destructive record of sample rate, bit depth, and conversion stages.

#### 2.2 RF64 (EBU Tech 3306)

Overcomes the 4 GB limit by specifying an initial `RF64` chunk with 64-bit size tables (`ds64`), allowing multi-terabyte uncompressed 32-bit float multi-track sessions.

#### 2.3 iXML Production Metadata Chunk

An XML document embedded in a RIFF chunk containing:

```xml
<BWFXML>
  <PROJECT>Feature Film</PROJECT>
  <SCENE>42A</SCENE>
  <TAKE>03</TAKE>
  <TAPE>Roll07</TAPE>
  <SPEED>
    <TIMECODE_RATE>24/1</TIMECODE_RATE>
    <FILE_SAMPLE_RATE>48000</FILE_SAMPLE_RATE>
  </SPEED>
  <TRACK_LIST>
    <TRACK><CHANNEL_INDEX>1</CHANNEL_INDEX><NAME>Boom</NAME></TRACK>
    <TRACK><CHANNEL_INDEX>2</CHANNEL_INDEX><NAME>Lav_Protagonist</NAME></TRACK>
  </TRACK_LIST>
</BWFXML>
```

#### 2.4 ADM BWF (Audio Definition Model, ITU-R BS.2076)

Standardized XML structure embedded in BWF files describing immersive 3D object-based audio (Dolby Atmos, MPEG-H). Represents channel-bed speakers (e.g. 7.1.4) alongside dynamic sound objects moving in continuous 3D Cartesian coordinates $(x, y, z)$ over time.

---

### 3. Session & Timeline Interchange Formats

| Format | Extension | Audio Media | Edits & Fades | Automation | Track Hierarchy |
| :--- | :--- | :---: | :---: | :---: | :---: |
| **AAF (Advanced Authoring Format)** | `.aaf` | Embedded or linked | ✓ | Volume/Pan | ✓ |
| **OMF 2.0 (Open Media Framework)** | `.omf` | Embedded or linked | ✓ | Limited | Flat tracks |
| **Standard MIDI File (SMF)** | `.mid`, `.midi` | Note events only | N/A | CC Automation | Tracks / Channels |
| **MusicXML** | `.musicxml`, `.xml` | Notation only | N/A | Dynamic markings | Parts / Staves |
| **DAW Project Interchange** | `.ptx`, `.als`, `.rpp` | External links | ✓ | Full automation | Full routing |

---

### 4. Real-Time Audio Engine & DSP Invariants

1. **Zero-Allocation Audio Thread**: In steady state, audio callback threads (`process()`) must **never** allocate or deallocate heap memory, invoke OS syscalls, perform file/socket I/O, or block on locking primitives.
2. **Signed 64-Bit Media Time (`craft-time`)**: Audio time is tracked as signed integer `Tick` units, permitting negative lead-in preroll and precise alignment.
3. **ITU-R BS.1770-4 & EBU R128 Conformance**: Integrated loudness (LUFS), loudness range (LRA), and true-peak (dBTP) oversampled 4x with non-ringing polyphase filters.

```typescript
interface AudioFormatCapability {
  format: string;
  extensions: string[];
  capabilities: {
    multichannel: boolean;
    floatingPoint: boolean;
    bextTimecode: boolean;
    ixmlMetadata: boolean;
    spatialAdm: boolean;
    losslessCompression: boolean;
  };
  sampleRates: number[];
  maxChannels: number;
}
```

---

## Part VI: Non-Linear Video Editing & Finishing Reference (FilmCraft Domain)

FilmCraft coordinates multi-track video timelines, intra-frame intermediate mastering, frame-rate conforming, and multi-format delivery.

---

### 1. Video Codecs & Intermediate Mastering

| Codec Family | Specification | Bit Depth | Chroma Sampling | Compression Type | Primary Use Case |
| :--- | :--- | :---: | :---: | :--- | :--- |
| **Apple ProRes 422 Proxy / LT** | Apple ProRes | 10-bit | 4:2:2 | Intra-frame DCT | Offline editorial proxy |
| **Apple ProRes 422 / 422 HQ** | Apple ProRes | 10-bit | 4:2:2 | Intra-frame DCT | Master delivery & editing |
| **Apple ProRes 4444 / 4444 XQ** | Apple ProRes | 10/12-bit | 4:4:4:4 | Intra-frame DCT + Alpha | High-end grading & VFX |
| **Avid DNxHD (SMPTE VC-3)** | SMPTE 2014-1 | 8/10-bit | 4:2:2 | Intra-frame DCT | Broadcast HD mastering |
| **Avid DNxHR (LB/SQ/HQ/HQX/444)** | Avid DNxHR | 8/10/12-bit | 4:2:2, 4:4:4 | Intra-frame DCT | 4K/8K resolution mastering |
| **H.264 / AVC** | ISO/IEC 14496-10 | 8/10-bit | 4:2:0, 4:2:2 | Inter-frame Long-GOP | Web streaming & proxy |
| **H.265 / HEVC** | ISO/IEC 23008-2 | 8/10/12-bit | 4:2:0, 4:2:2 | Inter-frame Long-GOP | 4K HDR & master distribution |
| **AV1** | AOMedia AV1 | 8/10/12-bit | 4:2:0, 4:2:2, 4:4:4 | Inter-frame Long-GOP | Next-generation royalty-free |

---

### 2. Camera RAW Video Formats

| Camera Format | Extension | Internal Essence | Demosaic Location | Metadata Control |
| :--- | :--- | :--- | :--- | :--- |
| **CinemaDNG** | `.dng` (Folder) | Uncompressed or lossless JPEG | Host NLE | ISO, White Balance, Tint |
| **REDCODE RAW (R3D)** | `.r3d` | Wavelet-compressed Bayer mosaic | Host / GPU | ISO, Color Science, FLUT |
| **Blackmagic RAW** | `.braw` | Partially demosaiced DCT sensor data | GPU SDK | ISO, Highlight Recovery |
| **Apple ProRes RAW** | `.mov` | Bayer sensor pattern in ProRes container | Host / GPU | White balance, exposure offset |
| **Sony XAVC** | `.mxf`, `.mp4` | Intra / Long-GOP H.264/H.265 | Hardware decoder | S-Log3, SGamut3.Cine |

---

### 3. Timeline Conforming & Editorial Interchange

#### 3.1 OpenTimelineIO (OTIO)

Modern standard schema modeling editorial timelines:

* `Timeline` $\to$ `Stack` $\to$ `Track` (Video/Audio/Marker) $\to$ `Clip`, `Gap`, `Transition`.
* Preserves linear time warps, clip media references (`ExternalReference`), active time ranges, and vendor-neutral markers.

#### 3.2 EDL (CMX 3600)

Line-oriented text format specifying cuts, dissolves, and source tape timecodes:

```text
001  TAPE_01  V     C        01:00:10:00 01:00:15:00 00:00:00:00 00:00:05:00
* FROM CLIP NAME: INTERVIEW_A.MOV
002  TAPE_02  V     D    030 02:14:00:00 02:14:04:00 00:00:05:00 00:00:09:00
```

#### 3.3 FCPXML Interchange Specification

XML timeline interchange supporting multi-camera clips, compound clips, audio component assignment, and precision retiming.

---

### 4. SMPTE Timecode & Captions

* **SMPTE 12M Timecode**: Non-Drop-Frame (NDF, `HH:MM:SS:FF`) vs Drop-Frame (DF, `HH:MM:SS;FF`). For 29.97 fps television broadcast, frame numbers 00 and 01 are dropped at the start of every minute except every tenth minute to eliminate a 108-frame/hour sync drift.
* **Captions**: CEA-608 (line 21 analog captions), CEA-708 (DTV digital caption stream embedded in H.264 SEI NALUs), SubRip (`.srt`), WebVTT (`.vtt`), IMSC / TTML.

---

## Part VII: VFX, Motion Graphics & Compositing Reference (EffectCraft Domain)

EffectCraft operates on multi-channel high-dynamic-range floating-point buffers, volumetric deep images, ACES color transformations, and 3D scene caches.

---

### 1. High-Dynamic-Range & Deep Raster Formats

| Format | Extension | Dynamic Range | Precision | Deep Samples | Multi-Part |
| :--- | :--- | :--- | :--- | :---: | :---: |
| **Multi-Part Deep OpenEXR** | `.exr` | 30+ stops | 16-bit half, 32-bit float | ✓ | ✓ |
| **DPX (SMPTE 268M)** | `.dpx` | Logarithmic / Linear | 10-bit log, 12/16-bit linear | - | - |
| **Cineon** | `.cin` | Logarithmic film | 10-bit log | - | - |
| **Radiance HDR** | `.hdr` | HDR | 32-bit RGBE shared exponent | - | - |

#### 1.1 Deep Compositing in OpenEXR

Conventional 2D compositing stores a single color and alpha per pixel ($R, G, B, A$). Deep OpenEXR stores an arbitrary number of samples per pixel, each with its own depth coordinate ($z$) and color:

$$\text{Pixel}(x, y) = \left\{ (z_0, \mathbf{c}_0, \alpha_0), (z_1, \mathbf{c}_1, \alpha_1), \dots, (z_{n-1}, \mathbf{c}_{n-1}, \alpha_{n-1}) \right\}$$

This allows seamless interpenetration of volumetric smoke, particle clouds, and motion-blurred hair without generating color fringing or edge artifacts.

---

### 2. ACES & Color Management (OpenColorIO)

EffectCraft enforces linear-light compositing under the Academy Color Encoding System (ACES):

* **ACES2065-1 (AP0)**: Ultra-wide color space enclosing the entire visible spectrum; intended for archival interchange.
* **ACEScg (AP1)**: Optimized working color gamut for computer graphics rendering and compositing with linear transfer characteristics.
* **ACEScc / ACEScct**: Logarithmic encodings of AP1 designed for color grading controls.
* **3D LUTs**: `.cube` (standard 3D LUT format containing 1D shaper curves and 3D tetrahedral color cube grids up to $65  imes 65  imes 65$), `.3dl` (standard 3D LUT format).

---

### 3. 3D Scene Interchange & Motion Tracking

* **Universal Scene Description (USD / USDZ)**: Composable scene graph format containing geometry meshes, materials, skeletal skinning, and lighting rigs.
* **Alembic (`.abc`)**: High-performance baked point cache and transform matrix interchange for transferring simulated fluid, cloth, and camera tracking data from 3D DCCs to compositing viewports.

---

## Part VIII: Non-Destructive RAW Grading Reference (LightCraft Domain)

LightCraft provides professional non-destructive RAW developing, tethered camera capture, asset cataloging, and color science grading.

---

### 1. Catalog Architecture & Non-Destructive Separation

In LightCraft, the original image essence is **never modified**. Developing is strictly a directed acyclic graph (DAG) of parametric metadata operations stored in SQLite databases and XMP sidecars:

```text
LightCraft Photo Pipeline
├── RAW Essence (Read-Only: .cr3, .nef, .arw, .dng)
│
├── Non-Destructive Metadata (Read-Write: .xmp / .lrcat)
│   ├── Crop & Perspective Matrix (Keystone, Angle)
│   ├── Demosaic Parameters & Noise Reduction Radius
│   ├── Camera Profile Matrix (DCP / ICC)
│   ├── Parametric Exposure, Contrast, Highlights, Shadows
│   ├── Parametric Tone Curves (RGB & Master)
│   ├── HSL / Color Grading Wheels (Lift, Gamma, Gain)
│   └── Vector Mask Stacks (Linear, Radial, AI Subject Masks)
│
└── Rendered Output (On-Demand: Display Buffer, TIFF, JPEG)
```

#### 1.1 SQLite-Based Photo Catalog (`.lrcat`)

An SQLite 3 relational database containing:

* `AgHarvestedExifMetadata`: Extracted camera capture parameters.
* `AgLibraryFile` / `AgLibraryFolder`: Filesystem asset index and volume UUIDs.
* `Adobe_imageDevelopSettings`: Serialized parametric adjustment history and snapshot states.

#### 1.2 DNG Camera Profiles (DCP)

Defines camera sensor color transformation through dual illuminant matrices (Standard Illuminant A 2856K and D65 6504K) and multi-dimensional HueSatMap lookup tables.

---

## Part IX: Precision Parametric CAD & BIM Reference (CADCraft Domain)

CADCraft operates on exact topological boundary representation (B-Rep) solid geometry, parametric constraint solvers, and building information models.

---

### 1. Neutral CAD Interchange Formats

| Format | Specification | Modeling Type | Geometric Capabilities | Primary Domain |
| :--- | :--- | :--- | :--- | :--- |
| **STEP AP203** | ISO 10303-203 | B-Rep Solid / Surface | Exact NURBS, solids, assemblies | Aerospace & mechanical |
| **STEP AP214** | ISO 10303-214 | B-Rep Solid | Colors, layers, GD&T, assemblies | Automotive manufacturing |
| **STEP AP242** | ISO 10303-242 | B-Rep Solid + PMI | Semantic 3D annotations, tessellation | Modern digital manufacturing |
| **IGES** | ANSI Y14.26M | Surface / Wireframe | NURBS surfaces, curves, wireframes | Legacy CAD interchange |
| **Parasolid** | `.x_t`, `.x_b` | Exact B-Rep Solid | Advanced freeform blending, solids | Mechanical engineering kernel |
| **ACIS SAT** | `.sat`, `.sab` | Exact B-Rep Solid | Exact manifold geometry | Spatial ACIS kernel exchange |
| **IFC (BIM)** | ISO 16739-1 | Architectural Objects | Parametric walls, slabs, MEP systems | Building information modeling |
| **3MF** | 3MF Consortium | Triangle Mesh | Color, materials, slices, lattices | Additive manufacturing / 3D print |

---

### 2. B-Rep Topology vs Polygonal Mesh

A critical architectural distinction in CADCraft is that exact engineering models are **not** polygonal meshes:

```text
B-Rep Solid Model (Exact Mathematical Boundary)
└── Solid Body
    └── Shell
        └── Faces (Bounded by parametric surfaces: Plane, Cylinder, Cone, NURBS)
            └── Loops
                └── Edges (Bounded by 3D curves: Line, Circle, Ellipse, Spline)
                    └── Vertices (Exact 3D Coordinates x, y, z)
```

Exporting CAD B-Rep to mesh formats (STL, OBJ, 3MF) is a **one-way destructive tessellation** where smooth circular cylinders become faceted polygonal approximations.

---

## Part X: Collaborative UI/UX Prototyping Reference (FigCraft Domain)

FigCraft operates on real-time collaborative vector interfaces, flexbox auto-layout constraint trees, reusable component variants, and interactive state transitions.

---

### 1. Format Inventory & Document Packages

| Format | Extension | Container Structure | Data Model | Interoperability Level |
| :--- | :--- | :--- | :--- | :--- |
| **Kiwi Schema Document** | `.fig` | Proprietary binary container | Kiwi binary schema (Document Tree) | Binary decode required |
| **Open JSON UI Schema Document** | `.sketch` | ZIP archive of JSON files | `document.json`, `pages/*.json` | Open documented JSON schema |
| **Vector UI Package** | `.xd` | ZIP archive of JSON & assets | Scene graph manifests | Documented ZIP structure |
| **Penpot** | `.penpot` | ZIP archive of JSON | Open web standard SVG/JSON | Fully open source |

---

### 2. Prototyping Domain Invariants

1. **Vector Networks**: Unlike standard Bézier paths where every anchor point has at most one incoming and one outgoing line, FigCraft vector networks allow arbitrary graph topology where a single node can branch into three or more paths.
2. **Auto-Layout Constraints**: Hierarchical layout containers supporting primary/counter axis alignment, padding, gap, and sizing policies (`Fixed`, `Hug contents`, `Fill container`).
3. **Component Architecture**: Master components with localized instance property overrides (text strings, colors, nested swap variants).

---

## Part XI: Structured Office Document Suite Reference (WordCraft, DeckCraft, GridCraft Domain)

The office productivity trio operates on structured rich-text flows, slide presentation trees, and multi-threaded calculation grids.

---

### 1. Document Standards & Package Schemas

| Application | OpenXML (ECMA-376 / ISO 29500) | OpenDocument (OASIS / ISO 26300) | Primary Internal Essence |
| :--- | :--- | :--- | :--- |
| **WordCraft** | `.docx` (`word/document.xml`) | `.odt` (`content.xml`) | Flowed paragraphs, runs, footnotes, sections |
| **DeckCraft** | `.pptx` (`ppt/slides/slide*.xml`) | `.odp` (`content.xml`) | Master slides, shapes, animations, canvas layout |
| **GridCraft** | `.xlsx` (`xl/worksheets/sheet*.xml`) | `.ods` (`content.xml`) | Cell matrix, formula dependency tree, formatting |

---

### 2. GridCraft Calculation Engine & Formula Trees

Spreadsheet documents are computational directed acyclic graphs (DAGs):

* **Calculation Chains (`calcChain.xml`)**: Execution order of formulas to ensure precedent cells resolve before dependent cells.
* **Shared Strings (`sharedStrings.xml`)**: Unique string deduplication pool for multi-million-row spreadsheets.
* **IEEE 754 Floating-Point Precision**: Guarding against binary floating-point representation artifacts (e.g. $0.1 + 0.2 \neq 0.3$) via decimal arithmetic and round-off policies.

---

## Part XII: Unified Multi-Application Architecture & Implementation Plan

### 1. Complete 13-Application Cross-Domain Comparative Analysis

| Craft Application | Domain Reference & Format Specification | Primary Document Model | Native Format | Clean-Room Open Exchange | Critical Preservation Invariant |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **PhotoCraft** | Layered Raster & Composite Pipeline | Layered Raster & Masks | `.psd` / `.psb` | Layered TIFF, OpenEXR | Channel depth, blend modes, layer masks |
| **VectorCraft** | Precision Vector Illustration Engine | Vector Scene Graph | `.ai` / `.ait` | `.svg`, PDF | Cubic Bézier paths, appearance stacks |
| **DesignCraft** | Multipage Publishing & Layout Engine | Multipage Publication Tree | `.indd` / `.indt` | `.idml` | Text flow, paragraph styles, pagination stability |
| **PdfCraft** | Structural PDF Inspection & Preflight | ISO 32000 Document Graph | `.pdf` | Pure PDF / PDF/X | Direct COS object trees, xref tables, font programs |
| **SoundCraft** | Digital Audio Workstation & Spatial DSP | Audio Timeline & DSP Graph | `.craft-sound` | BWF, RF64, AAF | Sample accuracy, timecode sync, real-time safety |
| **FilmCraft** | Nonlinear Timeline Video Editor | NLE Editorial Sequence | `.craft-film` | OpenTimelineIO, FCPXML, EDL | Non-destructive conforms, frame-rate retiming |
| **EffectCraft** | Motion Graphics & Compositing Graph | Compositing Node Graph | `.craft-fx` | Multi-part Deep EXR, USD | Floating-point color, z-depth, 3D caches |
| **LightCraft** | Non-Destructive RAW Image Grading | Parametric RAW Grading | `.craft-raw` | DNG Linear RAW, XMP | Non-destructive RAW preservation, camera profiles |
| **CADCraft** | Precision Parametric CAD & BIM | Parametric B-Rep Topology | `.craft-cad` | STEP AP242, IFC | Exact surface math, constraint graphs, tolerances |
| **FigCraft** | Collaborative UI/UX Prototyping | UI Layout & Prototypes | `.craft-fig` | Open vector UI schema, Vector Network | Flexbox auto-layout, component variants, states |
| **WordCraft** | Structured Document Authoring | Flowed WordprocessingML | `.docx` | `.odt`, Markdown | Section flows, footnote trees, style cascading |
| **DeckCraft** | Slide Presentations & Vector Graphics | Slide Presentation Tree | `.pptx` | `.odp`, PDF | Slide layout masters, vector shapes, transitions |
| **GridCraft** | High-Performance Spreadsheet Engine | High-Performance Formula Grid | `.xlsx` | `.ods`, CSV | Formula dependency DAGs, number formatting |

---

### 2. The Four-Tier Claim Taxonomy (`AGENTS.md`)

When evaluating or claiming file format capabilities within ArtCraft and `craft-libs`, every engine component must enforce:

$$\text{Documented} \neq \text{Implemented} \neq \text{Tested} \neq \text{Empirically Validated}$$

1. **Documented**: The format capability or container tag appears in specifications or vendor documentation.
2. **Implemented**: Parsing and writing logic exists in code without throwing unhandled errors.
3. **Tested**: Automated unit or synthetic proptests exercise the code path with passing status.
4. **Empirically Validated**: Real-world round-trip testing passes byte-level or rendering regression verification against the authentic desktop applications.

---

### 3. Consolidated Implementation Priorities for `craft-libs` & `mediacraft`

| Priority | Domain | Target Format / Engine | Target Crate | Key Milestone |
| :--- | :--- | :--- | :--- | :--- |
| **P0** | Raster | PSD / PSB Parsing & Diagnostics | `craft-psd` | Header validation, resource extraction, layer channel decoding, diagnostic reports |
| **P0** | Publishing | PDF Parsing & COS Object Graph | `craft-pdf` | ISO 32000 parser, xref rebuild, stream inflation, cyclic reference defense |
| **P0** | Publishing | ZIP / Package Safety | `craft-archive` | Zip Slip prevention, CRC32 verification, IDML/EPUB package reading |
| **P0** | Media | ISOBMFF Container Inspection | `craft-isobmff` | Faststart moov detection, 64-bit atom overflow safety, timescale diagnostics |
| **P0** | Media | Matroska / EBML Parsing | `craft-matroska` | VINT length validation, cluster element indexing, track report generation |
| **P1** | Audio | Broadcast WAV & Timecode | `craft-bwf` (Staged) | `bext` sample count timecode, `iXML` production metadata, RF64 64-bit sizes |
| **P1** | Video | Timeline Editorial Conforming | `craft-otio` (Staged) | OpenTimelineIO schema parser, clip time range mapping, gap resolution |
| **P1** | Publishing | IDML Document Interchange | `craft-idml` (Staged) | `designmap.xml`, `Stories/`, and `Styles.xml` reading and serialization |
| **P1** | Vector | SVG 1.1 / 2.0 Geometry Extraction | `craft-svg` (Staged) | Path command tokenizer, cubic Bézier conversion, viewport coordinate transform |
| **P1** | Temporal | XFL Uncompressed Project Reader | `craft-xfl` (Staged) | `DOMDocument.xml` and `DOMTimeline` frame span inspection |
| **P2** | Media | Stream-Copy Remuxing Facade | `craft-media` | Monotonic timeline reclocking, packet buffer management, fallback policies |
| **P2** | CAD | STEP B-Rep Reader | `craft-step` (Staged) | ISO 10303-21 entity parser, Cartesian point evaluation, topological graph |
| **P2** | UI Design | Vector Network Engine | `craft-vecnet` (Staged) | Non-manifold graph paths, multi-edge nodes, topology validation |
| **P2** | Office | Spreadsheet Formula Engine | `craft-calc` (Staged) | Dependency graph topologically sorted calculation chain, AST evaluator |
