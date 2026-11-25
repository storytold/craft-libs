# craft-heif test fixtures

- `grid-422-10bit.heic` (3,471 bytes, SHA-256 `b5b89e4439c238aa959a066ad5bbd6c8d858bc0ace3b14f2f00b97884202dd54`):
  a 192 × 128 `grid` of 3 × 2 tiles of 64 × 64, HEVC 4:2:2 10-bit (range extensions), `nclx` full
  range: the layout of Canon and Sony `.HIF` files. Copied unmodified from
  [heic-rs v0.1.5](https://github.com/tbraun96/heic-rs/blob/621014b8e60bb3527670e0b15445926e799340b8/tests/fixtures/grid-422-10bit.heic)
  (MIT OR Apache-2.0). heic-rs made it with `scripts/make-grid-vectors.sh`: libheif's `heif-enc`
  (x265), run as a separate program, encodes a picture that `scripts/gen-grid-pngs.py` computes
  from a formula (`grid_422`), so it holds no third-party image data. `src/tests_chroma.rs`
  computes the same formula to check the decode.
