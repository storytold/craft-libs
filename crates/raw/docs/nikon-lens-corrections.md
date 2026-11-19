# Nikon embedded distortion corrections

Nikon writes its own distortion correction into NEFs from its mirrorless bodies, in the raw SubIFD tag `0xc7d5`
(NikonNEFInfo). LightCraft reads it for the NIKON Z 6_2 (Z 6II), the NIKON Z 8 and the NIKON Z f and turns it into an
`OpcodeList3` `WarpRectilinear`, so it goes through the same profile correction control, optics path, orientation
handling and DNG export as the other embedded corrections. Other bodies are left alone for now even if they have the
same tag, because I've only checked files from these three.

This is distortion only. The vignetting block (`0x0006`) and what looks like lateral CA (`0x0007`) aren't applied
yet. Like the Sony, Fujifilm and Panasonic corrections it's on by default and can be turned off under Lens
Corrections.

## Where the data comes from

The [ExifTool Nikon tag names](https://exiftool.org/TagNames/Nikon.html) give the names of NikonNEFInfo and its
DistortionInfo entry (`0x0005`): DistortionCorrection and RadialDistortionCoefficient1 to 3. Nothing else was taken
from other software. The rest was worked out by comparing LightCraft's uncorrected render of each NEF with the
full-size JPEG the camera embeds in it. That JPEG is always distortion-corrected with Z lenses ("On (Required)"), and
with other lenses when Auto distortion control is on. The comparison matched textured patches between the two images
(normalised cross-correlation on the green channel at quarter size).

NikonNEFInfo starts with `"Nikon\0"` and a 4-byte version, followed by a small TIFF stream. The two entries used here:

| Entry | Bytes | What it holds |
|---|---|---|
| `0x0005` DistortionInfo | 0–3 | `"0100"` |
| | 4 | correction mode: 1 "On (Optional)", 3 "On (Required)", 0 none |
| | 8–15 | normalisation radius in mm, a signed rational. It's 21.6 (the FX half diagonal) in every file I looked at |
| | 16–19 | number of terms, 4 |
| | 20, 28, 36 | the three coefficients, called `k1`, `k2`, `k3` here after ExifTool's names |
| | 64–71 | the zoom `q` the camera uses to fill the frame |
| `0x0008` | 0–3, 4–11, 12–19 | `"0100"`, then the optical center's offset from the center of the image area in pixels (x right, y down) |

The `0x0008` offset is where the in-body stabilization left the sensor. It's zero with a VR lens, since the lens does
the stabilizing, and up to about 100 px with the Z lenses. The center of the distortion measured in the JPEG follows
it closely, for example (−90.1, +18.6) recorded and (−91.2, +17.5) measured.

From an output pixel to the raw, around the optical center, the mapping is

```
rho    = r_out / half diagonal of the image area
r_src  = r_out * (1 + k3*rho^2 + k2*rho^4 + k1*rho^6) / (1 + q)
```

So the stored order goes from the highest power down. Reading them the other way round (`k1` with `rho^2`) doesn't fit
the JPEG at all. `q` really is the zoom: fitting a separate zoom for every file gives `1 / (1 + q)` to four decimals
(0.9675 fitted and 0.9676 from `q` on one of them), so nothing is fitted per file. The decoder turns this into DNG
terms the same way `docs/sony-lens-corrections.md` describes for Sony, with the center in active-area coordinates and
the coefficients rescaled to the farthest corner of the active area.

## How well it matches

I checked it on 83 of my own Z 6II photos with three lenses between 20 and 200 mm, 45 Z 8 photos (below), plus the
CC0 Z f sample in the corpus. The numbers are the radial error against the camera JPEG at full size (6048 px wide), out to 0.95 half
diagonals:

| Lens | Focal length | Frames | Uncorrected median / p95 | Corrected median / p95 |
|---|---:|---:|---:|---:|
| VR 70-200mm f/2.8G (F-mount, FTZ) | 70 mm | 11 | 10.9 / 15.9 px | 0.19 / 0.57 px |
| VR 70-200mm f/2.8G (F-mount, FTZ) | 105–140 mm | 15 | 34–42 / 38–46 px | 0.13–0.18 / 0.59–0.90 px |
| VR 70-200mm f/2.8G (F-mount, FTZ) | 175–200 mm | 21 | 45–46 / 50–51 px | 0.10–0.18 / 0.48–0.95 px |
| NIKKOR Z 24-70mm f/4 S | 70 mm | 8 | 126.4 / 141.2 px | 0.48 / 4.38 px |
| NIKKOR Z 24-70mm f/4 S | 38–59 mm | 6 | 59–101 / 69–121 px | 0.36–0.76 / 4.6–6.8 px |
| NIKKOR Z 24-70mm f/4 S | 33–34 mm | 4 | 32–44 / 43–53 px | 1.0–1.3 / 8.5–9.6 px |
| NIKKOR Z 24-70mm f/4 S | 24 mm | 7 | 16.0 / 105.6 px | 6.90 / 23.56 px |
| NIKKOR Z 20mm f/1.8 S | 20 mm | 5 | 8.2 / 53.9 px | 1.49 / 5.46 px |
| Tamron 35-150mm (Z f, CC0 corpus) | 150 mm | 1 | 70.4 px median | 0.22 px median |

On the Z 8 (8256 px wide JPEGs) it looks the same. The 70-200 f/2.8G on the FTZ matches within 0.14 to 0.21 px median
at 70, 135 and 200 mm. The NIKKOR Z 24-120mm f/4 S is within 0.6 to 1.0 px median from 36 to 120 mm and 1.5 px at 24
to 28 mm, with the same kind of error toward the edges at the wide end as the Z lenses on the Z 6II. A few 14-24mm
f/2.8G frames had Auto distortion control off, so their JPEGs aren't corrected and match about the same either way.

I also rendered a few with `lightcraft-cli render` at default settings and measured those against the camera JPEG.
The median error was 0.10 px for the 70-200 at 200 mm and 0.12 px for the Z f sample. The 24-70 was 0.32 px at 70 mm,
on a frame with a 92 px sensor shift, and 8.4 px at 24 mm. The 20 mm came out at 1.3 px. On the Z 8 it was 0.14 px for the 70-200 at 200 mm, 0.45 and 0.68 px for the 24-120
at 68 and 120 mm, 1.8 px for the Z 20 mm and 3.7 px for the 24-120 at 24 mm. That checks the
center, crop and DNG conversion along with the model itself.

The F-mount lens on the FTZ matches about as well as I can measure. The native Z lenses still have a smooth error
toward the edges that doesn't depend on the sensor shift and is the same in every frame at a given focal length: about
+22 px at 0.7 to 0.9 half diagonals at 24 mm, +8 px at 33 mm and +3.5 px at 70 mm. It seems the camera applies an
extra correction for Z lenses that isn't in the readable coefficients. NikonNEFInfo `0x000e` looks encrypted and I
didn't try to decode it. Even so, the readable correction takes most of the distortion out. At 24 mm the error at 0.7
half diagonals goes from −82 px to +25 px, and at 70 mm from about −137 px to under 4 px.

## What's left out

Only `NIKON Z 6_2`, `NIKON Z 8` and `NIKON Z f` are corrected, matched exactly. A matching tag on another body doesn't tell us its
pixel pitch or image area, so other bodies can be added once someone checks their files. Only the full-frame 3:2
image area is corrected, because the stored 21.6 mm radius is the FX half diagonal, so DX and other crops are skipped.
The decoder also skips data it doesn't recognise (another layout, radius or term count, or no correction mode),
values that are clearly wrong (coefficients past ±1, a zoom past ±0.5, a sensor shift over 5% of the frame) and any
mapping that folds back on itself before the farthest corner.

The tests in `nef_lens` cover the polynomial and zoom, the sensor-shift center and refusing other bodies, layouts
and crops. `nikon_in_file_distortion_becomes_a_warp` runs a synthetic Z 6II NEF through the header and full decode
and a DNG export. `nikon_in_file_distortion_is_limited_to_validated_bodies` checks the corpus: the Z f sample gets the
warp and the D5100, D7000, D7500 and Coolpix B700 samples don't.

None of my photos, crops or reference images are committed, only the numbers above.
