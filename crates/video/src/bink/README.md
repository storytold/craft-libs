# Bink video decoder (`craft_video::bink`, feature `bink`)

Clean-room Bink 1 video decoder in pure Rust, for revisions `f` and `i`. Depends on `craft-media`
(`CodecParams`) and `thiserror`. No `unsafe`, no panics.

Bink has no published specification. This decoder was written from two sources:

- the public description of the format's outline on the MultimediaWiki (`wiki.multimedia.cx`,
  "Bink Container" and "Bink Video"), which names the block types and the kinds of bundle but
  leaves the Huffman trees, the bundle layout, the coefficient and residue coding, the transform
  and the quantiser tables unstated;
- black-box probing of an existing decoder: ffmpeg was run as a program on real files and on
  hand-built streams, and its output pictures and error messages were observed.

No FFmpeg source, and no other Bink implementation, was read. Every rule below that the wiki does
not state was derived from those observations, and every table in `tables.rs` is a measurement.
The scripts that did this are in `crates/video/tools/bink_probe`; its `README.md` says how to
rerun them.

## API

```rust
use craft_media::bink::open;
use craft_video::bink::Decoder;

let file = open(&bytes)?;
let mut dec = Decoder::from_params(&file.video_params())?;
for i in 0..file.frames.len() {
    let frame = file.read_frame(&bytes, i)?;
    let picture = dec.decode(file.split_frame(&frame)?.video)?; // 8-bit 4:2:0, tight planes
}
```

- Frames are decoded in order. A frame that is not a key frame builds on the picture before it.
- After an error the decoder keeps the last picture that decoded.
- `reset()` forgets the previous picture, for decoding from a key frame after a seek.

Refused by `Decoder::new`: revisions other than `f` and `i`, the alpha flag and the grayscale
flag (no sample of any was available to check against), and a picture larger than the decoder
memory budget.

## The format, as derived

Bits are read from the least significant end of each byte; a multi-bit field has its first bit
lowest.

### Packet

