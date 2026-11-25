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
  frame 2 is the first with picture content. Some scripts use bit positions of the file that was
  used (the luma plane of frame 0 ending at bit 461, its chroma planes starting at bits 480 and
  1184): `t8.py`, `t9.py`, `synth.py` and `trees2.py`, and through them `scans.py`.
- `BINK_PROBE_DECODER` (`fuzz_diff.py`): the program that decodes a `.bik` file with the Rust
  decoder; by default the `bink_binkdec` example in the workspace's release target directory.
- `BINK_PROBE_CASES` (`fuzz_diff.py`): the directory in which each run makes its own one for the
  cases it keeps; by default the system's temporary directory.
- `IF` and `REV` (`cmodel.py`): switch the coefficient model to variants that were tried and
  rejected (`IF=1`: members left at zero go next to their group instead of to the front of the
  list; `REV=0`: they keep their order). The defaults are the model that matches.

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
| `xform.py` | The transform as found, with its 32-bit wrapping |
| `wverify.py`, `texact5.py` | Quantiser weights (`weights.json`) and every dequantisation multiplier (`tables_measured.json`) |
| `inter6.py` | Random inter DCT blocks with motion at every quantiser |
| `dec.py`, `dec3.py` | The model decoder, compared with ffmpeg frame by frame on real files |
| `fuzz_diff.py` | Random synthetic streams of every block type, size and both revisions, decoded by ffmpeg and by the Rust decoder (the `bink_binkdec` example) and compared: the edge rules that no real file shows |
| `gen_tables.py` | Writes `src/bink/tables.rs` from the three data files |

## Data

- `trees.json`: for each tree, the code of each rank as the bits in read order.
- `scans.json`: for each scan pattern, the sample index of each step.
- `weights.json`: the quantiser weights in sixteenths, used as starting estimates.
- `tables_measured.json`: the multipliers, each the one integer that reproduces ffmpeg for every
  probe, with coefficients of up to 15 bits. The exception is the DC multiplier of most
  quantisers, where the exact ratio and the next integer decode every DC value a start field can
  hold identically; the exact ratio was taken, and DC values up to 16 bits were then checked
  with `fuzz_diff.py`.

## Regenerating the tables

```bash
python3 crates/video/tools/bink_probe/gen_tables.py
```
