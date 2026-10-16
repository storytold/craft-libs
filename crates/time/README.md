# Craft Time

`craft-time` provides exact, drift-free media time representation, rational frame-rate mathematics, SMPTE timecode formatting/parsing, and film footage calculations for the Craft suite.

Extracted from the common time engine in **FilmCraft** and **EffectCraft** (refs #2), it is designed for NLEs, motion graphics compositors, audio workstations, and media transcoders.

## Core Concepts

### Exact Ticks (`TICKS_PER_SECOND`)

All media time is tracked as an integer number of clock ticks at:

$$\text{TICKS\_PER\_SECOND} = 254{,}016{,}000{,}000 \text{ ticks/s}$$

This rate was chosen because it divides evenly into every broadcast and cinematic frame rate, as well as every standard professional audio sample rate:

- **Frame Rates**: 23.976, 24, 25, 29.97, 30, 48, 50, 59.94, 60, 119.88, 120 fps.
- **Audio Sample Rates**: 32 kHz, 44.1 kHz, 48 kHz, 88.2 kHz, 96 kHz, 192 kHz.

Edits, clip slicing, frame stepping, and audio/video alignment never suffer cumulative floating-point drift.

```rust
use craft_time::{Tick, TICKS_PER_SECOND};

// Convert from seconds or sample units
let one_sec = Tick::from_seconds_f64(1.0);
assert_eq!(one_sec.0, TICKS_PER_SECOND);

let audio_sample = Tick::from_units(48000, 48000);
assert_eq!(audio_sample, one_sec);
```

### Rational Frame Rates (`FrameRate`)

Frame rates are represented as exact fractions (`num / den`):

```rust
use craft_time::FrameRate;

// Standard NTSC and PAL rates
let ntsc_24 = FrameRate::FPS_23_976; // 24000 / 1001
let pal_25 = FrameRate::FPS_25;      // 25 / 1
let ntsc_30 = FrameRate::FPS_29_97;  // 30000 / 1001

// Start tick of frame 100
let start_tick = ntsc_30.tick_of(100);

// Frame containing a tick (floored)
let frame = ntsc_30.frame_at(start_tick);
assert_eq!(frame, 100);

// Snap to exact frame boundaries
let snapped = ntsc_30.snap(start_tick + Tick(5));
assert_eq!(snapped, start_tick);
```

### SMPTE Timecode (`format_timecode_frames` / `parse_timecode`)

Full support for SMPTE standard drop-frame (semicolon `;` separator) and non-drop frame (colon `:` separator) timecode:

```rust
use craft_time::{FrameRate, format_timecode_frames, parse_timecode};

let r = FrameRate::FPS_29_97;

// Format frame count to drop-frame timecode
let tc = format_timecode_frames(1800, r, true);
assert_eq!(tc, "00;01;00;02"); // Frames 00 and 01 dropped at minute mark

// Parse user-entered timecode (supports relative offsets like +15 or -1.00)
let parsed = parse_timecode("00;01;00;02", r, true, 0).unwrap();
assert_eq!(parsed, 1800);
```

### Film Footage (`format_feet_frames` / `parse_feet_frames`)

Calculates 35mm (16 frames/foot) and 16mm (40 frames/foot) film footage counts:

```rust
use craft_time::{format_feet_frames, parse_feet_frames};

// 35mm (16 frames per foot)
assert_eq!(format_feet_frames(199, 16), "0012+07");
assert_eq!(parse_feet_frames("12+07", 16, 0), Ok(199));

// 16mm (40 frames per foot)
assert_eq!(format_feet_frames(95, 40), "0002+15");
assert_eq!(parse_feet_frames("2+15", 40, 0), Ok(95));
```

## EffectCraft & FilmCraft Compatibility Differences

`craft-time` unifies time representation across the Craft suite while explicitly documenting application-specific design choices:

1. **After Effects Display Formatting**:
   FilmCraft uses standard broadcast SMPTE zero-padded hours (`00:00:02:15` via `format_timecode_frames`).
   EffectCraft / After Effects displays hours without leading zeroes (`0:00:02:15` via `format_timecode_ae`). Both functions are provided.
2. **Oversized Timecode Saturation**:
   In `craft-time`, `parse_timecode` employs saturating arithmetic (`saturating_mul`, `saturating_add`) when frame calculations exceed 64-bit limits, saturating to `i64::MAX` or `i64::MIN` to keep operations total without unrecoverable errors. In contrast, EffectCraft returns `Err(ParseError("timecode too large"))`.
3. **Degenerate `{0, 0}` Frame Rates**:
   In `craft-time`, `FrameRate::sane()` normalizes non-positive or zero rates (such as `{ num: 0, den: 0 }` or negative denominators) to `FrameRate::FPS_23_976` (nominal timecode base 24), maintaining standard broadcast defaults. In contrast, EffectCraft falls back to a timecode base of 1.
4. **Monotonic Operator Division**:
   Operator division on `Tick` (`Tick / i64`) and `Tick::saturating_div` saturate on overflow (`Tick(i64::MIN) / -1 -> Tick(i64::MAX)`), preserving strict mathematical monotonicity across the entire signed 64-bit range without inflection.

## Guarantees & Doctrines

- **Zero App Code**: Zero dependencies on application crates.
- **Never Crash**: `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`.
- **Total Arithmetic**: Division by zero or hostile rates from damaged projects return well-defined fallback states rather than panicking. Arithmetic saturates at 64-bit limits with 128-bit intermediate checks.
- **Forbid Unsafe**: `#![forbid(unsafe_code)]`.
