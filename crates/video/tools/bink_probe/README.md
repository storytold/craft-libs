# Bink video probe scripts

The scripts that derived the Bink video format for `craft_video::bink` by black-box probing.
They run ffmpeg as a program on real and hand-built `.bik` files and compare its output
pictures and error messages with a model. No Bink implementation's source is read by them or was
read to write them.

They are a record of how each rule and table was established, and a way to check it again. They
are not part of the build or the tests.

## Requirements

- Python 3 and `ffmpeg` on `PATH`.
- `BINK_PROBE_SAMPLE`: the path of the probe sample. The synthetic frames are built on one real
  file: 180x139, revision `i`, one audio track or none, whose frames 0 and 1 are black and whose
  frame 2 is the first with picture content. Bit positions quoted in some scripts (the luma
  plane of frame 0 ending at bit 461, chroma planes at bits 480 and 1184) belong to the file
  that was used and are only used by the early exploration scripts (`t8.py`, `t9.py`).

No sample is in the repository: existing Bink files are not ours to commit, and no tool here
writes them.

## Scripts

| Script | What it establishes |
|---|---|
| `probe.py` | Harness: reads a `.bik` file, builds a file from video packets, decodes it with ffmpeg |
| `t1.py`, `model.py` | Bit writer and reader; the tree header reader |
| `t8.py`, `t9.py` | A luma plane written from scratch (all fill blocks, one coded colour) |
| `trees2.py` | The 16 Huffman trees, walked prefix by prefix (`trees.json`) |
| `synth.py`, `scans.py` | Synthetic key-frame planes; the 16 run-fill scan patterns (`scans.json`) |
| `synth2.py`, `mvmap.py` | Synthetic inter frames; which motion vectors ffmpeg accepts |
| `cenc.py`, `cmodel.py` | Encoder and model of the coefficient coding, and its test on random streams |
| `rmodel.py` | Model of the residue coding and its test on random streams |
| `ac1.py`, `ac3.py`, `ac5.py`, `idct1.py`, `inter3.py` | Single DCT blocks: quantiser position, coefficient estimates, basis patterns |
| `dataset.py`, `fit1.py` | Random blocks from ffmpeg and the search for the exact fixed-point transform |
| `xform.py` | The transform as found |
| `wverify.py`, `texact5.py` | Quantiser weights (`weights.json`) and every dequantisation multiplier (`tables_measured.json`) |
| `inter6.py` | Random inter DCT blocks with motion at every quantiser |
| `dec.py`, `dec3.py` | The model decoder, compared with ffmpeg frame by frame on real files |
| `gen_tables.py` | Writes `src/bink/tables.rs` from the three data files |

## Data

- `trees.json`: for each tree, the code of each rank as the bits in read order.
- `scans.json`: for each scan pattern, the sample index of each step.
- `weights.json`: the quantiser weights in sixteenths, used as starting estimates.
- `tables_measured.json`: the multipliers. Where several neighbouring integers decode every
  coefficient value in the legal range identically, the one nearest the estimate was taken.

## Regenerating the tables

```bash
python3 crates/video/tools/bink_probe/gen_tables.py
```
