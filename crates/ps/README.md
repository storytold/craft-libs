# craft-ps

PostScript and EPS for the Craft apps. No feature is enabled by default.

| Feature | Provides |
| --- | --- |
| `read` | An EPS interpreter: runs the program and returns what it paints as a one-page PDF |

Writing PostScript and EPS (craft-libs #115) is planned as a separate feature next to `read`.

## Reading EPS

`read::eps_to_pdf(bytes, limits)` runs an EPS (plain, or DOS EPS with a binary header) and
returns a one-page PDF 1.4 the size of its bounding box: vectors stay vectors, CMYK and spot
colours stay as they are, so an app can draw and export the result as a placed PDF.
`read::cached_pdf` converts with the default limits and remembers the result by content.
`is_eps`, `bounding_box` and `postscript_section` read the file without running it.

It is written from the PostScript Language Reference (third edition), the EPS specification,
the Type 1 Font Format and Adobe Technical Notes 5012 (Type 42), 5176 (CFF) and 5177 (Type 2
charstrings); no GPL code was used. It covers what EPS files use:

- the language: Level 3 operators, an explicit execution stack (no Rust recursion for
  PostScript control flow), `stopped`, resources, `eexec`;
- graphics: paths, painting, clipping, `strokepath`, user paths and forms;
- colour: device spaces, Separation and DeviceN, Indexed, ICCBased, Lab;
- images, with filters decoded as they are read (ASCIIHex, ASCII85, RunLength, LZW, Flate with
  predictors, SubFile, ReusableStream; JPEG is passed through);
- smooth shadings (types 1–7, meshes included), shading and tiling patterns;
- text in Type 1, Type 3, Type 42 and CFF fonts (`StartData`), CIDFonts and composite fonts
  through CMaps, horizontal and vertical.

Not covered: Level 2 binary tokens, CIDFontType 0 with Type 1 charstrings, CIDFontType 1 and
the old composite FMapTypes other than 2 and 9.

## Limits

`Limits { steps, memory }` bounds one conversion. Every operator charges the work it does in
proportion to its operands (elements copied or searched, path segments, glyphs, samples)
against `steps` as it goes, and every byte read or decoded at any filter level counts too, so
the step limit bounds run time. `memory` bounds everything a conversion makes, copies
included. The defaults are 50 million steps and 1 GiB, or 20 million and 256 MiB on wasm32;
real EPS files use at most a few million steps and tens of megabytes, and a program that runs
into the limits ends within seconds. A program that fails or exceeds them before painting
anything gives `Err`; one that fails later keeps what it painted (with a warning).

`cargo run -p craft-ps --features read --example eps2pdf -- in.eps out.pdf` converts one file
and prints the steps and memory it used.
