// Camera colour matrices fitted to our own photographs of a ColorChecker Classic (post-November-2014 formulation),
// for cameras with no published spectral sensitivities. Kept by hand: each row's capture, fit and checks are in
// clean-room/crw/10-chart-calibration.md (scripts local-only). Reference colours: X-Rite's published L*a*b* (D50) for
// the chart, as packaged by colour-science (BSD-3-Clause). No number comes from Adobe or from a GPL raw decoder.
use craft_color_transform::Mat3;

/// Measured dual-illuminant colour calibration for a camera model.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub make: &'static str,
    pub model: &'static str,
    pub illuminant: [u16; 2],
    pub color_matrix: [Mat3; 2],
    pub forward_matrix: [Mat3; 2],
}

#[rustfmt::skip]
pub const CAMERAS: &[Camera] = &[
    // Canon EOS D60 (CRW), the maintainer's body, ISO 100. Matrix 1: one ~2,900 K tungsten bulb, 11 frames, filed as
    // Standard Illuminant A; matrix 2: direct afternoon sun, ~4,850 K, 20 frames, filed as D50 (both temperatures from
    // the camera's own white-balance presets). Fitted on chromaticity with neutral held at D50 (the sunlit chart was
    // unevenly lit): CIEDE2000 mean 2.42 / 1.70 on the 24 patches, 2.74 / 1.97 leaving each patch out.
    Camera {
        make: "Canon",
        model: "EOS D60",
        illuminant: [17, 23],
        color_matrix: [Mat3([[0.950961, -0.345297, 0.112165], [-0.545579, 1.400621, 0.558365], [-0.123107, 0.222004, 1.026236]]), Mat3([[0.609694, -0.127035, -0.045387], [-0.461201, 1.16644, 0.337283], [-0.115992, 0.229363, 0.791647]])],
        forward_matrix: [Mat3([[0.763921, 0.183562, 0.016813], [0.268545, 0.944022, -0.212567], [0.100742, -0.431647, 1.15601]]), Mat3([[0.757277, 0.190573, 0.016446], [0.291782, 1.009128, -0.300909], [0.026419, -0.264451, 1.063137]])],
    },
];
