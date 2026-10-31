# Canon CRW raw compression — derived format (FROZEN)

Status: **frozen, revision 2** (plan §5 P4). The derivation is not tuned; any correction
must reopen it explicitly with a recorded cause and a new freeze commit.

**Revision 2 (2026-10-13), cause:** the maintainer supplied 1,516 D60 files, 50 times the
first corpus. 223 of them failed on code-table entries the first corpus never exercised,
and 9 were misread as having no low-bits plane. Two corrections, both recorded in the
derivation log (not part of this record) and re-verified in `03-verification.md`: the low-bits presence rule
(§1) gained the header-position test, and the selector 0 and 1 second tables were completed
(§3). Nothing else in the model changed, and every file of the first corpus still verifies. Everything here was derived from the CC0/private sample files
and black-box probing of LibRaw 0.22.2 (`unprocessed_raw.exe`), per the approved plan; no
decoder source was consulted. The chronological evidence is the derivation log, which is not part
of this record; the initial payload-layout analysis remains in LightCraft PR #3021.

## 1 Payload layout

RawData (CIFF record 0x2005), with w×h from SensorInfo (0x1031) and
(selector, 0, entropyOffset = 514, compressedLength) from DecoderTable (0x1835):

```
[ low-bits plane, ceil(w*h/4) bytes ]   present on some bodies (rule below)
[ 514 zero bytes ]                      (the DecoderTable's entropyOffset)
[ entropy bitstream, ff00-stuffed ]
[ tail: 00 00 / ff 00 00 (EOS) or ff d9 (+00) (PowerShot) ]
```

- **Low-bits plane**: 2 bits per sensor sample, row-major, LSB-first within each byte
  (sample 4i in bits 1:0 … sample 4i+3 in bits 7:6). Present ⇔ either
  (a) the first ⌈w·h/4⌉ + 514 bytes of the payload contain any 0xff byte NOT followed by
  0x00 (the entropy stream is always stuffed, the low-bits plane is raw), or
  (b) the 514 bytes at offset ⌈w·h/4⌉ are all zero (the zero header sits right after the
  plane; without a plane that offset is deep in entropy data).
  Rule (a) alone, the revision-1 rule, misses planes that hold no 0xff byte: some D60
  files have a lowest bit that is almost always 0 (low values 1 and 3 on about 0.04 % of
  samples), so no four neighbouring samples are all 3. Verified: present on
  D30/D60/10D/300D/G5/G6, absent on G1.
- On the EOS bodies the DecoderTable's compressedLength excludes the low-bits plane;
  on G5/G6 it includes it. The decoder should not rely on compressedLength: entropy
  data starts at (low-bits size, if present) + 514 and runs to the tail.

## 2 Entropy bitstream

After removing the stuffing (drop the 0x00 after every 0xff), bits are read
**MSB-first**. The stream codes the w×h sensor samples' 10-bit "high" plane
(the full sample is `(high << 2) | low_bits` where a low-bits plane exists, else
`high` itself):

- Samples are produced in row-major order, grouped in **blocks of 64 samples** laid
  flat over the frame (blocks cross row boundaries; w need not divide by 64).
- Two canonical Huffman code tables per DecoderTableNumber (0, 1, 2): the token that
  begins each block is read from the **first table**, every other token from the
  **second table**.
- A token decodes to a leaf **(r, s)** with r ≤ 15: skip r samples (each emits its
  DPCM chain value unchanged, i.e. a zero diff), then read s magnitude bits and emit
  one diff (s = 0 emits a zero diff). Magnitudes follow the JPEG rule, MSB-first:
  m ≥ 2^(s−1) → d = +m, else d = m − 2^s + 1. Sample diffs fit s ≤ 10 (10-bit high
  plane), but first-table leaves reach s = 11+: the carry-coded block DC difference
  spans ±2046.
- The **(0,0) leaf read as a non-first token is EOB**: the rest of the block emits
  zero diffs. ((0,0) as the block's first token is a plain zero diff.)
- **Carry**: the diff landing on each block's first sample has the previous block's
  (combined) first diff added: `d0 += carry; carry = d0`. When the first sample is
  covered by a token's skip (or EOB), its diff is exactly `carry` (and carry is
  unchanged).
- **DPCM chains**: two running predictors by sample parity, both initialised to 512,
  and both **reset to 512 at the start of every sensor row**.
- If the stream ends before the frame is complete, remaining samples emit their chain
  values (observed: the final 2–3 samples of each file).

## 3 Code tables

Extracted from the samples by known-plaintext alignment and frozen as data:

- Selector 0: frozen code tables — 12 first-table + 162 second-table codes.
- Selector 1: frozen code tables — 12 + 162.
- Selector 2: frozen code tables — 12 + 162.

Each second table now holds all 162 symbols: (r, s) for r = 0..15, s = 1..10, plus (0, 0)
and (15, 0). Revision 1 lacked (7..14, 10) in selector 0 and (14, 10) in selector 1: runs
of 7 or more zero differences followed by a difference of 512 or more, which the first
corpus never contained. The nine codes were learned from the wider corpus by the same
known-plaintext method; each landed in one of the unassigned 16-bit slots inside the code
range, of which there were exactly eight and one. The slots still unassigned are the
all-ones tail of each table (5, 20 and 3 codes) and the first tables' longest code.

Tables are shared across bodies per selector (verified across D60/10D/300D/Rebel for
selector 0; D30, G1 and the private D60s for selector 1; 10D, 300D and a private D60
for selector 2). The tables have canonical Huffman structure; codes up to 16 bits, the
`11111111xxxxxxxx` region carrying the larger (run, size) pairs. The JSON rows are a
superset of each file's exercised codes: every row was learned from a real stream
position and validated by the decode reaching the end of that frame; rows never
exercised by two files may in principle be mergeable duplicates of canonical slots,
which the Rust implementation may normalise, provided the verification table of
`03-verification.md` still reproduces exactly.

## 4 Not yet established / out of scope here

- **G5 anomaly**: its entropy stream decodes end-to-end with the selector-1 tables,
  but values diverge beginning at sample 2056 (= 4 × 514), and its low-bits plane fits
  no tested layout (top rows mostly match, decaying to 0 %, no constant shift). The
  decoder detects the G5 by its make-model and returns `Error::Unsupported` until a
  follow-up derivation resolves it; a caller may fall back to the embedded JPEG.
- The exact meaning of each file's tail bytes (handled by the stream-end rule).
- Big-endian (`MM`) CIFF files: no sample exists.
- PowerShot G2/G3/Pro1/Pro70/S30–S70: no CC0 samples (licence-excluded); decodable
  speculatively via the same selectors once their geometry is read from SensorInfo,
  to be marked unverified.
- Black level, white balance and colour (plan P3) are documented separately.

## 5 Verification

See `03-verification.md` (written after the freeze): full-frame exact-match decode
against LibRaw 0.22.2, with zero tuning after the freeze commit: every sample file, public
and private, matches except the PowerShot G5, which the decoder refuses (28 of 29).
