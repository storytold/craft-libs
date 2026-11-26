# FFV1

Clean-room FFV1 lossless video codec, written from RFC 9043. Feature `ffv1`.

## What is supported

| | Decoder | Encoder |
|---|---|---|
| Version | 3 (the one with a configuration record) | 3 |
| Coder | range coder, default or custom state transition table | range coder, default table |
| Colour | YCbCr (any 2^n chroma subsampling, gray, optional transparency), RGB through the JPEG 2000 reversible colour transform (optional transparency) | the same, RGB up to 15 bits |
| Depth | 8 to 16 bits per sample | 8 to 16 bits (RGB 8 to 15) |
| Slices | any raster whose boundaries are multiples of the chroma subsampling | the same |
| Frames | keyframes | keyframes only (`intra` 1) |
| Error detection | slice and configuration record CRCs, checked | written |

Not supported, and reported as `Error::Unsupported`: the Golomb-Rice coder (`coder_type` 0),
versions 0 and 1 (parameters inside each keyframe, no configuration record), inter frames
(`keyframe` 0), and slice rasters whose boundaries fall inside a chroma sample.

## Using it

The configuration record goes in the container (Matroska `CodecPrivate`, the AVI `strf`
extension, an MP4 `glbl` box); each frame is one packet.

```text
let enc = Encoder::new(&template_frame, EncoderConfig::default())?;
let config = enc.config_record();
let packet = enc.encode(&frame)?;

let dec = Decoder::new(&config, width, height)?;
let frame = dec.decode(&packet)?;
```

## Notes on the format

- The Parameters in the configuration record are range coded with the *default* state transition
  table. A custom table (`coder_type` 2) applies to the slices only.
- Each Quantization Table starts from fresh states; a Quantization Table Set is five of them.
- A slice is one range-coded stream: for the first slice the `keyframe` bit, then the slice header,
  then the samples. Cb and Cr share one set of context states; transparency has its own.
- The context is the sum of five quantised differences. With the default tables this encoder
  writes 15125 contexts; FFmpeg refuses tables that multiply to more than 32768, so the encoder
  stays below that although the RFC's limit (on half the product) is looser.
- The encoder ends a stream with the top of its final range, so bytes after it (a CRC in the
  configuration record) cannot change what decodes.

## Verification

`tests/it/ffv1.rs`: round trips over every shape, hostile input (every truncation and bit flip),
and ffmpeg as the oracle in both directions (skipped without ffmpeg, a failure with
`CRAFT_MEDIA_REQUIRE_ORACLES=1`).
