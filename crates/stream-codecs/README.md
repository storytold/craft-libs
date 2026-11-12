# craft-stream-codecs

Byte-stream and bi-level image codecs that PDF, PostScript and TIFF share. The crate knows no
container: a caller hands it bytes and parameters. A PDF `/Filter` chain (stage order, limits
across stages, Flate, accounting) belongs to `craft-pdf`, which builds on this crate; a TIFF
reader can use the same LZW, predictor, CCITT and JBIG2 code.

- **Licence:** MIT OR Apache-2.0. Written from ISO 32000-2 §7.4, TIFF 6.0 §14 (predictor 2) and
  RFC 2083 §6 (PNG filters). CCITT Group 4 encoding (ITU-T T.6) is the `fax` crate's (MIT). CCITT
  decoding (T.4 and T.6) is `hayro-ccitt`'s (MIT OR Apache-2.0), bounded here: this crate has
  no CCITT decoder of its own. JBIG2 decoding is `hayro-jbig2`'s (MIT OR Apache-2.0), bounded
  here the same way. JBIG2 encoding
  is written from the public ISO/IEC 14492 Final Committee Draft (WG1 N1359, 1999); Annex H.2
  checks the arithmetic coder byte for byte.
- No `unsafe`. Never panics on input.
- From PdfCraft's `pdfcraft-filters` crate (revision `80e26e674d1aa33078e83c8139875047f3289eb6`):
  its codec modules, moved whole. Its filter chain and Flate stay with the PDF code.

## API

```rust
pub struct Params { predictor, colors, bits_per_component, columns, early_change }  // i64s; Default = 1, 1, 8, 1, 1
pub enum CodecError { Corrupt { filter, detail }, LimitExceeded(usize), Unsupported(String), Spent }
pub struct Failure { error, partial, discarded }   // what a decoder decoded before it failed
pub type Step = Result<Vec<u8>, Failure>;
pay(work: &mut u64, steps) -> Result<(), CodecError>   // an allowance shared across a run of images

lzw::decode(data, early_change, max) -> Step;        lzw::encode(data, early_change) -> Vec<u8>
ascii85::decode(data, max) -> Step;                  ascii85::encode(data) -> Vec<u8>
asciihex::decode(data, max) -> Step;                 asciihex::encode(data) -> Vec<u8>
runlength::decode(data, max) -> Step;                runlength::encode(data) -> Vec<u8>
predictor::decode(&params, data, name, rows) -> Step; predictor::encode(&params, data) -> Result<Vec<u8>, CodecError>

pub struct CcittParams { k, columns, rows, end_of_line, encoded_byte_align, end_of_block, black_is_1 }
decode_ccitt_within(data, &params, max_rows, work: &mut u64) -> Result<Vec<u8>, CodecError>
encode_g4(data, columns, rows) -> Result<Vec<u8>, CodecError>

decode_jbig2_within(data, globals, columns, rows, max, work: &mut u64) -> Result<Vec<u8>, CodecError>
encode_jbig2_generic(data, columns, rows) -> Result<Vec<u8>, CodecError>
encode_jbig2(data, columns, rows) -> Result<Vec<u8>, CodecError>
encode_jbig2_shared(&[(data, columns, rows)]) -> Result<Option<SharedJbig2>, CodecError>
```

`max` bounds a decoder's output and gives `LimitExceeded`. `Corrupt` names the codec in `filter`
by its PDF filter name. Bi-level images are rows of 1-bit samples, most significant bit first,
each row on a new byte.

## Behaviour

| Codec | Decode | Encode |
|---|---|---|
| LZW | 9–12-bit MSB-first codes, clear (256), end (257), early change on or off; a missing end code or initial clear is tolerated; a full table stops growing | greedy, leading clear, clear when the table fills, end code |
| Predictors | 2 = TIFF (1/2/4/8/16 bits per component), 10–15 = PNG with per-row tags 0–4; partial final rows decoded; bad parameters or tags are `Corrupt`; with `rows`, data that is exactly that many rows without tag bytes is taken as the samples and reported as damage | 2, 10 None, 11 Sub, 12 Up, 13 Average, 14 Paeth, 15 per-row optimum |
| ASCIIHex | white space ignored, odd digit padded with 0, `>` ends (optional), other characters are `Corrupt` | upper case, newline every 64 digits, `>` |
| ASCII85 | `<~` prefix, `z`, white space, `~>` (or bare `~`, or none), final partial group; `z` inside a group, a 1-character final group, overflow or bad characters are `Corrupt` | `z` for zero groups, newline every 75 characters, `~>` |
| RunLength | 0–127 literal, 129–255 repeat, 128 ends (optional); a truncated run is `Corrupt` | runs of 2 or more repeat, literals up to 128, 128 |
| CCITT fax | Group 3 (1-D and 2-D) and Group 4 through `hayro-ccitt`, as rows of 1-bit samples, black 0 unless `black_is_1`, at most `max_rows` rows (fewer with `rows`), the rows before any damage, up to `MAX_CCITT_COLUMNS` (1,048,576) samples a row; the rows are paid for from `work` before they are decoded | `encode_g4`: Group 4 with an end-of-block mark, 0 bits coded black; up to `MAX_G4_COLUMNS` (32,767) samples a row |
| JBIG2 | `decode_jbig2_within`: the embedded organisation through `hayro-jbig2`, with the decoded globals if any, as rows of 1-bit samples, black 0; the page has to be `columns` × `rows`, within `max` bytes and `MAX_JBIG2_PIXELS` (2^28), checked before it is decoded; reading and decoding spend from `work`, in the decoder's steps | `encode_jbig2_generic`: one immediate lossless generic region (template 0, typical prediction, MQ coder). `encode_jbig2`: the smaller of that and symbol coding (exact copies only, so the stream decodes to the same bits). `encode_jbig2_shared`: a group of images sharing one symbol dictionary (the PDF `/JBIG2Globals` data) |

The limits of the JBIG2 encoder (symbols per plan, instances per text region, marks, runs) are
those `hayro-jbig2` reads back; they are documented on the constants in `src/jbig2/symbols.rs`.

## Tests

- Unit tests per codec with hand-computed vectors (the ISO 32000-2 LZW example, PNG and TIFF rows).
- LZW is cross-checked against `weezl` (dev-dependency) in both directions for both early-change settings.
- `ccitt.rs`: decoding gives back the rows `encode_g4` was given; Group 3 layouts over seeded
  pictures; Group 4 output decoded by `hayro-ccitt` matches the input bits, with property tests
  over bitmaps of noise and of runs; any input returns rather than panics.
- `mq.rs`: the arithmetic coder reproduces Annex H.2's 30 bytes from its 256 decisions.
- `jbig2.rs`, `jbig2/symbols.rs`: output decoded by `hayro-jbig2` matches the input bits for
  every coding, plan and strip size, with property tests; size and limit refusals.
- `jbig2/decode.rs`: `decode_jbig2_within` gives back the rows of both codings; refuses a page
  of another size, past `max` or past `MAX_JBIG2_PIXELS` before decoding it, globals that are
  not JBIG2 and damaged data; stops when `work` is spent; every prefix of a stream returns
  rather than panics.

## Not here

Flate (a container's choice of zlib settings and tolerances), DCT and JPEG 2000, CCITT Group 3
encoding.
