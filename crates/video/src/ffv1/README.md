# FFV1

Clean-room FFV1 lossless video codec, written from RFC 9043. Feature `ffv1`.

## What is supported

| | Decoder | Encoder |
|---|---|---|
| Version | 0, 1 (parameters in each keyframe) and 3 (with a configuration record) | 3 |
| Coder | Golomb-Rice; range coder, default or custom state transition table | range coder, default table |
| Colour | YCbCr (any 2^n chroma subsampling, gray, optional transparency), RGB through the JPEG 2000 reversible colour transform (optional transparency) | the same, RGB up to 15 bits |
| Depth | 8 to 16 bits per sample | 8 to 16 bits (RGB 8 to 15) |
| Slices | any raster whose boundaries are multiples of the chroma subsampling (versions 0 and 1 have one slice) | the same |
| Frames | keyframes and non-keyframes | keyframes only (`intra` 1) |
| Error detection | slice and configuration record CRCs, checked | written |

Not supported, and reported as `Error::Unsupported`: slice rasters whose boundaries fall inside a
chroma sample, and a non-keyframe whose slice selects another Quantization Table Set than the frame
before it did.

## Using it

The configuration record of a version 3 stream goes in the container (Matroska `CodecPrivate`,
the AVI `strf` extension, an MP4 `glbl` box); versions 0 and 1 have none, and the decoder takes an
empty one. Each frame is one packet. Packets go to the decoder in decode order: a non-keyframe
continues the context states the frame before it ended with. After a seek, `reset` forgets them;
a non-keyframe is then an error until the next keyframe, as it is after a frame that failed.

```text
let enc = Encoder::new(&template_frame, EncoderConfig::default())?;
let config = enc.config_record();
let packet = enc.encode(&frame)?;

let dec = Decoder::new(&config, width, height)?;
let frame = dec.decode(&packet)?;
```

## Sources

RFC 9043, *FFV1 Video Coding Format Versions 0, 1, and 3* (August 2021). The decoder follows:

- 3.1 to 3.7: border, median predictor (with the 16-bit exception of 3.3.1 for the range coder),
  quantisation tables, context, colour spaces and the RGB exception of 3.7.2.1;
- 3.8.1: the range coder, its termination (3.8.1.1.1) and state transition tables;
- 3.8.2: the Golomb-Rice coder: signed Golomb-Rice codes with escape (3.8.2.1), run mode (3.8.2.2),
  sign extension (3.8.2.3), the adaptive VLC state (3.8.2.4) and its initial values (3.8.2.5);
- 3.8.1.3 and 3.8.2.5: states are reset by keyframes only, so non-keyframes continue them;
- 4.2: the Parameters of versions 0, 1 and 3, with the values inferred when absent (4.2.7 and
  4.2.11 to 4.2.17);
- 4.3 to 4.9: configuration record, frame, slice, slice header, slice content and slice footer;
- 4.4 and 5: a frame has as many slices as its packet holds, and each place on the slice raster is
  covered by exactly one of them; a non-keyframe repeats the slices of the frame before it.

## Notes on the format

Found by comparing with ffmpeg's FFV1 streams (ffmpeg run as a separate program only), where
RFC 9043 leaves the behaviour open or ffmpeg's streams depart from it:

- The Parameters are range coded with the *default* state transition table, in the configuration
  record and in the keyframes of versions 0 and 1. A custom table (`coder_type` 2) applies to the
  slices only.
- Each Quantization Table starts from fresh states; a Quantization Table Set is five of them.
- Booleans in the Parameters use the first state of the Parameters' state array.
- A version 3 slice is one range-coded stream: for the first slice the `keyframe` bit, then the
  slice header, then (range coder) the samples. `slice_size` counts the header and content, not the
  footer. Cb and Cr share one set of context states and one Quantization Table Set; transparency
  has its own.
- Switching to Golomb-Rice: in version 3, a sentinel symbol of state 129 follows the slice header,
  and the Golomb-Rice bits start one byte before where the range decoder has then read to. In
  versions 0 and 1 no sentinel symbol is read: the bits start one byte before where the range
  decoder has read to after the keyframe's Parameters (or a non-keyframe's `keyframe` bit).
- Golomb-Rice run mode: the run count and run mode start afresh on every line. `run_index` belongs
  to a plane and lives on from line to line, except in RGB slices, where the planes take turns line
  by line and share one `run_index` that starts at 0 for each slice. This departs from section
  3.8.2.2.1, which resets `run_index` for each plane: with one index per plane, ffmpeg's 8-bit RGB
  (`bgr0`) Golomb-Rice streams do not decode.
- The context is the sum of five quantised differences. With the default tables this encoder
  writes 15125 contexts; FFmpeg refuses tables that multiply to more than 32768, so the encoder
  stays below that although the RFC's limit (on half the product) is looser.
- The encoder ends a stream with the top of its final range, so bytes after it (a CRC in the
  configuration record) cannot change what decodes. The range decoder reads two bytes ahead of
  what an encoder writes before it ends a stream; one that reads further has run past the end of
  cut data, which the configuration record and range-coded slices report as truncated.

## Verification

`tests/it/ffv1.rs`: round trips over every shape, hostile input (every truncation and bit flip),
and ffmpeg as the oracle in both directions (skipped without ffmpeg, a failure with
`CRAFT_MEDIA_REQUIRE_ORACLES=1`). ffmpeg's version 0, 1 and 3 streams with the Golomb-Rice coder
and both range coder tables, with non-keyframes, in 4:2:0, 4:2:2, 4:4:4, gray, with transparency,
8-bit RGB and 10-bit YCbCr and RGB, decode to ffmpeg's own pixels. Damaged frames of each kind
never panic or hang, and are errors where a CRC or the end of the Golomb-Rice data shows it.
