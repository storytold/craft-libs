//! Occlusion, roughness and metallic packed into one RGBA image.
//!
//! [`try_pack_orm`] writes occlusion to red, roughness to green and metallic to blue, with alpha
//! 1, the channels glTF 2.0 section 3.9 reads its occlusion and metallic-roughness textures from
//! (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html>). Values are linear data copied
//! as the sources give them, with no transfer curve and no clamping.

use crate::source::{ScalarSource, TextureOpError, read_checked, try_buffer, try_color_image};
use crate::{Image, Limits};

/// Red where no occlusion source is given.
pub const DEFAULT_OCCLUSION: f32 = 1.0;
/// Green where no roughness source is given.
pub const DEFAULT_ROUGHNESS: f32 = 0.5;
/// Blue where no metallic source is given.
pub const DEFAULT_METALLIC: f32 = 0.0;

/// The planes [`try_pack_orm`] reads. Every given plane has the same size.
/// `#[non_exhaustive]`: start from [`OrmSources::default`], which gives none, and set the fields
/// you use.
#[derive(Clone, Copy, Default)]
#[non_exhaustive]
pub struct OrmSources<'a> {
    /// Occlusion, written to red.
    pub occlusion: Option<&'a dyn ScalarSource>,
    /// A second occlusion plane, such as baked occlusion, multiplied into red when given. Without
    /// [`OrmSources::occlusion`], red is this plane alone.
    pub occlusion_multiplier: Option<&'a dyn ScalarSource>,
    /// Roughness, written to green.
    pub roughness: Option<&'a dyn ScalarSource>,
    /// Metallic, written to blue.
    pub metallic: Option<&'a dyn ScalarSource>,
    /// The output size. Required when no plane is given; otherwise every plane must have it.
    pub size: Option<(u32, u32)>,
}

/// The RGBA image `[occlusion, roughness, metallic, 1]` of `sources`. A plane that is not given
/// takes [`DEFAULT_OCCLUSION`], [`DEFAULT_ROUGHNESS`] or [`DEFAULT_METALLIC`]. Rows are read one
/// at a time.
///
/// Returns [`TextureOpError::SizeMismatch`] when the planes and [`OrmSources::size`] differ in
/// size, [`TextureOpError::NoSize`] when nothing gives a size, an [`crate::ImageError`] when the
/// size is outside `limits` or memory runs out, all before any row is read,
/// [`TextureOpError::NonFiniteSample`] for a NaN or infinite sample,
/// [`TextureOpError::NonFiniteResult`] when the occlusion product overflows, and
/// [`TextureOpError::Source`] when a row cannot be read.
pub fn try_pack_orm(sources: &OrmSources<'_>, limits: &Limits) -> Result<Image, TextureOpError> {
    let mut size = sources.size;
    for source in [sources.occlusion, sources.occlusion_multiplier, sources.roughness, sources.metallic].into_iter().flatten() {
        let found = source.dimensions();
        match size {
            None => size = Some(found),
            Some(expected) if expected != found => return Err(TextureOpError::SizeMismatch { expected, found }),
            Some(_) => {}
        }
    }
    let (w, h) = size.ok_or(TextureOpError::NoSize)?;
    try_color_image(w, h, limits, |y0, rows| fill(sources, w, h, y0, rows))
}

/// One plane's current row, or its default when the plane is not given.
struct Plane<'a> {
    source: Option<&'a dyn ScalarSource>,
    row: Vec<f32>,
    default: f32,
}

