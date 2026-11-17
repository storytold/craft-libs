# AVIF decode fixtures

Small AVIF files from an independent encoder (libaom through FFmpeg 7.1), for `src/codecs/avif_decode.rs`.

- `quad-source.rgb`: the 33×17 source, raw RGB24 four-colour quadrants (red, green, blue, near-white).
- `quad-420-bt709.avif`: 8-bit 4:2:0, limited range, BT.709 matrix, odd size (FFmpeg leaves the primaries and transfer unspecified):
  `ffmpeg -f rawvideo -pix_fmt rgb24 -s 33x17 -i quad-source.rgb -c:v libaom-av1 -crf 12 -still-picture 1 -pix_fmt yuv420p -colorspace bt709 -color_primaries bt709 -color_trc iec61966-2-1 -color_range tv quad-420-bt709.avif`
- `quad-420-bt709.dav1d.yuv`: that file's AV1 picture as libdav1d decodes it, planar 8-bit 4:2:0 (Y, then Cb, then Cr):
  `ffmpeg -c:v libdav1d -i quad-420-bt709.avif -f rawvideo -pix_fmt yuv420p quad-420-bt709.dav1d.yuv`
- `quad-444-gbr-lossless.avif`: lossless 8-bit 4:4:4 with the identity (GBR) matrix, full range:
  `ffmpeg -f rawvideo -pix_fmt rgb24 -s 33x17 -i quad-source.rgb -c:v libaom-av1 -crf 0 -still-picture 1 -pix_fmt gbrp -colorspace rgb -color_range pc quad-444-gbr-lossless.avif`.
  It decodes to `quad-source.rgb` exactly.
