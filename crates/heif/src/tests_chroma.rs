//! A 4:2:2 grid, the layout of Canon and Sony "HIF" files: 3 × 2 tiles of 64 × 64, 10-bit, `nclx`
//! full range. The fixture comes from heic-rs (`tests/fixtures/README.md` says where); the picture
//! it was encoded from is a formula, repeated below, so the test needs no other decoder's output.

use super::*;

const GRID_422: &[u8] = include_bytes!("../tests/fixtures/grid-422-10bit.heic");

/// The picture `grid-422-10bit.heic` was encoded from: 192 × 128 slow colour waves, RGB in 0..=1
/// (heic-rs `scripts/gen-grid-pngs.py`, `grid_422`).
fn waves(x: u32, y: u32) -> [f64; 3] {
    let (x, y) = (f64::from(x), f64::from(y));
    [0.5 + 0.35 * (x / 31.0 + 0.3).sin(), 0.5 + 0.35 * (y / 23.0 + 1.1).sin(), 0.5 + 0.35 * ((x + y) / 41.0).cos()]
}

/// Peak signal-to-noise ratio of `img` against [`waves`], over all three channels, in dB.
fn psnr_to_waves(img: &Decoded) -> f64 {
    let mut sum = 0.0;
    for (i, px) in img.data.as_chunks::<3>().0.iter().enumerate() {
        let i = u32::try_from(i).unwrap();
        let want = waves(i % img.width, i / img.width);
        for (got, want) in px.iter().zip(want) {
            sum += (f64::from(*got) / 65535.0 - want).powi(2);
        }
    }
    let mse = sum / (img.data.len() as f64);
    -10.0 * mse.log10()
}

#[test]
fn a_4_2_2_grid_decodes_to_the_picture_it_was_encoded_from() {
    // Measured with heic-rs 0.1.5: 48.6 dB with libheif's chroma upsampling, 59.4 dB with
    // Apple's. heic-rs 0.1.4 misread the 4:2:2 chroma prediction modes (H.265 Table 8-3): the
    // file decoded without an error, at 19.0 dB.
    for chroma in [Chroma::Libheif, Chroma::Apple] {
        let img = decode(GRID_422, &Options { chroma, ..Options::default() }).unwrap();
        assert_eq!((img.width, img.height, img.has_alpha, img.bit_depth), (192, 128, false, 10));
        let psnr = psnr_to_waves(&img);
        assert!(psnr >= 45.0, "{chroma:?}: {psnr:.2} dB from the picture it was encoded from");
    }
}
