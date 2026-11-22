# craft-dds

DirectDraw Surface (DDS) textures, after Microsoft's public DDS and block-compression
documentation, with no dependencies. `craft-image-codecs` reads and writes DDS through this
crate and re-exports its `DdsCompression` and `DdsOptions`; an app that only needs DDS (a texture
exporter, say) can depend on it alone and leave out the other codecs and their licences.

```toml
craft-dds = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

| | Read | Write |
|---|---|---|
| Uncompressed | 8, 16, 24 or 32 bits per pixel described by channel masks, RGB or luminance, with or without alpha; padded rows (`DDSD_PITCH`); 16 bits per channel: `R16_UNORM`, `R16G16_UNORM`, `R16G16B16A16_UNORM` (DXGI 56, 35, 11 and their typeless forms; legacy FourCC 36) | BGRA, BGRX, luminance, luminance with alpha; from 16-bit pixels `R16_UNORM`, `R16G16_UNORM`, `R16G16B16A16_UNORM` |
| Block-compressed | BC1, BC2, BC3 (DXT1, DXT3, DXT5); unsigned BC4 (ATI1, BC4U, DXGI 79 and 80) and BC5 (ATI2, BC5U, DXGI 82 and 83) | BC1, BC2, BC3, BC4, BC5 |
| Headers | legacy and DX10 | legacy; DX10 for BC4, BC5 and the 16-bit formats |
| Mip levels | the top level only | optional full chain, box-filtered |

Cubemaps, texture arrays, volume textures, premultiplied-alpha textures (DXT2/DXT4, or DX10 alpha mode 2), signed (SNORM) BC4/BC5 and
the other DXGI formats (BC6H, BC7, float, signed or integer 16-bit) are refused with `DdsError::Unsupported` rather than read
in part.

Pixels are 8 bits per channel (`encode`, `Texture::decode`) or 16 bits per channel (`encode16`,
`Texture::decode16`), interleaved and top row first, in one of four `Layout`s (`Gray`,
`GrayA`, `Rgb`, `Rgba`). BC1 to BC3 are written from RGBA, BC4 from gray and BC5 from RGB
(`DdsCompression::layout` names it). BC4 reads back as gray. BC5 and `R16G16_UNORM` store red
and green only, so they read as RGB with blue 0: for a tangent-space normal map, X and Y, with Z
left to the renderer. ATI2 files are read as Microsoft's tools read them (red in the first block),
not with the two blocks swapped as FFmpeg does.

From 16-bit pixels, uncompressed textures keep 16 bits per channel: gray as `R16_UNORM`, gray
and alpha (or any two channels, such as a normal map's X and Y) as `R16G16_UNORM`, RGB (alpha
65535) and RGBA as `R16G16B16A16_UNORM`, all with the DX10 header; they read back as gray, RGB
with blue 0 (red and green holding the two channels), and RGBA. BC4 and BC5 keep 8-bit
endpoints, as the formats store them, but pick them and the indices against the 16-bit values,
so a smooth height or normal map has less error than when its 8-bit rounding is compressed. BC1 to BC3 round the pixels to 8 bits first.
`Texture::decode` rounds 16-bit formats to 8 bits; `Texture::decode16` returns them exactly,
rescales channel-mask fields from their own width (a 16-bit luminance field exactly),
interpolates BC4 and BC5 at 16 bits, and scales BC1 to BC3 by 257. `Texture::bits_per_channel`
gives the widest stored channel; `craft-image-codecs` reads a texture through `decode16` when it
is wider than 8 bits.

```rust
use craft_dds::{DdsCompression, DdsOptions, Layout, Texture};

let pixels = [10, 20, 30, 255].repeat(6 * 5);
let file = craft_dds::encode(6, 5, Layout::Rgba, &pixels, DdsOptions { compression: DdsCompression::Bc1, mipmaps: true })?;
// The header first: the size and layout are known before anything is allocated.
let texture = Texture::parse(&file)?;
assert_eq!((texture.width(), texture.height(), texture.layout()), (6, 5, Layout::Rgba));
let decoded = texture.decode()?;
```

`Texture::parse` requires a complete header and nonzero dimensions. `Texture::decode` bounds its output buffer to 1 GiB; use `decode_with_limit(max_bytes)` for an explicit per-buffer budget. Allocation is fallible. This is not a total process memory limit. The `craft-image-codecs` adapter forwards its own existing `Limits::max_alloc` instead of imposing the standalone default.

The block encoder is our own: RGB565 endpoints fitted along each block's principal axis and
refined by least squares, indices picked against the palette the reader decodes; BC1 turns texels
under half opacity into its transparent black, and BC3, BC4 and BC5 pick whichever mode of their
value blocks fits. The block functions don't depend on the DDS header, so other containers of
the same blocks can share them.

## Features

- `parallel`: encodes and decodes rows of blocks (and mip levels) on several threads with rayon.
  `craft-image-codecs` turns it on for native targets.
