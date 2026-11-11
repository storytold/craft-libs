# craft-xcf

GIMP XCF images (`.xcf`, `.xcf.gz`), read. Written from the public specification
(<https://developer.gimp.org/core/standards/xcf/>, versions 0 to 26) and from files GIMP 3.2
writes; no GIMP code is used.

- `is_xcf`, `is_gzipped_xcf`, `gunzip`: recognise a file and inflate a compressed one.
- `Xcf::parse`: the header, the image properties and every layer structure, as an `Image`:
  version, canvas, base type, `Precision`, `Compression`, colormap, resolution, guides, the ICC
  profile and comment parasites, the path count, and each `Layer` with its properties as GIMP
  wrote them (mode, opacity, offsets, locks, colour tag, composite mode and spaces, group flag,
  item path, text / vector / link flags, effect count). No pixel is decoded.
- `Xcf::layer_pixels`, `Xcf::layer_mask`, `Xcf::channel`: one drawable's pixels as normalised
  `f32` samples (`Pixels`), from uncompressed, RLE or zlib tiles at every precision (8, 16 and
  32-bit integer; 16, 32 and 64-bit float). Indexed pixels are expanded through the colormap.
  Each call takes its samples from a byte budget the caller owns, and a `keep_going` callback.
- `Image::tree`: the layers arranged by their item paths, with the ones whose path leads nowhere
  or nests too deep reported.

The reader repairs what GIMP writes wrongly: the length words of `PROP_COLORMAP`,
`PROP_USER_UNIT` and `PROP_LINK_LAYER` are measured, not trusted.

Layer modes, colour tags and composite settings stay numbers: what they mean to an application
is not decided here. Paths, layer effects and text or vector layer data are not read (their
presence is reported). There is no writer.

## Limits

| Limit | Value |
| --- | --- |
| Canvas or drawable side | 300,000 px (`MAX_SIDE`) |
| Canvas area | 1 GiB at 16 bytes per pixel (`MAX_PIXEL_BYTES`) |
| Decoded pixels | the caller's budget, charged 4 bytes a sample before allocation |
| Layers | 8192 (`MAX_LAYERS`); channels 1024 (`MAX_CHANNELS`) |
| Layer effects on one layer | 1024 |
| Properties in one list | 4096; a payload 64 MiB (`MAX_PAYLOAD`) |
| A string | 1 MiB (`MAX_STRING`) |
| Copied layer and channel names and item paths | 32 MiB in all (`MAX_METADATA_BYTES`) |
| Group nesting in `Image::tree` | 64 (`MAX_DEPTH`) |
| A gzipped file, inflated | 1 GiB (`MAX_INFLATED`) |
