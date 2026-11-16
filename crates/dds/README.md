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
| Uncompressed | 8, 16, 24 or 32 bits per pixel described by channel masks, RGB or luminance, with or without alpha; padded rows (`DDSD_PITCH`) | BGRA, BGRX, luminance, luminance with alpha |
| Block-compressed | BC1, BC2, BC3 (DXT1, DXT3, DXT5); unsigned BC4 (ATI1, BC4U, DXGI 79 and 80) and BC5 (ATI2, BC5U, DXGI 82 and 83) | BC1, BC2, BC3, BC4, BC5 |
| Headers | legacy and DX10 | legacy; DX10 for BC4 and BC5 |
| Mip levels | the top level only | optional full chain, box-filtered |

Cubemaps, texture arrays, volume textures, premultiplied-alpha textures (DXT2/DXT4, or DX10 alpha mode 2), signed (SNORM) BC4/BC5 and
the other DXGI formats (BC6H, BC7, float) are refused with `DdsError::Unsupported` rather than read
in part.

Pixels are 8 bits per channel, interleaved and top row first, in one of four `Layout`s (`Gray`,
`GrayA`, `Rgb`, `Rgba`). BC1 to BC3 are written from RGBA, BC4 from gray and BC5 from RGB
(`DdsCompression::layout` names it). BC4 reads back as gray. BC5 stores red and green only, so it
reads back as RGB with blue 0: for a tangent-space normal map, X and Y, with Z left to the
renderer. ATI2 files are read as Microsoft's tools read them (red in the first block), not with
the two blocks swapped as FFmpeg does.

```rust
use craft_dds::{DdsCompression, DdsOptions, Layout, Texture};

let pixels = [10, 20, 30, 255].repeat(6 * 5);
let file = craft_dds::encode(6, 5, Layout::Rgba, &pixels, DdsOptions { compression: DdsCompression::Bc1, mipmaps: true })?;
// The header first: the size and layout are known before anything is allocated.
let texture = Texture::parse(&file)?;
assert_eq!((texture.width(), texture.height(), texture.layout()), (6, 5, Layout::Rgba));
let decoded = texture.decode()?;
```

The block encoder is our own: RGB565 endpoints fitted along each block's principal axis and
refined by least squares, indices picked against the palette the reader decodes; BC1 turns texels
under half opacity into its transparent black, and BC3, BC4 and BC5 pick whichever mode of their
value blocks fits. The block functions don't depend on the DDS header, so other containers of
the same blocks can share them.

## Features

- `parallel`: encodes and decodes rows of blocks (and mip levels) on several threads with rayon.
  `craft-image-codecs` turns it on for native targets.
