# H.263 video (`craft_video::h263`, feature `h263-decode`)

Clean-room H.263 (ITU-T H.263, 01/2005) decoder in pure Rust: the format of 3GPP video, early
Flash and videoconferencing, and the base of Sorenson Spark. Written from the published
Recommendation only; no H.263 implementation was read. `ffmpeg` is used as an external black-box
oracle in the tests (`craft_media_testkit::ffmpeg_or_skip`, `CRAFT_MEDIA_REQUIRE_ORACLES=1`).

`h263-decode` turns on `mpeg4-decode`: H.263's TCOEF, MCBPC (I-pictures), CBPY and MVD codes, its
inverse transform and its half-sample interpolation are the ones the MPEG-4 Part 2 decoder uses
for its short video header, so the decoder takes them from there instead of copying them.

## What is decoded

- Picture layer: PTYPE and PLUSPTYPE, the five standard formats and custom picture formats
  (any size from 4x4 to 2048x1152, cropped when not a multiple of 16), custom pixel aspect ratio,
  custom picture clock (parsed, not used), rounding type, GOB headers, PEI/PSUPP (skipped).
- Macroblock layer: I- and P-pictures, COD, MCBPC with the type-5 codes, CBPY, DQUANT, INTRADC,
  TCOEF with the fixed-length escape, stuffing.
- Annex D, Unrestricted Motion Vector: vectors outside the picture, the extended range with the
  predictor rule, and the PLUSPTYPE form with Table D.3 and the Table D.1/D.2 limits.
- Annex F, Advanced Prediction: four vectors per macroblock and overlapped motion compensation.
- Annex J, Deblocking Filter, including four vectors per macroblock.

A packet may hold one picture or several back to back. Pictures come out in decoding order.

## What is refused

These return `DecodeError::Unsupported` naming the Annex: E (syntax-based arithmetic coding),
G and M (PB-frames), I (advanced intra coding), K (slices), N (reference picture selection),
O (B, EI and EP pictures, scalability), P (resampling), Q (reduced-resolution update),
R (independent segments), S (alternative inter VLC), T (modified quantisation), and
Continuous Presence Multipoint (Annex C). A stream that uses one of them is not decoded
partially.

## Specifications

| Document | Used for |
|---|---|
| ITU-T H.263 (01/2005) | picture, GOB, macroblock and block syntax (§5), the VLC tables (Tables 7, 8, 12, 14, 16, 17, D.3), vector prediction and decoding (§6.1, Annex D, Annex F), inverse quantisation (§6.2), deblocking (Annex J) |

## Memory and hostile input

`Decoder::set_memory_budget` bounds the picture store (default
`craft_video::budget::DECODE_MEMORY_BUDGET`); a header that would not fit is refused before
anything is allocated. A packet holds at most 8 pictures. Every read from the bitstream is checked;
the tests cut and flip every bit of small streams and expect an error, never a panic.

## Not done yet

The encoder, `TrackDecoder` integration (`craft_media::CodecId` has no H.263 entry), and the
annexes above. Each is its own change.
