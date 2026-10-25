# craft-units

Exact length units for the Craft office apps.

WordCraft, DeckCraft and GridCraft all read and write Office Open XML, which mixes several length
units in one document: English Metric Units (EMU) for DrawingML, twips for WordprocessingML, points
for print and font sizes, and CSS pixels for the web builds. Before this crate each app carried its
own copy of the conversions, in `f32` in one and `f64` in another, so a length could round
differently depending on which app read the file. This crate is the single implementation.

`Length` stores an exact signed integer count of EMU, the finest unit OOXML uses:

| Unit | EMU per unit |
|---|---:|
| inch | 914 400 |
| point | 12 700 |
| twip | 635 |
| centimetre | 360 000 |
| millimetre | 36 000 |
| CSS pixel (96 dpi) | 9 525 |

## Behaviour

- **Exact round trips.** A length read from a file and written back is bit-for-bit unchanged,
  because the stored value is the integer EMU count from the file.
- **Rounding half away from zero.** `0.5 pt` is `10 twips`; `-0.5 pt` is `-10 twips`.
- **Saturating, never wrapping.** Converting an out-of-range value clamps at the `i64` range.
- **Non-finite input is zero.** `NaN` and the infinities convert to zero, matching the behaviour
  the apps already had.
- **No panics on hostile input.** No `unwrap`, no indexing, no arithmetic that can overflow.

## Use

```rust
use craft_units::Length;

let page = Length::from_inches(8.5);
assert_eq!(page.emu(), 7_772_400);
assert_eq!(page.points(), 612.0);
assert_eq!(page.twips(), 12_240);

let parsed = Length::parse("2.5cm");
assert_eq!(parsed, Some(Length::from_centimetres(2.5)));
```

Nothing here decides layout, print policy or document structure; those stay in each app. The crate
owns conversion and representation only.

## Quality gates

```sh
cargo fmt -p craft-units --check
cargo clippy -p craft-units --all-targets -- -D warnings
cargo test -p craft-units
```

The crate follows the repository rules in [`../../AGENTS.md`](../../AGENTS.md): every public item is
documented, the crate denies the panic lints, and `unsafe` is forbidden at the workspace level.
