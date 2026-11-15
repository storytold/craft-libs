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
| Block-compressed | BC1, BC2, BC3 (DXT1, DXT3, DXT5) | BC1, BC2, BC3 |
| Headers | legacy and DX10 | legacy |
| Mip levels | the top level only | optional full chain, box-filtered |

Cubemaps, texture arrays, volume textures, premultiplied DXT2/DXT4 and the other DXGI formats
are refused with `DdsError::Unsupported` rather than read in part.

Pixels are 8 bits per channel, interleaved and top row first, in one of four `Layout`s (`Gray`,
`GrayA`, `Rgb`, `Rgba`). Block compressions are written from RGBA.

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
under half opacity into its transparent black, and BC3 picks whichever alpha mode fits.

## Features

- `parallel`: encodes and decodes rows of blocks (and mip levels) on several threads with rayon.
  `craft-image-codecs` turns it on for native targets.
