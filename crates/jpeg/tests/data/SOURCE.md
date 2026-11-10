# Test images

The JPEG files come from libjpeg-turbo 3.0.0, `testimages/`
(https://github.com/libjpeg-turbo/libjpeg-turbo/tree/3.0.0/testimages), under the IJG License
(README.ijg in this directory). The other files are derived from them, or made with
libjpeg-turbo's tools, as described below.
`SHA256SUMS` lists every file.

| File | What it is |
|---|---|
| `testorig.jpg` | 227×149 YCbCr, 2×2 luma sampling, baseline Huffman (SOF0) |
| `testimgari.jpg` | the same image, arithmetic coded (SOF9), one interleaved scan |
| `testimgari-huffman.jpg` | `testimgari.jpg` losslessly transcoded to baseline Huffman (SOF0) |
| `testorig12.jpg` | 227×149 YCbCr, 12-bit extended sequential Huffman (SOF1) |
| `testorig12.ppm` | libjpeg-turbo's decode of `testorig12.jpg`: 12-bit RGB, binary PPM, maxval 4095 |
| `testorig-ari-progressive.jpg` | `testorig.jpg` transcoded to progressive arithmetic (SOF10) |
| `testorig-ari-progressive-restart.jpg` | the same with a restart interval of 3 MCU rows |
| `testorig12-ari.jpg` | `testorig12.jpg` transcoded to 12-bit arithmetic (SOF9) |
| `testorig12-ari-progressive.jpg` | `testorig12.jpg` transcoded to 12-bit progressive arithmetic (SOF10) |
| `whitetail-ari.jpg` | 512×512 gray, arithmetic coded (SOF9), noise in the top 16 rows, white below |
| `whitetail-huffman.jpg` | `whitetail-ari.jpg` losslessly transcoded to baseline Huffman (SOF0) |

`testimgari.jpg` and `testorig.jpg` share their quantization tables but not their coefficients:
libjpeg-turbo decodes them to different pixels. `testimgari-huffman.jpg` holds exactly the
coefficients of `testimgari.jpg`, so the two must decode to identical samples. The same holds
for each jpegtran transcode below and its source.

## How the derived files were made

`testimgari-huffman.jpg`, with jpegtran from libjpeg-turbo 3.1.0 (Alpine 3.22,
`libjpeg-turbo-utils-3.1.0-r0`):

```sh
jpegtran -copy none testimgari.jpg > testimgari-huffman.jpg
```

The arithmetic transcodes of `testorig.jpg` and `testorig12.jpg`, with jpegtran from
libjpeg-turbo 3.1.3 (Alpine 3.24, `libjpeg-turbo-utils`):

```sh
jpegtran -copy none -arithmetic -progressive testorig.jpg > testorig-ari-progressive.jpg
jpegtran -copy none -arithmetic -progressive -restart 3 testorig.jpg > testorig-ari-progressive-restart.jpg
jpegtran -copy none -arithmetic testorig12.jpg > testorig12-ari.jpg
jpegtran -copy none -arithmetic -progressive testorig12.jpg > testorig12-ari-progressive.jpg
```

`testorig12.ppm`, with libjpeg-turbo 3.2.0 through imagecodecs 2026.10.10 (default settings:
integer IDCT `JDCT_ISLOW`, fancy upsampling), run as
`uv run --with imagecodecs --with numpy python make.py testorig12.jpg testorig12.ppm`:

```python
import sys, imagecodecs, numpy as np
src, dst = sys.argv[1], sys.argv[2]
a = imagecodecs.jpeg8_decode(open(src, "rb").read())
h, w, c = a.shape
assert c == 3 and a.dtype == np.uint16 and a.max() <= 4095
with open(dst, "wb") as f:
    f.write(b"P6\n%d %d\n4095\n" % (w, h))
    f.write(a.astype(">u2").tobytes())
```

`djpeg -dct int testorig12.jpg` from libjpeg-turbo 3.1.0 writes a byte-identical file.

`whitetail-ari.jpg` and `whitetail-huffman.jpg`, with cjpeg and jpegtran from libjpeg-turbo 3.1.3
(Alpine 3.24, `libjpeg-turbo-utils`). libjpeg's arithmetic encoder drops the trailing zero bytes
of a scan, so the white rows decode almost entirely from zeros supplied at the EOI marker.
`python3 whitetail.py whitetail.pgm` writes the source image:

```python
import sys
w, h, top = 512, 512, 16
px = bytes(((x * 3 + y * 5) ^ (x * y)) & 0xFF if y < top else 255 for y in range(h) for x in range(w))
open(sys.argv[1], "wb").write(b"P5\n%d %d\n255\n" % (w, h) + px)
```

```sh
cjpeg -arithmetic -quality 75 whitetail.pgm > whitetail-ari.jpg
jpegtran -copy none whitetail-ari.jpg > whitetail-huffman.jpg
```

"This software is based in part on the work of the Independent JPEG Group."
