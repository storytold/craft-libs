# craft-kra

Krita documents (`.kra`, `.krz`), read. Written from files Krita writes; no Krita code is used.

A Krita document is a ZIP whose first, stored entry is `mimetype` = `application/x-krita`. It
holds `maindoc.xml` (the node tree), one tiled pixel file per paint layer and mask under
`<image name>/layers/`, the ICC profile in `<image name>/annotations/icc`, and Krita's rendering
of the whole image in `mergedimage.png`.

- `is_kra`, `zip_mimetype`: recognise an archive by its first entry, without unzipping.
- `Archive::open`: checks the mimetype and reads `maindoc.xml` into an `Image` of `Node`s. Every
  node kind is kept with the attributes the file has (`nodetype`, `compositeop`, `opacity`,
  offsets, `colorspacename`, `filename`, masks, ...). No pixel is read.
- `Archive::layer_file`, `default_pixel`, `icc_profile`, `merged_image`: the other entries, each
  with a size limit.
- `tiles::TileFile`: a tiled pixel file's header and tile table; `decode` gives one tile's
  interleaved pixels. `tiles::lzf_decompress` is the LZF inside.
- `tiles::Space`: the pixel layouts read (`RGBA`, `RGBA16`, `GRAYA`, `GRAYAU16`, and one-byte
  masks), with `rgba` and `samples` to turn Krita's B, G, R, A little-endian pixels into channel
  order.

What a node kind, a composite operation or a colour space means to an application is not decided
here. There is no writer.

## Limits

| Limit | Value |
| --- | --- |
| Canvas side | 300,000 px (`MAX_SIDE`); offsets and tile positions up to four times that |
| Canvas area | 1 GiB at 8 bytes per pixel (`MAX_PIXEL_BYTES`) |
| `<layer>` nodes | 8192 (`MAX_NODES`) |
| `maindoc.xml` | 32 MiB, 512 levels of nesting, no internal DTD subset |
| A pixel file, inflated | 1 GiB (`MAX_LAYER_FILE`) |
| ICC profile | 16 MiB |
| Tile | 1024 × 1024 px, 64 bytes per pixel, header lines of 256 bytes |

A pixel file is decoded one tile at a time into a buffer the caller owns, so the memory a
document takes is the caller's to budget.

## ZIP

The archive is read with the `zip` crate directly. `zip_mimetype` (the leading stored `mimetype`
entry) and the bounded entry read are the same in every mimetype-first ZIP format and belong in a
shared ZIP crate once there is one.
