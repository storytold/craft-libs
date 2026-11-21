# craft-deskew

Scan straightening for any app that handles scanned pages: measure how far a raster's content
is tilted, and rotate it level.

```rust
use craft_deskew::{Image, Layout, Options, Outcome, straighten};

let image = Image::new(Layout::grey8(width, height), &samples)?;
match straighten(&image, &Options::default()) {
    Outcome::Straightened { skew, samples, .. } => { /* re-encode `samples` */ }
    Outcome::AlreadyStraight(_) => {}
    Outcome::Declined(why) => { /* tell the user it couldn't tell */ }
    _ => {}
}
```

## What it does

- `estimate_skew` measures within ±15° to 0.05°, with a confidence from 0 to 1. It projects
  ink pixels at each angle and keeps the most sharply peaked profile; text lines, rules and
  title blocks all work, with no OCR. A dark frame touching the edge (a scanner border) is
  ignored. A page too sparse or too dark to project falls back to a vote by its edges.
- `rotate` turns the content about the centre into an image of the same size and layout.
  Uncovered corners take the most common border colour. 8- and 16-bit images are resampled
  bilinearly; 1-, 2-, 4-bit and indexed images by nearest neighbour, so they never gain values
  they did not have.
- `straighten` measures, rotates and measures again, and returns one of three outcomes:
  straightened, already straight, or declined with the reason (nothing to measure, not
  confident, or the correction did not help). It never reports "can't tell" as "straight".

Angles are degrees, positive counter-clockwise as displayed.

## Input

`Layout` describes the samples as a PDF image stores them: 1, 2, 4, 8 or 16 bits, 1 to 32
interleaved components, rows padded to a byte, most significant bit first, which value is
paper (`Paper::Max` for grey and RGB, `Paper::Zero` for CMYK or an inverted decode), and
whether the samples are palette indices (`Layout::new`, then `as_indexed`). Images over 2^28
pixels are refused. Measuring is bounded by a 1600-pixel grid and 200,000 scored points;
rotating allocates one output buffer the size of the input. Nothing panics.

## Encoding the result

The crate returns samples; the app encodes them. To keep the file size, stay in the source's
codec family: a JPEG scan goes back to JPEG at a matching quality, a 1-bit scan stays 1-bit
(CCITT G4 or JBIG2 where available, else Flate on the packed bits), and 8-bit Flate data uses a
PNG predictor. Re-encoding a JPEG scan as plain Flate makes it several times larger.
