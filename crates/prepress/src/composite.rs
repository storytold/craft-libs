//! A color preview composed from plates, as a Separations Preview shows the page.
//!
//! The preview is built from the plates alone, after the renderer has decided per plate what
//! overprints and what knocks out, so it shows overprint as printed without changing any
//! paint's blend mode.

use craft_pdf::interpret::color::{CmykConversion, cmyk_to_srgb8};

use crate::cms::lab_to_srgb8;
use crate::coverage::filled;
use crate::{Alternate, InkId, Plates, Process};

/// Pixels converted per call into the CMYK profile, to bound the scratch buffers.
const CHUNK: usize = 16 * 1024;

fn on(enabled: &[bool], index: usize) -> bool {
    enabled.get(index).copied().unwrap_or(true)
}

/// CMYK bytes to sRGB bytes through `cms`, else through the bundled profile pages are drawn
/// with; paper white if neither converts.
fn to_srgb(cms: &dyn CmykConversion, cmyk: &[u8], rgb: &mut [u8]) {
    if cms.to_srgb8(cmyk, rgb).or_else(|| cmyk_to_srgb8(cmyk, rgb)).is_none() {
        rgb.fill(255);
    }
}

/// Spot ink `alternate` alone at each tint byte, in sRGB: a Lab alternate runs in a straight
/// line from paper white (L\* 100) to the solid, a CMYK one scales its components, as a
/// `Separation` color space's usual tint transform does. Index the result by tint byte;
/// `None` when a Lab alternate can't be converted.
pub fn appearance(alternate: Alternate, cms: &dyn CmykConversion) -> Option<Vec<[u8; 3]>> {
    let mut rgb = vec![0_u8; 256 * 3];
    match alternate {
        Alternate::Lab([l, a, b]) => {
            let lab: Vec<f32> = (0..=255_u8)
                .flat_map(|t| {
                    let t = f32::from(t) / 255.0;
                    [100.0 + t * (l - 100.0), t * a, t * b]
                })
                .collect();
            lab_to_srgb8(&lab, &mut rgb)?;
        }
        Alternate::Cmyk(solid) => {
            let cmyk: Vec<u8> = (0..=255_u8)
                .flat_map(|t| solid.map(|c| (f32::from(t) * if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 }).round() as u8))
                .collect();
            to_srgb(cms, &cmyk, &mut rgb);
        }
    }
    Some(rgb.as_chunks::<3>().0.to_vec())
}

impl Plates {
    /// The enabled inks as they print, as sRGB8 (three bytes per pixel, row-major,
    /// `width × height` pixels). `enabled` is indexed by [`InkId::index`]; inks past its end
    /// are enabled, so `&[]` shows every ink.
    ///
    /// The process plates convert through `cms` ([`CmsConversion`](crate::CmsConversion) for
    /// the printing condition the plates were made for, see [`Plates::conversion`]), relative
    /// colorimetric with black point compensation, or through the bundled CMYK profile when
    /// `cms` can't. Each spot plate is then multiplied over them by the spot's appearance at
    /// its tint, from its alternate. Spots whose Lab alternate craft-cms can't convert are
    /// left out. Empty when `width × height × 3` overflows or can't be allocated.
    pub fn composite(&self, enabled: &[bool], cms: &dyn CmykConversion) -> Vec<u8> {
        let Some(mut rgb) = self.pixels().and_then(|n| n.checked_mul(3)).and_then(|len| filled(len, 255_u8)) else {
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
        for (c, out) in rgb.chunks_mut(CHUNK * 3).enumerate() {
            let start = c * CHUNK;
            let len = out.len() / 3;
            cmyk.clear();
            for i in start..start + len {
                cmyk.extend(process.iter().map(|plane| plane.and_then(|p| p.get(i)).copied().unwrap_or(0)));
            }
            to_srgb(cms, &cmyk, out);
        }

        for (id, spot) in self.inks.spots().filter(|(id, _)| on(enabled, id.index())) {
            let Some(plane) = self.plane(id).filter(|p| p.iter().any(|t| *t > 0)) else { continue };
            let Some(ramp) = appearance(spot.alternate, cms) else { continue };
            for (px, tint) in rgb.as_chunks_mut::<3>().0.iter_mut().zip(plane) {
                if let Some(ink) = ramp.get(usize::from(*tint)).filter(|_| *tint > 0) {
                    for (c, s) in px.iter_mut().zip(ink) {
                        *c = ((u16::from(*c) * u16::from(*s) + 127) / 255) as u8;
                    }
                }
            }
        }
        rgb
    }
}

#[cfg(test)]
mod tests {
    use crate::CmsConversion;
    use crate::cms::srgb8_to_lab;
    use crate::coverage::tests::synthetic;

