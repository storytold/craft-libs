# craft-ora

OpenRaster documents (`.ora`), read and written. Written from the public specification
(<https://www.openraster.org/>, `stack.xml` 0.0.x).

An OpenRaster document is a ZIP whose first, stored entry is `mimetype` = `image/openraster`. It
holds `stack.xml` (the layer tree), one PNG per layer, `mergedimage.png` (the whole image) and
`Thumbnails/thumbnail.png`.

- `is_ora`, `zip_mimetype`: recognise an archive by its first entry, without unzipping.
- `Archive::open`: checks the mimetype and reads `stack.xml` into an `Image`: `Node::Layer`,
  `Node::Stack` and `Node::Other`, topmost first, with the attributes the file has (`name`,
  `visibility`, `opacity`, `composite-op`, `edit-locked`, `alpha-preserve`, `x`, `y`, `src`,
  `isolation`). `Archive::entry` reads a layer image or any other entry under a size limit.
- `Image::explicit_group_isolation`: whether the file's version (0.0.6 and later) makes
  `isolation` explicit, or leaves pass-through to be inferred as older files do.
- `Image::to_xml` and `Writer`: write `stack.xml` and the archive, mimetype first, entries
  stored.

Layer images are PNG; decoding and encoding them is the caller's, as is what a `composite-op`
means. `SVG_COMPOSITE_OPS` lists the names the specification defines.

## Limits

| Limit | Value |
| --- | --- |
| Canvas side | 300,000 px (`MAX_SIDE`); layer offsets up to four times that |
| Canvas area | 1 GiB at 8 bytes per pixel (`MAX_PIXEL_BYTES`) |
| Elements under the root stack | 8192 (`MAX_NODES`) |
| `stack.xml` | 16 MiB, 512 levels of nesting, no document type declaration |
| An entry | the caller's limit; `MAX_ENTRY` (512 MiB) is the suggested one for a layer image |

## ZIP

The archive is read and written with the `zip` crate directly. `zip_mimetype` (the leading stored
`mimetype` entry), the bounded entry read and the mimetype-first writer are the same in every
mimetype-first ZIP format and belong in a shared ZIP crate once there is one.
