//! The plates as they print, in a chosen output profile: process inks convert from the press's
//! CMYK straight to the output (CMYK → PCS → output, not through sRGB), and spot inks combine
//! in the output space.

use std::sync::Arc;

use craft_cms::{Builtin, ColorSpace, Intent, Profile, Transform, TransformOptions, cached};

use crate::cms::encode_lab;
use crate::composite::{cmyk_ramp, lab_ramp, on};
use crate::coverage::filled;
use crate::{Alternate, InkId, Plates, Process};

/// Pixels converted per call, to bound the scratch buffer.
const CHUNK: usize = 16 * 1024;

/// Why an [`Output`] can't be built.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum OutputError {
    #[error("the press profile is not CMYK")]
    PressNotCmyk,
    #[error("the output profile is not gray, RGB or CMYK")]
    UnsupportedOutput,
    #[error("craft-cms can't convert into the output profile: {0}")]
    Transform(String),
}

/// A conversion from a press (the CMYK printing condition the plates were made for) to an
/// output profile, such as a printer's. Built once, used for every composite.
pub struct Output {
    process: Arc<Transform>,
    from_lab: Arc<Transform>,
    channels: usize,
    subtractive: bool,
    /// The press's paper in the output: what no ink at all converts to, and the baseline every
    /// spot's appearance is taken relative to.
    paper: [u8; 4],
    /// The press's paper as Lab: where a spot's Lab tint ramp starts.
    paper_lab: [f32; 3],
}

impl Output {
    /// Press CMYK into `output` (gray, RGB or CMYK) at `intent`, with black point compensation
    /// when `bpc` (craft-cms ignores it for absolute colorimetric). Relative colorimetric with
    /// `bpc` maps the press's paper and black onto the output's; absolute colorimetric keeps
    /// the press paper's tint. That paper is the baseline a spot's appearance is taken relative
    /// to (see [`Plates::composite_to`]).
    pub fn new(press: &Profile, output: &Profile, intent: Intent, bpc: bool) -> Result<Self, OutputError> {
        if press.color_space != ColorSpace::Cmyk {
            return Err(OutputError::PressNotCmyk);
        }
        let subtractive = match output.color_space {
            ColorSpace::Gray | ColorSpace::Rgb => false,
            ColorSpace::Cmyk => true,
            _ => return Err(OutputError::UnsupportedOutput),
        };
        // The fast path's tables: a composite converts every pixel of a page.
        let opts = TransformOptions { intent, bpc, precise_float: false };
        let failed = |e: craft_cms::CmsError| OutputError::Transform(format!("{e:?}"));
        let channels = output.channels();
        let press_to_lab = cached(press, Builtin::LabD50.profile(), opts).map_err(failed)?;
        let process = cached(press, output, opts).map_err(failed)?;
        let from_lab = cached(Builtin::LabD50.profile(), output, opts).map_err(failed)?;
        // The press's paper: what the process plates print where no ink reaches. A spot's
        // appearance is taken relative to it, so a spot with no ink adds nothing even under
        // absolute colorimetric, which keeps the paper's own tint.
        let mut paper = [0_u8; 4];
        process.convert_u8(&[0; 4], 4, &mut paper[..channels], channels, false).map_err(failed)?;
        // Where a spot's Lab tint ramp starts: white (L* 100), which every intent that maps white
        // to white also prints the paper as, and the press paper's own colour under absolute
        // colorimetric. The press gives that colour through its device-to-PCS tables, the ones
        // the conversion above needs anyway, so an output that converts still does whatever its
        // own tables hold, and the ramp's start is not tied to what the output can print.
        let paper_lab = if intent == Intent::AbsoluteColorimetric {
            let mut lab = [0.0_f32; 3];
            press_to_lab.convert_f32(&[0.0; 4], 4, &mut lab, 3, false).map_err(failed)?;
            [lab[0] * 100.0, lab[1] * 255.0 - 128.0, lab[2] * 255.0 - 128.0]
        } else {
            [100.0, 0.0, 0.0]
        };
        Ok(Self { process, from_lab, channels, subtractive, paper, paper_lab })
    }