    fn near(got: &[u8], expected: [u8; 3], tolerance: u8) -> bool {
        got.iter().zip(expected).all(|(g, e)| g.abs_diff(e) <= tolerance)
    }

    #[test]
    fn a_spot_appearance_runs_from_paper_to_its_solid() {
        const GOLD: [u8; 3] = [212, 175, 55];
        let ramp = super::appearance(crate::Alternate::Lab(srgb8_to_lab(GOLD).unwrap()), &CmsConversion::fogra39()).unwrap();
        assert_eq!(ramp.len(), 256);
        assert!(near(&ramp[0], [255, 255, 255], 1) && near(&ramp[255], GOLD, 2), "{:?} {:?}", ramp[0], ramp[255]);
    }

    #[test]
    fn a_black_and_gold_composite_multiplies_gold_over_black() {
        const GOLD: [u8; 3] = [212, 175, 55];
        let lab = srgb8_to_lab(GOLD).unwrap();
        // Black alone, Gold alone, Gold over black, Gold at 50%.
        let p = synthetic(4, 1, lab, [vec![0; 4], vec![0; 4], vec![0; 4], vec![255, 0, 255, 0], vec![0, 255, 255, 128]]);
        let cms = CmsConversion::fogra39();
        let rgb = p.composite(&[], &cms);
        assert_eq!(rgb.len(), 12);

        let black = &rgb[0..3];
        let (lo, hi) = (black.iter().min().unwrap(), black.iter().max().unwrap());
        assert!(*hi < 80 && hi - lo < 12, "Coated FOGRA39 black is a dark neutral: {black:?}");
        assert!(near(&rgb[3..6], GOLD, 2), "Gold: {:?}", &rgb[3..6]);
        let over: Vec<u8> = black.iter().zip(GOLD).map(|(k, g)| ((u16::from(*k) * u16::from(g) + 127) / 255) as u8).collect();
        assert!(near(&rgb[6..9], [over[0], over[1], over[2]], 2), "Gold over black: {:?}", &rgb[6..9]);
        let half = &rgb[9..12];
        assert!(half.iter().zip(GOLD).all(|(h, g)| *h > g && *h < 255) && half[2] > 110 && half[2] < 200, "Gold at 50%: {half:?}");

        let no_black = p.composite(&[true, true, true, false], &cms);
        assert!(near(&no_black[0..3], [255, 255, 255], 1), "paper: {:?}", &no_black[0..3]);
        assert!(near(&no_black[6..9], GOLD, 2));
        let no_gold = p.composite(&[true, true, true, true, false], &cms);
        assert_eq!(&no_gold[6..9], black);
    }

    #[test]
    fn plates_too_large_to_hold_give_empty_results() {
        let p = crate::Plates { width: u32::MAX, height: u32::MAX, ..synthetic(0, 0, [50.0, 0.0, 0.0], Default::default()) };
        assert!(p.composite(&[], &CmsConversion::fogra39()).is_empty());
        assert!(p.tac(&[]).is_empty());
        assert_eq!(p.stats(&[]).max_total_at, None);
        assert!(p.above(10.0, &[]).regions().is_empty());
        assert_eq!(p.tint(crate::InkId::process(crate::Process::Black), u32::MAX - 1, u32::MAX - 1), None);
    }
}
