# Synthetic ATN parser fixtures

These seven tiny files are original synthetic test data. The four version-16 files were independently
encoded field by field from the version-16 layout documented in `src/atn.rs`, without calling
this crate's writer; the three version-12 files (`v12-*.atn`) encode the version-12 layout documented
there (photocraft#10743). They are licensed MIT OR Apache-2.0 under the repository's licenses. They contain no
user artwork, Photoshop program code or proprietary assets. They are not Photoshop-authored
interoperability oracles.

| File | Bytes | Purpose | SHA-256 |
|---|---:|---|---|
| `empty.atn` | 13 | Empty set and explicit big-endian version | `6fd936f91296a13b0e7456a38b139a100fd17407d1c582d4502ce76413d01539` |
| `metadata.atn` | 214 | UTF-16 surrogate pairs/unpaired units, action order, function key, modifiers, panel flags, disabled step, unknown string event, dialog options | `cec2ff5a25e003aa29e790ea6b110f06ba848f0afd7901f3f8ad4740f782ede2` |
| `descriptors.atn` | 859 | Every descriptor value variant and every reference form currently represented by the parser | `6c76502822c1f36c8060a71adb227c4540663518785bbab1b70951dac0ef58ad` |
| `float-bits.atn` | 428 | NaN payload, negative zero, infinities, and floats inside lists, objects and arrays | `0aa6278a569c777f249b305e889896132c2a87e44a24cb1a55d1d3fc0928fd0c` |
| `v12-stop.atn` | 113 | Version 12: Stop with Unicode text and a Boolean | `293462a7d5cf0043fdc5135630524699f545679088c54bad83f4a86e15160271` |
| `v12-values.atn` | 562 | Version 12: named plug-in event, every legacy value and reference form, bare `warp`/`list`/`time` codes, an object array with three objects and two columns, unpaired UTF-16, opaque path/alias/data bytes, and a descriptorless step | `23d3f377996435988367365ee5c2a15462c0ed42ce506dc4a15039387ac6a853` |
| `v12-commands.atn` | 404 | Version 12: Gaussian Blur, Levels, Select All and Make Layer | `451958b686da9aab45e65ae902c86333b7149d72021c21904e3aae8e39eb77cb` |

`tests/it/atn_contract.rs` checks the independently encoded version-16 bytes and expected public
structures; `tests/it/atn_v12.rs` checks the
version-12 files. The version-12 files follow the version 12 layout in `src/atn.rs`, assembled from
the measured grammar in photocraft#10603. Their `Idnt` reference follows the inferred index-like layout; no
real version 12 identifier reference was available. Saving them produces version 16, so keep them
out of the version-16 byte-identical corpus oracle.

The files also seed the `atn` fuzz target: `cargo fuzz run atn crates/psd/tests/fixtures/atn`
(from `crates/psd`: `cargo fuzz run atn tests/fixtures/atn`).

A fixture change should explain the changed wire fields and update this table; do not regenerate
expectations merely to make a failing parser pass.
