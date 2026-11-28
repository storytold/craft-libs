# craft-video: DV

Clean-room, pure-Rust (no `unsafe`) decoder of DV video pictures. Part of `craft-video` (feature `dv`, or
`dv-decode` alone). It depends on nothing outside `std` (rayon with the crate's `threads` feature) and builds for
`wasm32-unknown-unknown`.

- **Formats:** 25 Mb/s consumer DV and DVCAM (IEC 61834-2: 525/60 in 4:1:1, 625/50 in 4:2:0), DVCPRO (SMPTE ST 314
  at 25 Mb/s: 4:1:1 in both systems), and DVCPRO50 (SMPTE ST 314 at 50 Mb/s: 4:2:2, the frame in two channels of DIF
  sequences). 4:3 and 16:9 (the VAUX display select mode).
- **Output:** a 720 × 480 or 720 × 576 picture of 8-bit Y'CbCr in the frame's own sampling, converting into
  `frame::VideoFrame` (interlaced, bottom field first; BT.601 colours: SMPTE 170M for 525/60, BT.470 BG for 625/50;
  the sample aspect ratio that makes the whole 720 pixels 4:3 or 16:9).
- **Not yet:** DVCPRO HD (SMPTE ST 370), the sound and subcode in the DIF blocks, and an encoder (storytold/craft-libs#1512).

## Sources

- ITU-R BT.1618-1 (2011-03; withdrawn, still published by the ITU): the DIF structure (Fig. 4, Tables 1–5), the VAUX
  packs (Fig. 7, Tables 13 and 14), the macro block and super block order (§1.7.2.1 and Table 19, §2.1, Figs 16, 20,
  22, 24 and 26), the DCT and weighting (§2.2, Fig. 27), classes and quantisation (§2.3, Fig. 28, Table 23), the
  variable-length codes (§2.4, Tables 24 and 25), the three passes over a video segment (§2.6) and the error codes
  (§2.6, Fig. 33). The figures were read from the document's EMF drawings.
- SMPTE ST 314 (2005, stabilised 2013; published by SMPTE): the same content, used to cross-check the tables; the
  50 Mb/s structure from BT.1618-1 Fig. 2 (two channels of 10 / 12 DIF sequences), §1.7.2.1 and Table 18 (channel 0
  codes the even super block rows, channel 1 the odd ones), Figs 19, 21 and 25 (macro blocks of two 16 × 8 Y blocks,
  super blocks of 9 × 3 macro blocks down and up the columns), Fig. 29 (the compressed macro block: Y0 and Y1, then
  the X0 / X1 data areas, which are not decoded, and the additional AC area E) and §2.6 (passes 2 and 3 fill E after
  the space left in the Y areas). The VAUX source pack's signal type 00100 marks the 50 Mb/s structure.
- US 5,585,930 (a published patent): the 625/50 4:2:0 super block of IEC 61834 (12 × 5 macro block rows and columns,
  each a 9 × 3 group numbered down and up the columns) and its colour difference on every other line, which the two
  documents above do not give.

Two corrections to Table 25, found by checking every code against Table 24's lengths (a unit test checks that each
code has the length Table 24 gives and that no code is a prefix of another): both the ITU and the SMPTE text print
(7, 1) and (8, 1) as `1110000s` and `1110001s`, and (1, 16) and (1, 17) as `11111011110s` and `11111011111s`, one bit
short (as printed, the first two would overlap (9, 1)'s `11100010s`); Table 24's lengths (8 and 12) give `11100000s`,
`11100001s`, `111110111110s` and `111110111111s`.

No other DV implementation was read. ffmpeg is used only as an external fixture generator and test oracle.

## Errors

Every length and number read from a frame is checked. A frame shorter than its DIF sequences is an error; damaged
blocks inside a whole frame decode grey (the DC error code of §2.6 is honoured the same way).

## Tests

`tests/it/dv.rs` makes DV files with ffmpeg (525/60 4:1:1, progressive and interlaced with the 2-4-8 DCT; 625/50
4:2:0 16:9 interlaced; DVCPRO50 525/60 interlaced and 625/50 16:9) and compares every frame's planes with ffmpeg's
decoding in the same sampling (`yuv411p`, `yuv420p`, `yuv422p`). DV leaves the inverse DCT's arithmetic to the
decoder, so planes are compared by PSNR (above 45 dB each; about 61–64 dB in practice). The unit tests check the
code table, the scan orders, the quantisation table, that the super block layouts cover the picture exactly once,
that the inverse DCT undoes §2.2.1's forward transform, and the three-pass reading. Damaged and truncated frames go
through `craft_media_testkit::hostile::hammer`.