impl<'a> Plane<'a> {
    fn new(source: Option<&'a dyn ScalarSource>, default: f32, w: u32, h: u32) -> Result<Plane<'a>, TextureOpError> {
        let row = if source.is_some() { try_buffer(w as usize, w, h)? } else { Vec::new() };
        Ok(Plane { source, row, default })
    }

    fn load(&mut self, y: u32) -> Result<(), TextureOpError> {
        match self.source {
            Some(source) => read_checked(source, y, &mut self.row),
            None => Ok(()),
        }
    }

    fn at(&self, x: usize) -> f32 {
        self.row.get(x).copied().unwrap_or(self.default)
    }
}

/// Fill the RGBA rows starting at row `y0`.
fn fill(sources: &OrmSources<'_>, w: u32, h: u32, y0: u32, rows: &mut [f32]) -> Result<(), TextureOpError> {
    let mut occlusion = Plane::new(sources.occlusion, DEFAULT_OCCLUSION, w, h)?;
    let mut multiplier = Plane::new(sources.occlusion_multiplier, 1.0, w, h)?;
    let mut roughness = Plane::new(sources.roughness, DEFAULT_ROUGHNESS, w, h)?;
    let mut metallic = Plane::new(sources.metallic, DEFAULT_METALLIC, w, h)?;
    for (y, out) in (y0..).zip(rows.chunks_exact_mut(w as usize * 4)) {
        for plane in [&mut occlusion, &mut multiplier, &mut roughness, &mut metallic] {
            plane.load(y)?;
        }
        for (x, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let ao = if sources.occlusion_multiplier.is_some() { occlusion.at(x) * multiplier.at(x) } else { occlusion.at(x) };
            if !ao.is_finite() {
                return Err(TextureOpError::NonFiniteResult { x: x as u32, y });
            }
            *px = [ao, roughness.at(x), metallic.at(x), 1.0];
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normal::{HeightNormalOptions, try_height_to_normal};
    use crate::{ImageError, Kind, SourceError};

    /// An 8-bit channel value, as an 8-bit PNG stores it.
    fn byte(v: f32) -> i32 {
        (v.clamp(0.0, 1.0) * 255.0).round() as i32
    }

    fn pack(sources: &OrmSources<'_>) -> Image {
        match try_pack_orm(sources, &Limits::default()) {
            Ok(img) => img,
            Err(e) => panic!("packing failed: {e}"),
        }
    }

    fn constant(v: f32) -> Image {
        Image::filled(64, 64, Kind::Gray, [v; 4])
    }

    #[test]
    fn gltf_png_matches_the_composite() {
        let (rough, metal) = (constant(0.25), constant(0.75));
        let s = OrmSources { roughness: Some(&rough), metallic: Some(&metal), ..OrmSources::default() };
        let orm = pack(&s).get(5, 5, false).map(byte);
        assert_eq!(orm[0], 255, "no AO anywhere → white");
        assert!((orm[1] - 64).abs() <= 1 && (orm[2] - 191).abs() <= 1, "{orm:?}");
        assert_eq!(orm[3], 255);
        // A flat height gives the flat normal.
        let Ok(n) = try_height_to_normal(&constant(0.0), &HeightNormalOptions::default(), &Limits::default()) else { panic!("normal map") };
        let n = n.get(5, 5, false).map(byte);
        assert!((n[0] - 128).abs() <= 1 && (n[1] - 128).abs() <= 1 && n[2] == 255, "{n:?}");
    }

    #[test]
    fn ao_mixes_baked_and_channel() {
        let (channel, baked) = (constant(0.5), constant(0.5));
        let mut s = OrmSources { occlusion: Some(&channel), occlusion_multiplier: Some(&baked), ..OrmSources::default() };
        assert!((byte(pack(&s).get(3, 3, false)[0]) - 64).abs() <= 1);
        // Without the multiplier the channel is used alone; without the channel the multiplier is.
        s.occlusion_multiplier = None;
        assert_eq!(pack(&s).get(3, 3, false)[0], 0.5);
        s.occlusion = None;
        s.occlusion_multiplier = Some(&baked);
        assert_eq!(pack(&s).get(3, 3, false)[0], 0.5);
    }

    #[test]
    fn each_plane_goes_to_its_own_channel() {
        let ao = Image::gray(3, 2, |x, y| 0.1 * (x + 3 * y) as f32);
        let rough = Image::gray(3, 2, |x, _| 0.2 + 0.01 * x as f32);
        let metal = Image::gray(3, 2, |_, y| 0.9 - 0.5 * y as f32);
        let s = OrmSources { occlusion: Some(&ao), roughness: Some(&rough), metallic: Some(&metal), ..OrmSources::default() };
        let orm = pack(&s);
        for y in 0..2 {
            for x in 0..3 {
                let want = [ao.get(x, y, false)[0], rough.get(x, y, false)[0], metal.get(x, y, false)[0], 1.0];
                assert_eq!(orm.get(x, y, false), want, "({x}, {y})");
            }
        }
        // Values outside 0..=1 are kept.
        let hdr = Image::filled(1, 1, Kind::Gray, [-0.5; 4]);
        let s = OrmSources { metallic: Some(&hdr), ..OrmSources::default() };
        assert_eq!(pack(&s).data(), &[1.0, 0.5, -0.5, 1.0]);
    }

    #[test]
    fn missing_planes_take_the_defaults() {
        let mut s = OrmSources::default();
        assert_eq!(try_pack_orm(&s, &Limits::default()), Err(TextureOpError::NoSize));
        s.size = Some((2, 1));
        assert_eq!(pack(&s).data(), &[1.0, 0.5, 0.0, 1.0, 1.0, 0.5, 0.0, 1.0]);
        s.size = Some((0, 0));
        assert_eq!(try_pack_orm(&s, &Limits::default()), Err(ImageError::BadSize(0, 0).into()));
    }

    /// A source of any claimed size that fails every read.
    struct Failing(u32, u32);

    impl ScalarSource for Failing {
        fn dimensions(&self) -> (u32, u32) {
            (self.0, self.1)
        }
        fn read_row(&self, _y: u32, _x0: u32, _out: &mut [f32]) -> Result<(), SourceError> {
            Err(SourceError("tile unavailable".into()))
        }
    }

    #[test]
    fn sizes_are_checked_before_reading() {
        let (a, b) = (Failing(4, 4), Failing(4, 5));
        let mut s = OrmSources { roughness: Some(&a), metallic: Some(&b), ..OrmSources::default() };
        assert_eq!(try_pack_orm(&s, &Limits::default()), Err(TextureOpError::SizeMismatch { expected: (4, 4), found: (4, 5) }));
        s.metallic = None;
        s.size = Some((5, 4));
        assert_eq!(try_pack_orm(&s, &Limits::default()), Err(TextureOpError::SizeMismatch { expected: (5, 4), found: (4, 4) }));
        s.size = None;
        let tiny = Limits { max_bytes: 63, ..Limits::default() };
        assert_eq!(try_pack_orm(&s, &tiny), Err(ImageError::TooLarge(4, 4).into()));
        let huge = Failing(u32::MAX, u32::MAX);
        s.roughness = Some(&huge);
        let open = Limits { max_side: u32::MAX, max_pixels: u64::MAX, max_bytes: u64::MAX };
        assert_eq!(try_pack_orm(&s, &open), Err(ImageError::TooLarge(u32::MAX, u32::MAX).into()));
        s.roughness = Some(&a);
        assert!(matches!(try_pack_orm(&s, &Limits::default()), Err(TextureOpError::Source { y: 0, .. })));
    }

    #[test]
    fn non_finite_samples_are_errors() {
        let l = Limits::default();
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let Ok(img) = Image::from_data(2, 2, Kind::Gray, vec![0.0, 0.0, 0.0, bad], &l) else { panic!("valid image") };
            let s = OrmSources { occlusion_multiplier: Some(&img), ..OrmSources::default() };
            assert_eq!(try_pack_orm(&s, &l), Err(TextureOpError::NonFiniteSample { x: 1, y: 1 }));
        }
        // Finite occlusion values whose product overflows.
        let Ok(big) = Image::from_data(2, 1, Kind::Gray, vec![1.0, f32::MAX], &l) else { panic!("valid image") };
        let Ok(two) = Image::from_data(2, 1, Kind::Gray, vec![1.0, 2.0], &l) else { panic!("valid image") };
        let s = OrmSources { occlusion: Some(&big), occlusion_multiplier: Some(&two), ..OrmSources::default() };
        assert_eq!(try_pack_orm(&s, &l), Err(TextureOpError::NonFiniteResult { x: 1, y: 0 }));
    }
}