    /// Bytes per pixel of the output: 1 gray, 3 RGB, 4 CMYK.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// The spot's appearance over the paper at each tint byte, `channels` bytes per tint: as
    /// [`appearance`](crate::composite::appearance) has it, taken relative to the paper's own
    /// appearance; `None` when it can't convert.
    fn ramp(&self, alternate: Alternate) -> Option<Vec<u8>> {
        let mut out = filled(256 * self.channels, 0_u8)?;
        match alternate {
            Alternate::Lab(lab) => {
                let encoded: Vec<f32> = lab_ramp(self.paper_lab, lab).as_chunks::<3>().0.iter().flat_map(|c| encode_lab(*c)).collect();
                let mut values = filled(out.len(), 0.0_f32)?;
                self.from_lab.convert_f32(&encoded, 3, &mut values, self.channels, false).ok()?;
                for (byte, v) in out.iter_mut().zip(values) {
                    *byte = if v.is_finite() { (v.clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 };
                }
            }
            Alternate::Cmyk(solid) => self.process.convert_u8(&cmyk_ramp(solid), 4, &mut out, self.channels, false).ok()?,
        }
        // The pixels already carry the paper's appearance: the process plates converted it, and
        // absolute colorimetric keeps its tint. What a spot adds is its appearance *over* the
        // paper, so each step of the ramp is taken relative to the paper's.
        for (c, byte) in out.iter_mut().enumerate() {
            *byte = self.over_paper(*byte, self.paper[c % self.channels]);
        }
        Some(out)
    }

    /// Appearance `value` relative to the paper's `paper` in one channel: the light an additive
    /// output shows, or the ink a subtractive one prints, for an ink over paper. Clamped, so an
    /// ink never brightens a pixel and never takes ink off it, and exact: an appearance equal to
    /// the paper's is the paper, whatever the paper is. An appearance lighter than the paper's,
    /// which no ink is, therefore prints as nothing.
    fn over_paper(&self, value: u8, paper: u8) -> u8 {
        let (value, paper) = (u32::from(value), u32::from(paper));
        if self.subtractive {
            // Inks add: (255 - v)/255 and (255 - p)/255 are the ink and the paper's own ink as
            // transmittances, and their ratio is the ink's over paper. A paper at full ink (a
            // zero denominator) leaves nothing to add to.
            let (ink, bare) = (255 - value, 255 - paper);
            let transmittance = (ink * 255 + bare / 2).checked_div(bare).unwrap_or(255).min(255);
            (255 - transmittance) as u8
        } else {
            // Light multiplies, so the ink's share is its light over the paper's. A paper in
            // darkness (a zero denominator) has no light to take away.
            (value * 255 + paper / 2).checked_div(paper).unwrap_or(255).min(255) as u8
        }
    }

    /// `ink` printed over `px`: a multiply of an additive output's values, and of a
    /// subtractive one's complements (ISO 32000-2 §11.3.4), so inks add.
    fn combine(&self, px: &mut [u8], ink: &[u8]) {
        for (c, s) in px.iter_mut().zip(ink) {
            let (c16, s16) = (u16::from(*c), u16::from(*s));
            *c = if self.subtractive { (255 - ((255 - c16) * (255 - s16) + 127) / 255) as u8 } else { ((c16 * s16 + 127) / 255) as u8 };
        }
    }
}

impl Plates {
    /// The enabled inks as they print, in `output`'s space ([`Output::channels`] bytes per
    /// pixel, row-major, `width × height` pixels). `enabled` is as for
    /// [`Plates::composite`].
    ///
    /// The process plates convert from the press profile `output` was built with, directly.
    /// Each spot plate then combines over them with the spot's appearance at its tint, from its
    /// alternate, converted into the output and taken relative to the press paper's appearance,
    /// which the process plates already print: a spot whose alternate is no ink adds nothing, and
    /// a spot prints the colour its alternate prints as process ink rather than the paper tinted
    /// by it, within a channel's 8-bit rounding and with an appearance lighter than the paper's
    /// clamped to the paper ([`Output`]'s `over_paper`). A `Lab` alternate's ramp runs from the
    /// paper to its solid. Spots whose alternate can't convert are left out.
    /// The overprint decisions are the plates'. Empty when the buffer overflows, can't be
    /// allocated, or craft-cms can't convert the process inks.
    pub fn composite_to(&self, enabled: &[bool], output: &Output) -> Vec<u8> {
        let n = output.channels;
        let Some(mut out) = self.pixels().and_then(|p| p.checked_mul(n)).and_then(|len| filled(len, 0_u8)) else {
            return Vec::new();
        };
        let process: Vec<Option<&[u8]>> = Process::ALL
            .iter()
            .map(|p| {
                let id = InkId::process(*p);
                self.plane(id).filter(|_| on(enabled, id.index()))
            })
            .collect();
        let mut cmyk = Vec::with_capacity(CHUNK * 4);
        for (c, chunk) in out.chunks_mut(CHUNK * n).enumerate() {
            let start = c * CHUNK;
            cmyk.clear();
            for i in start..start + chunk.len() / n {
                cmyk.extend(process.iter().map(|plane| plane.and_then(|p| p.get(i)).copied().unwrap_or(0)));
            }
            if output.process.convert_u8(&cmyk, 4, chunk, n, false).is_err() {
                return Vec::new();
            }
        }

        for (id, spot) in self.inks.spots().filter(|(id, _)| on(enabled, id.index())) {
            let Some(plane) = self.plane(id).filter(|p| p.iter().any(|t| *t > 0)) else { continue };
            let Some(ramp) = output.ramp(spot.alternate) else { continue };
            for (px, tint) in out.chunks_exact_mut(n).zip(plane) {
                let at = usize::from(*tint) * n;
                if let Some(ink) = ramp.get(at..at + n).filter(|_| *tint > 0) {
                    output.combine(px, ink);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use craft_cms::{Builtin, Intent, Transform, TransformOptions, builtin};

    use super::{Output, OutputError};
    use crate::cms::srgb8_to_lab;
    use crate::coverage::tests::synthetic;
    use crate::{Alternate, CmsConversion, Conversion, Ink, InkSet, Plates, Spot};

    fn fogra39() -> &'static craft_cms::Profile {
        Builtin::CoatedFogra39.profile()
    }

    fn near(got: &[u8], expected: &[u8], tolerance: u8) -> bool {
        got.len() == expected.len() && got.iter().zip(expected).all(|(g, e)| g.abs_diff(*e) <= tolerance)
    }

    /// A spot ink `name` whose alternate is `alternate`.
    fn spot(name: &str, alternate: Alternate) -> Ink {
        Ink::Spot(Spot { name: name.into(), alternate, solidity: None })
    }

    /// A one-row page for `inks`, from their planes in [`InkSet::iter`] order.
    fn page(inks: InkSet, planes: Vec<Vec<u8>>) -> Plates {
        let width = planes.first().map_or(0, Vec::len) as u32;
        Plates { inks, unseparated: Vec::new(), width, height: 1, dpi: 72.0, conversion: Conversion::DeviceFormulas, planes }
    }

    /// The four process planes, `cmyk` at each of `pixels` pixels.
    fn process_planes(cmyk: [u8; 4], pixels: usize) -> Vec<Vec<u8>> {
        cmyk.iter().map(|c| vec![*c; pixels]).collect()
    }

    /// Coated FOGRA39's tables on a darker paper. ICC-absolute colorimetry takes a paper's
    /// colour from the profile's media white point, so a press whose paper is darker than
    /// FOGRA39's is that profile with a darker `wtpt`; its paper then prints as ink (or as a
    /// darker gray) on the output, which is what makes a repeated baseline visible.
    fn darker_paper_press(scale: f64) -> craft_cms::Profile {
        let mut press = fogra39().clone();
        press.white_point = press.white_point.map(|v| v * scale);
        press.with_encoded_bytes()
    }

    #[test]
    fn a_cmyk_output_of_the_press_profile_keeps_the_press_inks() {
        // Cyan, red and a light blue come back within the profile's own round trip. Through
        // sRGB, cyan is out of gamut and comes back as [201, 59, 0, 0]. (Black is left out: the
        // output's BToA tables generate their own black, as they would for a printer.)
        let p = synthetic(3, 1, [70.0, 5.0, 60.0], [vec![255, 0, 64], vec![0, 255, 32], vec![0, 255, 0], vec![0; 3], vec![0; 3]]);
        let output = Output::new(fogra39(), fogra39(), Intent::RelativeColorimetric, false).unwrap();
        assert_eq!(output.channels(), 4);
        let cmyk = p.composite_to(&[], &output);
        assert!(near(&cmyk[0..4], &[255, 0, 0, 0], 12), "cyan: {:?}", &cmyk[0..4]);
        assert!(near(&cmyk[4..8], &[0, 255, 255, 0], 12), "red: {:?}", &cmyk[4..8]);
        assert!(near(&cmyk[8..12], &[64, 32, 0, 0], 12), "light blue: {:?}", &cmyk[8..12]);
    }

    #[test]
    fn an_srgb_output_matches_the_preview_composite() {
        const GOLD: [u8; 3] = [212, 175, 55];
        let lab = srgb8_to_lab(GOLD).unwrap();
        // Black alone, Gold alone, Gold over black, Gold at 50%.
        let p = synthetic(4, 1, lab, [vec![0; 4], vec![0; 4], vec![0; 4], vec![255, 0, 255, 0], vec![0, 255, 255, 128]]);
        let output = Output::new(fogra39(), builtin::srgb(), Intent::RelativeColorimetric, true).unwrap();
        let rgb = p.composite_to(&[], &output);
        let preview = p.composite(&[], &CmsConversion::fogra39());
        assert!(near(&rgb, &preview, 2), "{rgb:?} {preview:?}");
        assert!(near(&p.composite_to(&[true, true, true, false], &output)[0..3], &[255, 255, 255], 1), "black hidden");
    }

    #[test]
    fn a_spot_adds_its_ink_in_a_cmyk_output() {
        let mut inks = InkSet::new();
        inks.add(Ink::Spot(Spot { name: "Rubine".into(), alternate: Alternate::Cmyk([0.0, 1.0, 0.0, 0.0]), solidity: None }));
        // Cyan with the spot over it, and the spot alone.
        let planes = vec![vec![255, 0], vec![0; 2], vec![0; 2], vec![0; 2], vec![255, 255]];
        let p = Plates { inks, unseparated: Vec::new(), width: 2, height: 1, dpi: 72.0, conversion: Conversion::DeviceFormulas, planes };
        let output = Output::new(fogra39(), fogra39(), Intent::RelativeColorimetric, false).unwrap();
        let cmyk = p.composite_to(&[], &output);
        // Magenta's round trip through the profile adds a little yellow.
        assert!(near(&cmyk[0..4], &[255, 255, 8, 0], 8), "spot over cyan: {:?}", &cmyk[0..4]);
        assert!(near(&cmyk[4..8], &[0, 255, 8, 0], 8), "spot alone: {:?}", &cmyk[4..8]);
        assert!(near(&p.composite_to(&[true, true, true, true, false], &output)[0..4], &[255, 0, 0, 0], 6), "spot hidden");
    }

    #[test]
    fn outputs_need_a_cmyk_press_and_a_gray_rgb_or_cmyk_output() {
        let lab = Builtin::LabD50.profile();
        let srgb = builtin::srgb();
        assert_eq!(Output::new(srgb, srgb, Intent::RelativeColorimetric, true).err(), Some(OutputError::PressNotCmyk));
        assert_eq!(Output::new(fogra39(), lab, Intent::RelativeColorimetric, true).err(), Some(OutputError::UnsupportedOutput));
        let gray = Output::new(fogra39(), builtin::linear_gray(), Intent::RelativeColorimetric, true).unwrap();
        let p = synthetic(2, 1, [70.0, 5.0, 60.0], [vec![0; 2], vec![0; 2], vec![0; 2], vec![0, 255], vec![0; 2]]);
        let g = p.composite_to(&[], &gray);
        assert!(g.len() == 2 && g[0] > 250 && g[1] < 10, "paper and black in gray: {g:?}");
    }
    /// The reported regression: absolute colorimetric keeps the press paper's tint on the page,
    /// and a spot whose alternate is no ink must not print that tint a second time. Relative
    /// colorimetric is the control: its paper is the output's white, so it never showed.
    #[test]
    fn a_spot_with_no_ink_adds_nothing() {
        for dst in [builtin::srgb(), builtin::linear_gray()] {
            for intent in [Intent::RelativeColorimetric, Intent::AbsoluteColorimetric] {
                let mut inks = InkSet::new();
                inks.add(spot("Clear", Alternate::Cmyk([0.0; 4])));
                let mut planes = process_planes([0; 4], 2);
                planes.push(vec![0, 255]);
                let got = page(inks, planes).composite_to(&[], &Output::new(fogra39(), dst, intent, false).unwrap());
                let n = got.len() / 2;
                assert_eq!(&got[..n], &got[n..], "{} {}: {got:?}", dst.description, intent.id());
                if intent == Intent::AbsoluteColorimetric && dst.color_space == craft_cms::ColorSpace::Rgb {
                    assert!(near(&got[..3], &[239, 241, 244], 1), "the press paper's tint: {:?}", &got[..3]);
                }
            }
        }
    }

    /// Two more shapes of the same rule: the paper is printed once however many no-ink spots a
    /// page has, and a no-ink spot over a real one changes nothing.
    #[test]
    fn no_ink_spots_do_not_repeat_the_paper() {
        let output = Output::new(fogra39(), builtin::srgb(), Intent::AbsoluteColorimetric, false).unwrap();
        let mut inks = InkSet::new();
        for name in ["Clear", "Varnish", "Primer"] {
            inks.add(spot(name, Alternate::Cmyk([0.0; 4])));
        }
        let mut planes = process_planes([0; 4], 1);
        planes.extend([vec![255], vec![255], vec![255]]);
        let paper = page(InkSet::new(), process_planes([0; 4], 1)).composite_to(&[], &output);
        assert_eq!(page(inks, planes).composite_to(&[], &output), paper, "three no-ink spots over the paper");

        let rubine = Alternate::Cmyk([0.0, 1.0, 0.0, 0.0]);
        let mut both = InkSet::new();
        both.add(spot("Rubine", rubine));
        both.add(spot("Clear", Alternate::Cmyk([0.0; 4])));
        let mut planes = process_planes([0; 4], 1);
        planes.extend([vec![255], vec![255]]);
        let mut alone = InkSet::new();
        alone.add(spot("Rubine", rubine));
        let mut alone_planes = process_planes([0; 4], 1);
        alone_planes.push(vec![255]);
        assert_eq!(
            page(both, planes).composite_to(&[], &output),
            page(alone, alone_planes).composite_to(&[], &output),
            "Rubine at 100% with a no-ink spot over it"
        );
    }

    /// A spot at 100% prints what its alternate prints as process ink: the paper goes under it
    /// once, not once for the page and again for the spot. The press's paper is darker than the
    /// output's here, so absolute colorimetric prints it as ink and has a baseline to repeat.
    #[test]
    fn a_spot_prints_its_own_appearance_over_the_paper() {
        let press = darker_paper_press(0.7);
        // The press is a real profile: its bytes parse, and only its paper changed.
        let parsed = craft_cms::Profile::parse(&press.to_bytes()).expect("the darker-paper press is a valid ICC profile");
        assert!(parsed.media_white()[1] < fogra39().media_white()[1], "its paper is the darker one");
        assert!(parsed.same_colors(fogra39()), "the darker-paper press prints FOGRA39's colours");
        for (name, dst, no_ink) in [("sRGB", builtin::srgb(), 255_u8), ("gray", builtin::linear_gray(), 255), ("CMYK", fogra39(), 0)] {
            for intent in [Intent::RelativeColorimetric, Intent::AbsoluteColorimetric] {
                let output = Output::new(&press, dst, intent, false).unwrap();
                let mut inks = InkSet::new();
                inks.add(spot("Rubine", Alternate::Cmyk([0.0, 1.0, 0.0, 0.0])));
                let mut planes = process_planes([0; 4], 1);
                planes.push(vec![255]);
                let printed = page(inks, planes).composite_to(&[], &output);
                let process = page(InkSet::new(), process_planes([0, 255, 0, 0], 1)).composite_to(&[], &output);
                if intent == Intent::AbsoluteColorimetric {
                    assert!(process[0].abs_diff(no_ink) > 10, "{name}: the paper has no tint to repeat: {process:?}");
                }
                assert!(near(&printed, &process, 1), "{name} {}: spot {printed:?} as process {process:?}", intent.id());
            }
        }
    }

    /// A spot's filter is the paper's baseline whatever is under it: over a process colour it
    /// scales that colour by the same ratio it scales the paper by.
    #[test]
    fn a_spots_filter_is_the_same_over_any_background() {
        let output = Output::new(fogra39(), builtin::srgb(), Intent::AbsoluteColorimetric, false).unwrap();
        let rubine = Alternate::Cmyk([0.0, 1.0, 0.0, 0.0]);
        let with_rubine = |cmyk: [u8; 4]| {
            let mut inks = InkSet::new();
            inks.add(spot("Rubine", rubine));
            let mut planes = process_planes(cmyk, 1);
            planes.push(vec![255]);
            page(inks, planes).composite_to(&[], &output)
        };
        let magenta = page(InkSet::new(), process_planes([0, 255, 0, 0], 1)).composite_to(&[], &output);
        let paper = page(InkSet::new(), process_planes([0; 4], 1)).composite_to(&[], &output);
        let (over_magenta, over_paper) = (with_rubine([0, 255, 0, 0]), with_rubine([0; 4]));
        for c in 0..3 {
            // A filter is exact on a background big enough to hold it: an 8-bit channel of 12
            // rounds a 0.5 to 1 and reads 8% instead of 5%.
            if magenta[c] < 24 || paper[c] < 24 {
                continue;
            }
            let ratio = |a: u8, b: u8| f64::from(a) / f64::from(b);
            let (over_a, over_b) = (ratio(over_magenta[c], magenta[c]), ratio(over_paper[c], paper[c]));
            assert!((over_a - over_b).abs() < 0.03, "channel {c}: Rubine over magenta {over_magenta:?}, over paper {over_paper:?}");
        }
    }

    /// A Lab alternate's tint ramp runs from the paper to the solid: a half tint prints the
    /// half-way colour, not the paper tinted by it, which is what applying the paper twice gives.
    #[test]
    fn a_lab_tint_ramp_runs_from_the_paper_to_the_solid() {
        const GOLD: [f32; 3] = [70.0, 5.0, 60.0];
        let srgb = builtin::srgb();
        let output = Output::new(fogra39(), srgb, Intent::AbsoluteColorimetric, false).unwrap();
        let mut inks = InkSet::new();
        inks.add(spot("Gold", Alternate::Lab(GOLD)));
        let mut planes = process_planes([0; 4], 2);
        planes.push(vec![255, 128]);
        let printed = page(inks, planes).composite_to(&[], &output);
        let paper = page(InkSet::new(), process_planes([0; 4], 1)).composite_to(&[], &output);
        // Read colours back in Lab, and the paper's own Lab.
        let to_lab = Transform::with_options(
            srgb,
            Builtin::LabD50.profile(),
            TransformOptions { intent: Intent::AbsoluteColorimetric, bpc: false, precise_float: false },
        )
        .unwrap();
        let lab = |px: &[u8]| {
            let mut rgb = [0.0_f32; 3];
            for (v, b) in rgb.iter_mut().zip(px) {
                *v = f32::from(*b) / 255.0;
            }
            let mut out = [0.0_f32; 3];
            to_lab.convert_f32(&rgb, 3, &mut out, 3, false).unwrap();
            [out[0] * 100.0, out[1] * 255.0 - 128.0, out[2] * 255.0 - 128.0]
        };
        let (paper_lab, solid, half) = (lab(&paper), lab(&printed[..3]), lab(&printed[3..]));
        // The solid is the alternate's own colour; the half tint is half way from the paper to it.
        let midway: [f32; 3] = std::array::from_fn(|i| (paper_lab[i] + GOLD[i]) / 2.0);
        for i in 0..3 {
            assert!((solid[i] - GOLD[i]).abs() < 0.5, "Gold solid {solid:?}, its alternate {GOLD:?}");
            assert!((half[i] - midway[i]).abs() < 0.5, "Gold at 50% {half:?}, half way from {paper_lab:?} to {GOLD:?}");
        }
    }

    /// The ramp's arithmetic at the edges: an appearance equal to the paper's is the paper, a
    /// paper with no light or at full ink leaves nothing to take away, and a spot never brightens
    /// a pixel.
    #[test]
    fn a_spots_appearance_is_taken_relative_to_the_paper() {
        let rgb = Output::new(fogra39(), builtin::srgb(), Intent::AbsoluteColorimetric, false).unwrap();
        assert_eq!(rgb.over_paper(239, 239), 255, "no ink over the paper");
        assert_eq!(rgb.over_paper(255, 239), 255, "an appearance brighter than the paper");
        assert_eq!(rgb.over_paper(0, 239), 0, "black");
        assert_eq!(rgb.over_paper(239, 0), 255, "a paper in darkness");
        let cmyk = Output::new(fogra39(), fogra39(), Intent::AbsoluteColorimetric, false).unwrap();
        assert_eq!(cmyk.over_paper(45, 45), 0, "no ink over the paper");
        assert_eq!(cmyk.over_paper(0, 45), 0, "an appearance with less ink than the paper");
        assert_eq!(cmyk.over_paper(255, 45), 255, "a solid");
        assert_eq!(cmyk.over_paper(45, 255), 0, "a paper at full ink");
        // Relative colorimetric prints the paper as the space's white (255 = no ink), where the
        // ratio is the identity: every ramp entry comes back exactly as it was, for every tint.
        for value in 0..=255_u8 {
            assert_eq!(rgb.over_paper(value, 255), value, "relative sRGB, {value}");
            assert_eq!(cmyk.over_paper(value, 0), value, "relative CMYK, {value}");
        }
    }

    /// Converting into an output needs its PCS-to-device tables only: craft-cms asks for exactly
    /// those (`CmsConversion`'s `NoPcsToDevice` is about the other direction), so a printer's
    /// profile may hold just them, and a spot's Lab ramp must not make the output's device-to-PCS
    /// tables a requirement: the press's tables give the paper its Lab.
    #[test]
    fn an_output_needs_only_its_pcs_to_device_tables() {
        let shipped = Builtin::CoatedCmyk.profile();
        Output::new(fogra39(), shipped, Intent::AbsoluteColorimetric, false).expect("the profile as shipped converts");
        // The same profile with its device-to-PCS tables taken out, re-encoded and parsed again:
        // new bytes, so craft-cms's cache cannot hand back a transform of the original.
        let mut only_b_to_a = shipped.clone();
        only_b_to_a.a2b = Default::default();
        let printer = craft_cms::Profile::parse(&only_b_to_a.with_encoded_bytes().to_bytes()).expect("a CMYK output profile with only BToA tables");
        assert!(printer.a2b.iter().all(Option::is_none) && printer.b2a.iter().any(Option::is_some), "the restricted profile is BToA only");
        let output = Output::new(fogra39(), &printer, Intent::AbsoluteColorimetric, false).expect("an output with only BToA tables converts");
        let mut inks = InkSet::new();
        inks.add(spot("Gold", Alternate::Lab([70.0, 5.0, 60.0])));
        inks.add(spot("Rubine", Alternate::Cmyk([0.0, 1.0, 0.0, 0.0])));
        let mut planes = process_planes([0; 4], 2);
        planes.extend([vec![0, 255], vec![0, 255]]);
        let got = page(inks, planes).composite_to(&[], &output);
        let paper = page(InkSet::new(), process_planes([0; 4], 2)).composite_to(&[], &output);
        assert_eq!(&got[..4], &paper[..4], "the paper pixel");
        assert_ne!(&got[4..8], &paper[4..8], "the spots print: {got:?}");
    }
}