| Revision | Layout |
|---|---|
| `i` | a 32-bit word (the byte size of the luma plane's data, not needed for decoding), then the luma, Cr and Cb planes |
| `f` | the luma, Cb and Cr planes |

In revision `f` a colour is a sign bit (bit 7) and a 7-bit magnitude around 128: a coded value
`v` is the sample `128 - (v & 127)` when bit 7 is set and `128 + v` otherwise. In revision `i` a
colour is the sample itself.

Each plane starts on a 32-bit boundary, counted from the start of the packet. A plane is
`ceil(width / 8)` by `ceil(height / 8)` blocks of 8x8 samples; chroma planes are half the luma
size, rounded up.

### Plane

1. 23 tree headers: block types, 16x16 sub-types, 16 trees for the high nibble of a colour (one
   per value of the previous colour's high nibble), the low nibble of a colour, patterns, x
   offsets, y offsets and run lengths.
2. For each row of blocks: a new chunk for every bundle that is used up and not finished, then
   the row's blocks. Blocks take values from the bundles and, for some types, read further bits
   in place.

A tree header is a 4-bit tree number and, unless it is 0, the order of the tree's 16 symbols:

- a 1 bit, a 3-bit count `n`, then `n + 1` symbols of 4 bits that come first; the rest follow in
  ascending order;
- or a 0 bit, a 2-bit depth `d`, then `d + 1` merge passes over runs of 1, 2, 4 and 8 symbols:
  two neighbouring runs are merged by taking, for each output symbol, one bit that says which
  run gives its next symbol, until one run is empty.

The 16 trees are fixed prefix codes (`TREES`). Tree 0 is four raw bits.

### Bundles

A chunk starts with its length. The field is `bit_length(n + 511)` bits wide, where `n` is the
most values a row of blocks can need:

| Bundle | `n` | After the length |
|---|---|---|
| Block types, sub-types | blocks per row (sub-types: half) | 1 bit: all values equal (4 bits follow), else coded values |
| Colours | 64 per block | 1 bit: all values equal (one coded colour follows), else coded colours |
| Patterns | 8 per block | coded values: low nibble, then high nibble |
| X and y offsets | blocks per row | 1 bit: all values equal, else coded values; a 4-bit magnitude and a sign bit when it is not zero (1 = negative) |
| Intra DC | blocks per row | an 11-bit start value, then groups of up to 8 deltas |
| Inter DC | blocks per row | a 10-bit start magnitude and a sign bit when it is not zero, then the same groups |
| Run lengths | 48 per block | 1 bit: all values equal (4 bits follow), else coded values |

- A length of 0 ends the bundle for the plane.
- In the two type bundles, symbols 12 to 15 repeat the previous value 4, 8, 12 or 32 times. The
  previous value is 0 at the start of every chunk.
- A colour is its high nibble, coded with the tree the previous colour's high nibble selects,
  then its low nibble.
- A group of DC deltas starts with a 4-bit bit count. A count of 0 repeats the value; otherwise
  each delta is that many bits and a sign bit when it is not zero.

### Blocks

| Type | Block |
|---|---|
| 0 | copied from the previous picture |
| 1 | a 16x16 block: a sub-type (3, 5, 6, 8 or 9) is read, the block is decoded as 8x8 and every sample doubled. It is listed on both of its rows and decoded on the first. |
| 2 | motion: an x and a y offset; copied from the previous picture at that offset |
| 3 | run fill: a 4-bit scan pattern; then runs until 64 samples are placed: a run length (bundle value + 1), then 1 bit: one colour for the run, or a colour per sample. When 63 samples are placed the last takes one more colour. |
| 4 | motion plus residue: offsets, then the residue coder on the samples |
| 5 | intra DCT: one intra DC value, then coefficients and a quantiser |
| 6 | fill: one colour |
| 7 | inter DCT: offsets, one inter DC value, then coefficients and a quantiser; added to the motion block |
| 8 | pattern fill: two colours and 8 pattern bytes, one per row; bit `x` of a byte selects the second colour |
| 9 | raw: 64 colours in raster order |

Sums of a prediction and a residue wrap to 8 bits, as do intra DCT results. Nothing is clamped.

### DCT coefficients

A DCT block reads a 4-bit plane count, the coefficient data, then a 4-bit quantiser index. The
coefficient data is a walk over a list, once per bit plane from the highest down. The list starts
as the 2x2 groups at (row 0, column 2), (2, 0) and (2, 2), then the single positions (0, 1),
(1, 0) and (1, 1). Each entry reads 1 bit; 0 leaves it for the next plane. Otherwise:

- a single position becomes non-zero: its magnitude is the plane's bit, followed at once by its
  lower bits, then a sign bit. The entry is removed.
- a 2x2 group codes its four members in raster order: a 1 bit leaves the member zero and puts it
  at the front of the list as a single position (it is not looked at again in this plane); a 0
  bit makes it non-zero as above. One of the three starting groups is then replaced by its 4x4
  child region ((0, 4), (4, 0) or (4, 4)), which is looked at again at once; any other group is
  removed.
- a 4x4 region is replaced by its top-left 2x2 group, looked at again at once, and its other three
  2x2 groups are added to the end of the list.

### Residue

A residue block reads a 7-bit budget (stored value + 1), a 3-bit top plane, then the same list
walk over sample positions, with the list starting as the groups (0, 2), (2, 0), (2, 2) and
(0, 0). It differs from the coefficient coder in two ways:

- a new non-zero sample gets only the plane's bit and its sign; before each plane's walk, every
  sample that is already non-zero reads 1 bit that adds the plane's bit to its magnitude;
- every new non-zero sample and every added bit spends one unit of the budget, and the block
  ends at once when the budget is spent.

### Dequantisation and transform

A coefficient is multiplied by the entry of its position in the quantiser's table and shifted
right by 11; the DC value likewise. The tables (`INTRA_QUANT`, `INTER_QUANT`) hold 16 fractional
bits and include the scale factors of the transform. The inverse transform is a fixed-point
scaled inverse DCT: columns, then rows, with four multiplications per pass by 2896, 3784, 2217
and 5352 (each shifted right by 11), and a final `(x + 127) >> 8`.

## How each part was established

| Part | Method |
|---|---|
| Container, plane order, alignment | rewriting planes of real frames with known fill colours and moving them |
| Tree headers | re-encoding a header in an equivalent form and requiring identical output |
| Huffman trees | a synthetic fill block whose colour is coded with each tree; a marker colour after the code confirms its length |
| Bundles and plain block types | synthetic planes, then exact agreement on real frames |
| Scan patterns | a run-fill block of 64 one-sample runs per pattern |
| Coefficient and residue coding | a model compared with ffmpeg on random streams (data length, positions, signs, values) |
| Transform | candidate fixed-point transforms compared with ffmpeg on random blocks until one matched every sample |
| Quantiser tables | each of the 2048 entries narrowed to one integer by probes chosen to separate the candidates |
| Whole decoder | every frame of the sample files compared with ffmpeg, sample for sample |

## Limits

- The decoder requires a motion vector's reference block to lie inside the padded plane. ffmpeg
  accepts some references that run across a row end and rejects some inside the plane; no real
  file needs either.
- A block that needs a value its bundle does not hold is an error.
- Arithmetic in the transform wraps, so no coefficient can overflow.
- Pictures are bounded by `budget::DECODE_MEMORY_BUDGET`, chunk sizes by their length fields.

Not verified, because the samples cannot show it: whether the run-length field counts 48 or 64
values per block and whether the sub-type field counts half the blocks per row or all of them
(the two give the same width at every sample size), and everything about revisions other than
`f` and `i`.

## Tests

- Unit tests: the bit reader, refusals, synthetic frames of each block type written bit by bit and
  checked against values observed from ffmpeg for the same constructions, and hostile input
  (never a panic).
- `tests/bink_oracle.rs` compares every decoded frame with ffmpeg for the `.bik` files under
  `CRAFT_BINK_SAMPLES_DIR` (`CRAFT_BINK_ORACLE_FRAMES` limits the frames per file). It is skipped
  when the variable is not set: no tool here writes Bink files, and existing ones are not ours
  to commit.
