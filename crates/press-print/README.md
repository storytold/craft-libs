# craft-press-print

Press-accurate printing for the Craft apps. A page prints as the press would print it: overprint
decided per plate, spot inks as inks rather than their alternates, knockout and isolated groups,
and the press's colors, converted once into the printer's RGB profile.

It is an opt-in alternative to craft-print's PDF path, which draws a job with the system's PDF
renderer. That renderer does not simulate overprint and draws spot and DeviceN colors through their
alternates.

## How a page prints

1. craft-prepress separates the page in bands (`plates_in_bands`), one 8-bit plate per ink.
2. Each band composites into the printer's profile (`Plates::composite_to`). The process plates
   convert from the printing condition they were separated through (`press_profile`: the page's
   output intent, else `Options::fallback`), straight from CMYK to the printer. Spot plates
   combine over them.
3. The bands join into one RGB image of the page.
4. `craft_print::rgb::print` sends it with `ColorPolicy::ApplicationManaged`, so the driver
   applies no second conversion.

```rust
use craft_cms::Profile;
use craft_press_print::{Settings, print, render};
use craft_print::rgb;

let settings = Settings::new(Profile::parse(&printer_icc)?);
// The page as it prints, to preview or to keep.
let page = render(&pdf, 0, 600.0, &settings)?;
// Or straight to a printer, at its resolution.
let job = rgb::Job {
    printer: "Office printer",
    settings: None,
    title: "proof.pdf",
    output_file: None,
    copies: 1,
    color_policy: rgb::ColorPolicy::ApplicationManaged,
    primitives: &[],
};
print(&job, &pdf, 0, &settings, (0.0, 0.0, 612.0, 792.0))?;
```

`Settings` holds the separation options, the printer's RGB profile (sRGB when the printer's own
profile is not known), the rendering intent and black point compensation. A gray or CMYK printer
profile is refused: `rgb::print` takes RGB.

## Limits

- `rgb::print` takes one image per job, so `print` sends one page per job.
- `print` separates at the printer's resolution, whatever the size of `rect`, within
  craft-prepress's limits: the renderer's 8192-pixel side limit across the page (about 960 dpi on
  Letter or A4) and `MAX_BANDED_PIXELS` in all. The page image is 3 bytes per pixel and is held
  whole. `rgb::print` takes at most 512 MiB, so a page past that separates again at the
  resolution that fits.
- Each band renders the page once per ink, so time grows with resolution and ink count.
- Printing needs Windows. Elsewhere `print` returns `rgb::PrintError::Unsupported`, and `render`
  still works.
