# JPEG 2000 interoperability fixtures

Small JPEG 2000 files from an independent encoder, OpenJPEG's `opj_compress` 2.5.4 (Ubuntu package
`libopenjp2-tools` 2.5.4-1ubuntu0.1), for `tests/it/jp2_interop.rs`. They are our own test data:
16×16 synthetic ramps that `make_fixtures.py` writes, with no third-party image content.

Regenerate them all, byte for byte, with `OPJ_COMPRESS=/path/to/opj_compress python3 make_fixtures.py`
(Python 3, standard library only). Channel `c` of pixel `i` (row-major) holds
`(i·2^(d−8) + (i·37 mod 2^(d−8)) + c·2^(d−2)) mod 2^d` at `d` bits, so each channel sits a quarter of
the range from the next and the bits below the top 8 vary. Every `opj_compress` call adds `-n 3`
(three resolution levels; a 16×16 image allows at most five):

| File | Source given to `opj_compress` | Extra options | Then |
| --- | --- | --- | --- |
| `gray-8.jp2`, `gray-16.jp2` | PGM, maxval 255 or 65535 | | |
| `rgb-8.jp2`, `rgb-16.jp2` | PPM, maxval 255 or 65535 | (reversible colour transform by default) | |
| `graya-8.jp2`, `graya-16.jp2`, `rgba-8.jp2`, `rgba-16.jp2` | PNG with alpha, 8 or 16 bits | | |
| `gray-12.jp2`, `rgb-12.jp2` | planar big-endian raw | `-F 16,16,1,12,u` or `-F 16,16,3,12,u` | |
| `gray-12.j2k` | the `gray-12` raw | `-F 16,16,1,12,u`, raw codestream output | |
| `rgba-16.j2k` | the `rgba-16` PNG | raw codestream output | |
| `rgba-16-no-cdef.jp2` | planar raw, 4 components | `-F 16,16,4,16,u` (no channel definitions) | |
| `rgb-16-no-mct.jp2` | the `rgb-16` PPM | `-mct 0` | |
| `rgb-16-ict.jp2` | the `rgb-16` PPM | `-I` (irreversible 9/7 wavelet and colour transform) | |
| `rgb-16-icc.jp2` | the `rgb-16` PPM | | `colr` replaced by method 2 with a 132-byte RGB profile header and no tags |
| `rgba-16-swapped-cdef.jp2` | the `rgba-16` PNG | | `cdef` replaced: component 0 blue, 2 red, 3 opacity |
| `rgba-16-alpha-first.jp2` | the `rgba-16` PNG | | `cdef` replaced: component 0 opacity, 1 to 3 colours |
| `rgb-16-sycc.jp2` | the `rgb-16` PPM | `-mct 0` | `colr` replaced by enumerated sYCC (18) |
| `gray-16-signed.jp2` | planar raw, signed | `-F 16,16,1,16,s` | |
| `rgb-16-420.jp2` | planar raw, chroma at half size | `-F 16,16,3,16,u@1x1:2x2:2x2 -mct 0` | `colr` replaced by enumerated sRGB (16; `opj_compress` marks it sYCC) |
| `palette-10.jp2` | planar raw, 10-bit indices | `-F 16,16,1,10,u` | `colr` set to sRGB, `pclr` (1024 entries, three 16-bit columns, entry `i` = the 16-bit ramp at pixel `i mod 256`) and `cmap` added |

The "then" edits rewrite boxes inside the JP2 header box (`jp2h`) only; the codestream is OpenJPEG's.
