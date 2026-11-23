//! Bluetooth Low Complexity Subband Coding (SBC) audio codec.
//!
//! Clean-room implementation based on Bluetooth SIG Advanced Audio Distribution Profile
//! (A2DP) Specification v1.3, Appendix B (Section 12: "Technical Specification of SBC")
//! and Hands-Free Profile (HFP 1.6) mSBC specification.
//!
//! SBC is a subband codec designed to deliver audio quality with low computational
//! complexity, operating at 16 kHz, 32 kHz, 44.1 kHz, or 48 kHz in Mono, Dual Channel,
//! Stereo, or Joint Stereo configurations with 4 or 8 subbands.

#![allow(clippy::excessive_precision, clippy::needless_range_loop, clippy::cognitive_complexity, clippy::too_many_arguments)]

const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_SAMPLES: usize = 64 * 1024 * 1024;

/// Standard SBC syncword byte (`0x9C`).
pub const SYNCWORD_SBC: u8 = 0x9C;
/// mSBC wideband speech syncword byte (`0xAD`).
pub const SYNCWORD_MSBC: u8 = 0xAD;

/// Offset table for 4 subbands loudness bit allocation (Table 12.21).
pub static OFFSET4: [[i8; 4]; 4] = [[-1, 0, 0, 0], [-2, 0, 0, 1], [-2, 0, 0, 1], [-2, 0, 0, 1]];

/// Offset table for 8 subbands loudness bit allocation (Table 12.22).
pub static OFFSET8: [[i8; 8]; 4] = [[-2, 0, 0, 0, 0, 0, 0, 1], [-3, 0, 0, 0, 0, 0, 1, 2], [-4, 0, 0, 0, 0, 0, 1, 2], [-4, 0, 0, 0, 0, 0, 1, 2]];

/// Analysis and synthesis filter coefficients for 4 subbands (Table 12.23).
pub(crate) static PROTO_4_40: [f32; 40] = [
    0.0000000000e+00_f32,
    5.3654897600e-04_f32,
    1.4918835700e-03_f32,
    2.7337090400e-03_f32,
    3.8372019300e-03_f32,
    3.8920514900e-03_f32,
    1.8658169100e-03_f32,
    -3.0601228600e-03_f32,
    1.0913762000e-02_f32,
    2.0438508700e-02_f32,
    2.8875739200e-02_f32,
    3.2193929000e-02_f32,
    2.5876781100e-02_f32,
    6.1324518600e-03_f32,
    -2.8821727400e-02_f32,
    -7.7646349400e-02_f32,
    1.3559327400e-01_f32,
    1.9498784100e-01_f32,
    2.4663666200e-01_f32,
    2.8182820300e-01_f32,
    2.9431533200e-01_f32,
    2.8182820300e-01_f32,
    2.4663666200e-01_f32,
    1.9498784100e-01_f32,
    -1.3559327400e-01_f32,
    -7.7646349400e-02_f32,
    -2.8821727400e-02_f32,
    6.1324518600e-03_f32,
    2.5876781100e-02_f32,
    3.2193929000e-02_f32,
    2.8875739200e-02_f32,
    2.0438508700e-02_f32,
    -1.0913762000e-02_f32,
    -3.0601228600e-03_f32,
    1.8658169100e-03_f32,
    3.8920514900e-03_f32,
    3.8372019300e-03_f32,
    2.7337090400e-03_f32,
    1.4918835700e-03_f32,
    5.3654897600e-04_f32,
];

/// Analysis and synthesis filter coefficients for 8 subbands (Table 12.24).
pub(crate) static PROTO_8_80: [f32; 80] = [
    0.0000000000e+00_f32,
    1.5657539800e-04_f32,
    3.4325642500e-04_f32,
    5.5462020200e-04_f32,
    8.2391950600e-04_f32,
    1.1399250700e-03_f32,
    1.4764016900e-03_f32,
    1.7837172500e-03_f32,
    2.0118254200e-03_f32,
    2.1037198900e-03_f32,
    1.9945455400e-03_f32,
    1.6165628300e-03_f32,
    9.0215450200e-04_f32,
    -1.7880536100e-04_f32,
    -1.6497309800e-03_f32,
    -3.4971745400e-03_f32,
    5.6594947300e-03_f32,
    8.0294116300e-03_f32,
    1.0458444300e-02_f32,
    1.2747233500e-02_f32,
    1.4652526300e-02_f32,
    1.5904560300e-02_f32,
    1.6220847100e-02_f32,
    1.5318410600e-02_f32,
    1.2937180600e-02_f32,
    8.8575754000e-03_f32,
    2.9240844200e-03_f32,
    -4.9157802400e-03_f32,
    -1.4640407600e-02_f32,
    -2.6109875200e-02_f32,
    -3.9075138100e-02_f32,
    -5.3187303200e-02_f32,
    6.7998943100e-02_f32,
    8.2984757800e-02_f32,
    9.7575391800e-02_f32,
    1.1119668900e-01_f32,
    1.2326454800e-01_f32,
    1.3326441500e-01_f32,
    1.4075350500e-01_f32,
    1.4538984700e-01_f32,
    1.4695506800e-01_f32,
    1.4538984700e-01_f32,
    1.4075350500e-01_f32,
    1.3326441500e-01_f32,
    1.2326454800e-01_f32,
    1.1119668900e-01_f32,
    9.7575391800e-02_f32,
    8.2984757800e-02_f32,
    -6.7998943100e-02_f32,
    -5.3187303200e-02_f32,
    -3.9075138100e-02_f32,
    -2.6109875200e-02_f32,
    -1.4640407600e-02_f32,
    -4.9157802400e-03_f32,
    2.9240844200e-03_f32,
    8.8575754000e-03_f32,
    1.2937180600e-02_f32,
    1.5318410600e-02_f32,
    1.6220847100e-02_f32,
    1.5904560300e-02_f32,
    1.4652526300e-02_f32,
    1.2747233500e-02_f32,
    1.0458444300e-02_f32,
    8.0294116300e-03_f32,
    -5.6594947300e-03_f32,
    -3.4971745400e-03_f32,
    -1.6497309800e-03_f32,
    -1.7880536100e-04_f32,
    9.0215450200e-04_f32,
    1.6165628300e-03_f32,
    1.9945455400e-03_f32,
    2.1037198900e-03_f32,
    2.0118254200e-03_f32,
    1.7837172500e-03_f32,
    1.4764016900e-03_f32,
    1.1399250700e-03_f32,
    8.2391950600e-04_f32,
    5.5462020200e-04_f32,
    3.4325642500e-04_f32,
    1.5657539800e-04_f32,
];

/// Scaled synthesis filter prototype for 4 subbands.
pub(crate) static SYNTH_D_4: [f32; 40] = [
    -0.0000000000e+00_f32,
    -2.1461959040e-03_f32,
    -5.9675342800e-03_f32,
    -1.0934836160e-02_f32,
    -1.5348807720e-02_f32,
    -1.5568205960e-02_f32,
    -7.4632676400e-03_f32,
    1.2240491440e-02_f32,
    -4.3655048000e-02_f32,
    -8.1754034800e-02_f32,
    -1.1550295680e-01_f32,
    -1.2877571600e-01_f32,
    -1.0350712440e-01_f32,
    -2.4529807440e-02_f32,
    1.1528690960e-01_f32,
    3.1058539760e-01_f32,
    -5.4237309600e-01_f32,
    -7.7995136400e-01_f32,
    -9.8654664800e-01_f32,
    -1.1273128120e+00_f32,
    -1.1772613280e+00_f32,
    -1.1273128120e+00_f32,
    -9.8654664800e-01_f32,
    -7.7995136400e-01_f32,
    5.4237309600e-01_f32,
    3.1058539760e-01_f32,
    1.1528690960e-01_f32,
    -2.4529807440e-02_f32,
    -1.0350712440e-01_f32,
    -1.2877571600e-01_f32,
    -1.1550295680e-01_f32,
    -8.1754034800e-02_f32,
    4.3655048000e-02_f32,
    1.2240491440e-02_f32,
    -7.4632676400e-03_f32,
    -1.5568205960e-02_f32,
    -1.5348807720e-02_f32,
    -1.0934836160e-02_f32,
    -5.9675342800e-03_f32,
    -2.1461959040e-03_f32,
];

/// Scaled synthesis filter prototype for 8 subbands.
pub(crate) static SYNTH_D_8: [f32; 80] = [
    -0.0000000000e+00_f32,
    -1.2526031840e-03_f32,
    -2.7460514000e-03_f32,
    -4.4369616160e-03_f32,
    -6.5913560480e-03_f32,
    -9.1194005600e-03_f32,
    -1.1811213520e-02_f32,
    -1.4269738000e-02_f32,
    -1.6094603360e-02_f32,
    -1.6829759120e-02_f32,
    -1.5956364320e-02_f32,
    -1.2932502640e-02_f32,
    -7.2172360160e-03_f32,
    1.4304428880e-03_f32,
    1.3197847840e-02_f32,
    2.7977396320e-02_f32,
    -4.5275957840e-02_f32,
    -6.4235293040e-02_f32,
    -8.3667554400e-02_f32,
    -1.0197786800e-01_f32,
    -1.1722021040e-01_f32,
    -1.2723648240e-01_f32,
    -1.2976677680e-01_f32,
    -1.2254728480e-01_f32,
    -1.0349744480e-01_f32,
    -7.0860603200e-02_f32,
    -2.3392675360e-02_f32,
    3.9326241920e-02_f32,
    1.1712326080e-01_f32,
    2.0887900160e-01_f32,
    3.1260110480e-01_f32,
    4.2549842560e-01_f32,
    -5.4399154480e-01_f32,
    -6.6387806240e-01_f32,
    -7.8060313440e-01_f32,
    -8.8957351200e-01_f32,
    -9.8611638400e-01_f32,
    -1.0661153200e+00_f32,
    -1.1260280400e+00_f32,
    -1.1631187760e+00_f32,
    -1.1756405440e+00_f32,
    -1.1631187760e+00_f32,
    -1.1260280400e+00_f32,
    -1.0661153200e+00_f32,
    -9.8611638400e-01_f32,
    -8.8957351200e-01_f32,
    -7.8060313440e-01_f32,
    -6.6387806240e-01_f32,
    5.4399154480e-01_f32,
    4.2549842560e-01_f32,
    3.1260110480e-01_f32,
    2.0887900160e-01_f32,
    1.1712326080e-01_f32,
    3.9326241920e-02_f32,
    -2.3392675360e-02_f32,
    -7.0860603200e-02_f32,
    -1.0349744480e-01_f32,
    -1.2254728480e-01_f32,
    -1.2976677680e-01_f32,
    -1.2723648240e-01_f32,
    -1.1722021040e-01_f32,
    -1.0197786800e-01_f32,
    -8.3667554400e-02_f32,
    -6.4235293040e-02_f32,
    4.5275957840e-02_f32,
    2.7977396320e-02_f32,
    1.3197847840e-02_f32,
    1.4304428880e-03_f32,
    -7.2172360160e-03_f32,
    -1.2932502640e-02_f32,
    -1.5956364320e-02_f32,
    -1.6829759120e-02_f32,
    -1.6094603360e-02_f32,
    -1.4269738000e-02_f32,
    -1.1811213520e-02_f32,
    -9.1194005600e-03_f32,
    -6.5913560480e-03_f32,
    -4.4369616160e-03_f32,
    -2.7460514000e-03_f32,
    -1.2526031840e-03_f32,
];

pub(crate) static SYNTH_COS_4: [[f32; 4]; 8] = [
    [7.0710678119e-01_f32, -7.0710678119e-01_f32, -7.0710678119e-01_f32, 7.0710678119e-01_f32],
    [3.8268343237e-01_f32, -9.2387953251e-01_f32, 9.2387953251e-01_f32, -3.8268343237e-01_f32],
    [6.1232339957e-17_f32, -1.8369701987e-16_f32, 3.0616169979e-16_f32, -4.2862637970e-16_f32],
    [-3.8268343237e-01_f32, 9.2387953251e-01_f32, -9.2387953251e-01_f32, 3.8268343237e-01_f32],
    [-7.0710678119e-01_f32, 7.0710678119e-01_f32, 7.0710678119e-01_f32, -7.0710678119e-01_f32],
    [-9.2387953251e-01_f32, -3.8268343237e-01_f32, 3.8268343237e-01_f32, 9.2387953251e-01_f32],
    [-1.0000000000e+00_f32, -1.0000000000e+00_f32, -1.0000000000e+00_f32, -1.0000000000e+00_f32],
    [-9.2387953251e-01_f32, -3.8268343237e-01_f32, 3.8268343237e-01_f32, 9.2387953251e-01_f32],
];

pub(crate) static SYNTH_COS_8: [[f32; 8]; 16] = [
    [
        7.0710678119e-01_f32,
        -7.0710678119e-01_f32,
        -7.0710678119e-01_f32,
        7.0710678119e-01_f32,
        7.0710678119e-01_f32,
        -7.0710678119e-01_f32,
        -7.0710678119e-01_f32,
        7.0710678119e-01_f32,
    ],
    [
        5.5557023302e-01_f32,
        -9.8078528040e-01_f32,
        1.9509032202e-01_f32,
        8.3146961230e-01_f32,
        -8.3146961230e-01_f32,
        -1.9509032202e-01_f32,
        9.8078528040e-01_f32,
        -5.5557023302e-01_f32,
    ],
    [
        3.8268343237e-01_f32,
        -9.2387953251e-01_f32,
        9.2387953251e-01_f32,
        -3.8268343237e-01_f32,
        -3.8268343237e-01_f32,
        9.2387953251e-01_f32,
        -9.2387953251e-01_f32,
        3.8268343237e-01_f32,
    ],
    [
        1.9509032202e-01_f32,
        -5.5557023302e-01_f32,
        8.3146961230e-01_f32,
        -9.8078528040e-01_f32,
        9.8078528040e-01_f32,
        -8.3146961230e-01_f32,
        5.5557023302e-01_f32,
        -1.9509032202e-01_f32,
    ],
    [
        6.1232339957e-17_f32,
        -1.8369701987e-16_f32,
        3.0616169979e-16_f32,
        -4.2862637970e-16_f32,
        5.5109105962e-16_f32,
        -2.4499125789e-15_f32,
        -9.8033641995e-16_f32,
        -2.6948419388e-15_f32,
    ],
    [
        -1.9509032202e-01_f32,
        5.5557023302e-01_f32,
        -8.3146961230e-01_f32,
        9.8078528040e-01_f32,
        -9.8078528040e-01_f32,
        8.3146961230e-01_f32,
        -5.5557023302e-01_f32,
        1.9509032202e-01_f32,
    ],
    [
        -3.8268343237e-01_f32,
        9.2387953251e-01_f32,
        -9.2387953251e-01_f32,
        3.8268343237e-01_f32,
        3.8268343237e-01_f32,
        -9.2387953251e-01_f32,
        9.2387953251e-01_f32,
        -3.8268343237e-01_f32,
    ],
    [
        -5.5557023302e-01_f32,
        9.8078528040e-01_f32,
        -1.9509032202e-01_f32,
        -8.3146961230e-01_f32,
        8.3146961230e-01_f32,
        1.9509032202e-01_f32,
        -9.8078528040e-01_f32,
        5.5557023302e-01_f32,
    ],
    [
        -7.0710678119e-01_f32,
        7.0710678119e-01_f32,
        7.0710678119e-01_f32,
        -7.0710678119e-01_f32,
        -7.0710678119e-01_f32,
        7.0710678119e-01_f32,
        7.0710678119e-01_f32,
        -7.0710678119e-01_f32,
    ],
    [
        -8.3146961230e-01_f32,
        1.9509032202e-01_f32,
        9.8078528040e-01_f32,
        5.5557023302e-01_f32,
        -5.5557023302e-01_f32,
        -9.8078528040e-01_f32,
        -1.9509032202e-01_f32,
        8.3146961230e-01_f32,
    ],
    [
        -9.2387953251e-01_f32,
        -3.8268343237e-01_f32,
        3.8268343237e-01_f32,
        9.2387953251e-01_f32,
        9.2387953251e-01_f32,
        3.8268343237e-01_f32,
        -3.8268343237e-01_f32,
        -9.2387953251e-01_f32,
    ],
    [
        -9.8078528040e-01_f32,
        -8.3146961230e-01_f32,
        -5.5557023302e-01_f32,
        -1.9509032202e-01_f32,
        1.9509032202e-01_f32,
        5.5557023302e-01_f32,
        8.3146961230e-01_f32,
        9.8078528040e-01_f32,
    ],
    [
        -1.0000000000e+00_f32,
        -1.0000000000e+00_f32,
        -1.0000000000e+00_f32,
        -1.0000000000e+00_f32,
        -1.0000000000e+00_f32,
        -1.0000000000e+00_f32,
        -1.0000000000e+00_f32,
        -1.0000000000e+00_f32,
    ],
    [
        -9.8078528040e-01_f32,
        -8.3146961230e-01_f32,
        -5.5557023302e-01_f32,
        -1.9509032202e-01_f32,
        1.9509032202e-01_f32,
        5.5557023302e-01_f32,
        8.3146961230e-01_f32,
        9.8078528040e-01_f32,
    ],
    [
        -9.2387953251e-01_f32,
        -3.8268343237e-01_f32,
        3.8268343237e-01_f32,
        9.2387953251e-01_f32,
        9.2387953251e-01_f32,
        3.8268343237e-01_f32,
        -3.8268343237e-01_f32,
        -9.2387953251e-01_f32,
    ],
    [
        -8.3146961230e-01_f32,
        1.9509032202e-01_f32,
        9.8078528040e-01_f32,
        5.5557023302e-01_f32,
        -5.5557023302e-01_f32,
        -9.8078528040e-01_f32,
        -1.9509032202e-01_f32,
        8.3146961230e-01_f32,
    ],
];

pub(crate) static ANALYSIS_COS_4: [[f32; 8]; 4] = [
    [
        7.0710678119e-01_f32,
        9.2387953251e-01_f32,
        1.0000000000e+00_f32,
        9.2387953251e-01_f32,
        7.0710678119e-01_f32,
        3.8268343237e-01_f32,
        6.1232339957e-17_f32,
        -3.8268343237e-01_f32,
    ],
    [
        -7.0710678119e-01_f32,
        3.8268343237e-01_f32,
        1.0000000000e+00_f32,
        3.8268343237e-01_f32,
        -7.0710678119e-01_f32,
        -9.2387953251e-01_f32,
        -1.8369701987e-16_f32,
        9.2387953251e-01_f32,
    ],
    [
        -7.0710678119e-01_f32,
        -3.8268343237e-01_f32,
        1.0000000000e+00_f32,
        -3.8268343237e-01_f32,
        -7.0710678119e-01_f32,
        9.2387953251e-01_f32,
        3.0616169979e-16_f32,
        -9.2387953251e-01_f32,
    ],
    [
        7.0710678119e-01_f32,
        -9.2387953251e-01_f32,
        1.0000000000e+00_f32,
        -9.2387953251e-01_f32,
        7.0710678119e-01_f32,
        -3.8268343237e-01_f32,
        -4.2862637970e-16_f32,
        3.8268343237e-01_f32,
    ],
];

pub(crate) static ANALYSIS_COS_8: [[f32; 16]; 8] = [
    [
        7.0710678119e-01_f32,
        8.3146961230e-01_f32,
        9.2387953251e-01_f32,
        9.8078528040e-01_f32,
        1.0000000000e+00_f32,
        9.8078528040e-01_f32,
        9.2387953251e-01_f32,
        8.3146961230e-01_f32,
        7.0710678119e-01_f32,
        5.5557023302e-01_f32,
        3.8268343237e-01_f32,
        1.9509032202e-01_f32,
        6.1232339957e-17_f32,
        -1.9509032202e-01_f32,
        -3.8268343237e-01_f32,
        -5.5557023302e-01_f32,
    ],
    [
        -7.0710678119e-01_f32,
        -1.9509032202e-01_f32,
        3.8268343237e-01_f32,
        8.3146961230e-01_f32,
        1.0000000000e+00_f32,
        8.3146961230e-01_f32,
        3.8268343237e-01_f32,
        -1.9509032202e-01_f32,
        -7.0710678119e-01_f32,
        -9.8078528040e-01_f32,
        -9.2387953251e-01_f32,
        -5.5557023302e-01_f32,
        -1.8369701987e-16_f32,
        5.5557023302e-01_f32,
        9.2387953251e-01_f32,
        9.8078528040e-01_f32,
    ],
    [
        -7.0710678119e-01_f32,
        -9.8078528040e-01_f32,
        -3.8268343237e-01_f32,
        5.5557023302e-01_f32,
        1.0000000000e+00_f32,
        5.5557023302e-01_f32,
        -3.8268343237e-01_f32,
        -9.8078528040e-01_f32,
        -7.0710678119e-01_f32,
        1.9509032202e-01_f32,
        9.2387953251e-01_f32,
        8.3146961230e-01_f32,
        3.0616169979e-16_f32,
        -8.3146961230e-01_f32,
        -9.2387953251e-01_f32,
        -1.9509032202e-01_f32,
    ],
    [
        7.0710678119e-01_f32,
        -5.5557023302e-01_f32,
        -9.2387953251e-01_f32,
        1.9509032202e-01_f32,
        1.0000000000e+00_f32,
        1.9509032202e-01_f32,
        -9.2387953251e-01_f32,
        -5.5557023302e-01_f32,
        7.0710678119e-01_f32,
        8.3146961230e-01_f32,
        -3.8268343237e-01_f32,
        -9.8078528040e-01_f32,
        -4.2862637970e-16_f32,
        9.8078528040e-01_f32,
        3.8268343237e-01_f32,
        -8.3146961230e-01_f32,
    ],
    [
        7.0710678119e-01_f32,
        5.5557023302e-01_f32,
        -9.2387953251e-01_f32,
        -1.9509032202e-01_f32,
        1.0000000000e+00_f32,
        -1.9509032202e-01_f32,
        -9.2387953251e-01_f32,
        5.5557023302e-01_f32,
        7.0710678119e-01_f32,
        -8.3146961230e-01_f32,
        -3.8268343237e-01_f32,
        9.8078528040e-01_f32,
        5.5109105962e-16_f32,
        -9.8078528040e-01_f32,
        3.8268343237e-01_f32,
        8.3146961230e-01_f32,
    ],
    [
        -7.0710678119e-01_f32,
        9.8078528040e-01_f32,
        -3.8268343237e-01_f32,
        -5.5557023302e-01_f32,
        1.0000000000e+00_f32,
        -5.5557023302e-01_f32,
        -3.8268343237e-01_f32,
        9.8078528040e-01_f32,
        -7.0710678119e-01_f32,
        -1.9509032202e-01_f32,
        9.2387953251e-01_f32,
        -8.3146961230e-01_f32,
        -2.4499125789e-15_f32,
        8.3146961230e-01_f32,
        -9.2387953251e-01_f32,
        1.9509032202e-01_f32,
    ],
    [
        -7.0710678119e-01_f32,
        1.9509032202e-01_f32,
        3.8268343237e-01_f32,
        -8.3146961230e-01_f32,
        1.0000000000e+00_f32,
        -8.3146961230e-01_f32,
        3.8268343237e-01_f32,
        1.9509032202e-01_f32,
        -7.0710678119e-01_f32,
        9.8078528040e-01_f32,
        -9.2387953251e-01_f32,
        5.5557023302e-01_f32,
        -9.8033641995e-16_f32,
        -5.5557023302e-01_f32,
        9.2387953251e-01_f32,
        -9.8078528040e-01_f32,
    ],
    [
        7.0710678119e-01_f32,
        -8.3146961230e-01_f32,
        9.2387953251e-01_f32,
        -9.8078528040e-01_f32,
        1.0000000000e+00_f32,
        -9.8078528040e-01_f32,
        9.2387953251e-01_f32,
        -8.3146961230e-01_f32,
        7.0710678119e-01_f32,
        -5.5557023302e-01_f32,
        3.8268343237e-01_f32,
        -1.9509032202e-01_f32,
        -2.6948419388e-15_f32,
        1.9509032202e-01_f32,
        -3.8268343237e-01_f32,
        5.5557023302e-01_f32,
    ],
];

/// Sampling frequency of the SBC bitstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplingFrequency {
    /// 16 000 Hz
    Freq16k,
    /// 32 000 Hz
    Freq32k,
    /// 44 100 Hz
    Freq44k1,
    /// 48 000 Hz
    Freq48k,
}

impl SamplingFrequency {
    /// Returns sampling frequency in Hz.
    #[inline]
    #[must_use]
    pub const fn as_hz(self) -> u32 {
        match self {
            Self::Freq16k => 16_000,
            Self::Freq32k => 32_000,
            Self::Freq44k1 => 44_100,
            Self::Freq48k => 48_000,
        }
    }

    /// Derives sampling frequency from Hz.
    pub fn from_hz(hz: u32) -> Result<Self, Error> {
        match hz {
            16_000 => Ok(Self::Freq16k),
            32_000 => Ok(Self::Freq32k),
            44_100 => Ok(Self::Freq44k1),
            48_000 => Ok(Self::Freq48k),
            _ => Err(Error::InvalidSampleRate(hz)),
        }
    }

    /// Returns the 2-bit header code.
    #[inline]
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Freq16k => 0b00,
            Self::Freq32k => 0b01,
            Self::Freq44k1 => 0b10,
            Self::Freq48k => 0b11,
        }
    }

    /// Index into offset tables (0 to 3).
    #[inline]
    #[must_use]
    pub const fn table_index(self) -> usize {
        match self {
            Self::Freq16k => 0,
            Self::Freq32k => 1,
            Self::Freq44k1 => 2,
            Self::Freq48k => 3,
        }
    }
}

/// Number of blocks per SBC frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockCount {
    /// 4 blocks
    Blocks4,
    /// 8 blocks
    Blocks8,
    /// 12 blocks
    Blocks12,
    /// 16 blocks
    Blocks16,
    /// 15 blocks (mSBC wideband speech)
    Blocks15,
}

impl BlockCount {
    /// Returns integer block count.
    #[inline]
    #[must_use]
    pub const fn count(self) -> usize {
        match self {
            Self::Blocks4 => 4,
            Self::Blocks8 => 8,
            Self::Blocks12 => 12,
            Self::Blocks16 => 16,
            Self::Blocks15 => 15,
        }
    }

    /// Derives block count from integer.
    pub fn from_count(count: usize) -> Result<Self, Error> {
        match count {
            4 => Ok(Self::Blocks4),
            8 => Ok(Self::Blocks8),
            12 => Ok(Self::Blocks12),
            16 => Ok(Self::Blocks16),
            15 => Ok(Self::Blocks15),
            _ => Err(Error::InvalidBlockCount(count)),
        }
    }

    /// Returns 2-bit header code.
    #[inline]
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Blocks4 => 0b00,
            Self::Blocks8 => 0b01,
            Self::Blocks12 => 0b10,
            Self::Blocks16 | Self::Blocks15 => 0b11,
        }
    }
}

/// Channel mode of the SBC bitstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMode {
    /// 1 channel (Mono)
    Mono,
    /// 2 channels with independent bit allocation
    DualChannel,
    /// 2 channels with shared bit allocation
    Stereo,
    /// 2 channels with shared bit allocation and joint mid/side subbands
    JointStereo,
}

impl ChannelMode {
    /// Returns number of audio channels.
    #[inline]
    #[must_use]
    pub const fn channels(self) -> usize {
        match self {
            Self::Mono => 1,
            Self::DualChannel | Self::Stereo | Self::JointStereo => 2,
        }
    }

    /// Returns 2-bit header code.
    #[inline]
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Mono => 0b00,
            Self::DualChannel => 0b01,
            Self::Stereo => 0b10,
            Self::JointStereo => 0b11,
        }
    }
}

/// Bit allocation method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AllocationMethod {
    /// Psychoacoustic loudness allocation
    #[default]
    Loudness,
    /// Signal-to-noise ratio allocation
    Snr,
}

impl AllocationMethod {
    /// Returns 1-bit header code.
    #[inline]
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Loudness => 0,
            Self::Snr => 1,
        }
    }
}

/// Number of subbands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subbands {
    /// 4 subbands
    Subbands4,
    /// 8 subbands
    Subbands8,
}

impl Subbands {
    /// Returns number of subbands as integer.
    #[inline]
    #[must_use]
    pub const fn count(self) -> usize {
        match self {
            Self::Subbands4 => 4,
            Self::Subbands8 => 8,
        }
    }

    /// Derives Subbands from integer.
    pub fn from_count(count: usize) -> Result<Self, Error> {
        match count {
            4 => Ok(Self::Subbands4),
            8 => Ok(Self::Subbands8),
            _ => Err(Error::InvalidSubbands(count)),
        }
    }

    /// Returns 1-bit header code.
    #[inline]
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Subbands4 => 0,
            Self::Subbands8 => 1,
        }
    }
}

/// Parsed header of an SBC audio frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Syncword byte (`0x9C` or `0xAD`).
    pub syncword: u8,
    /// Sampling frequency.
    pub sampling_frequency: SamplingFrequency,
    /// Number of blocks.
    pub blocks: BlockCount,
    /// Channel mode.
    pub channel_mode: ChannelMode,
    /// Allocation method.
    pub allocation_method: AllocationMethod,
    /// Number of subbands.
    pub subbands: Subbands,
    /// Bitpool size (1 to 255).
    pub bitpool: u8,
    /// Stored CRC-8 parity check word.
    pub crc_check: u8,
    /// Joint stereo flags for subbands 0..subbands-1.
    pub join: [bool; 8],
}

impl Header {
    /// Parses an SBC frame header from a byte slice.
    pub fn parse(src: &[u8]) -> Result<Self, Error> {
        if src.len() < 4 {
            return Err(Error::TruncatedFrame { expected: 4, actual: src.len() });
        }
        let syncword = src[0];
        if syncword == SYNCWORD_MSBC {
            let crc_check = src[3];
            return Ok(Self {
                syncword,
                sampling_frequency: SamplingFrequency::Freq16k,
                blocks: BlockCount::Blocks15,
                channel_mode: ChannelMode::Mono,
                allocation_method: AllocationMethod::Loudness,
                subbands: Subbands::Subbands8,
                bitpool: 26,
                crc_check,
                join: [false; 8],
            });
        }
        if syncword != SYNCWORD_SBC {
            return Err(Error::InvalidSyncword(syncword));
        }
        let b1 = src[1];
        let freq_code = (b1 >> 6) & 3;
        let blocks_code = (b1 >> 4) & 3;
        let mode_code = (b1 >> 2) & 3;
        let alloc_code = (b1 >> 1) & 1;
        let subbands_code = b1 & 1;

        let sampling_frequency = match freq_code {
            0 => SamplingFrequency::Freq16k,
            1 => SamplingFrequency::Freq32k,
            2 => SamplingFrequency::Freq44k1,
            _ => SamplingFrequency::Freq48k,
        };
        let blocks = match blocks_code {
            0 => BlockCount::Blocks4,
            1 => BlockCount::Blocks8,
            2 => BlockCount::Blocks12,
            _ => BlockCount::Blocks16,
        };
        let channel_mode = match mode_code {
            0 => ChannelMode::Mono,
            1 => ChannelMode::DualChannel,
            2 => ChannelMode::Stereo,
            _ => ChannelMode::JointStereo,
        };
        let allocation_method = if alloc_code == 0 { AllocationMethod::Loudness } else { AllocationMethod::Snr };
        let subbands = if subbands_code == 0 { Subbands::Subbands4 } else { Subbands::Subbands8 };
        let bitpool = src[2];
        let crc_check = src[3];

        let max_bitpool = match channel_mode {
            ChannelMode::Mono | ChannelMode::DualChannel => 16 * subbands.count(),
            ChannelMode::Stereo | ChannelMode::JointStereo => 32 * subbands.count(),
        };
        if bitpool == 0 || (bitpool as usize) > max_bitpool {
            return Err(Error::InvalidBitpool(bitpool));
        }

        let mut join = [false; 8];
        if channel_mode == ChannelMode::JointStereo && src.len() >= 5 {
            let join_byte = src[4];
            let sb_count = subbands.count();
            for sb in 0..(sb_count.saturating_sub(1)) {
                let bit_shift = 7 - (sb as u8);
                join[sb] = ((join_byte >> bit_shift) & 1) != 0;
            }
        }

        Ok(Self { syncword, sampling_frequency, blocks, channel_mode, allocation_method, subbands, bitpool, crc_check, join })
    }

    /// Total length of the frame in bytes (Section 12.9).
    #[must_use]
    pub fn frame_length(&self) -> usize {
        if self.syncword == SYNCWORD_MSBC {
            return 57;
        }
        let nrof_subbands = self.subbands.count();
        let nrof_channels = self.channel_mode.channels();
        let nrof_blocks = self.blocks.count();
        let bitpool = self.bitpool as usize;

        let base = 4 + (4 * nrof_subbands * nrof_channels) / 8;
        match self.channel_mode {
            ChannelMode::Mono | ChannelMode::DualChannel => base + (nrof_blocks * nrof_channels * bitpool).div_ceil(8),
            ChannelMode::Stereo => base + (nrof_blocks * bitpool).div_ceil(8),
            ChannelMode::JointStereo => base + (nrof_subbands + nrof_blocks * bitpool).div_ceil(8),
        }
    }

    /// Number of audio samples per channel produced by this frame.
    #[inline]
    #[must_use]
    pub const fn samples_per_frame(&self) -> usize {
        self.blocks.count() * self.subbands.count()
    }

    /// Number of audio channels.
    #[inline]
    #[must_use]
    pub const fn channels(&self) -> usize {
        self.channel_mode.channels()
    }

    /// Sampling rate in Hz.
    #[inline]
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.sampling_frequency.as_hz()
    }

    /// Bit rate of the stream in bits per second (Section 12.9).
    #[must_use]
    pub fn bit_rate_bps(&self) -> u32 {
        let flen = self.frame_length() as u64;
        let fs = self.sample_rate() as u64;
        let sub = self.subbands.count() as u64;
        let blk = self.blocks.count() as u64;
        let bps = (8 * flen * fs) / (sub * blk);
        bps as u32
    }
}

/// Information about a decoded SBC stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamInfo {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count (1 or 2).
    pub channels: usize,
    /// Channel mode.
    pub channel_mode: ChannelMode,
    /// Block count.
    pub blocks: usize,
    /// Subband count.
    pub subbands: usize,
    /// Bitpool setting.
    pub bitpool: u8,
    /// Allocation method.
    pub allocation_method: AllocationMethod,
}

/// Errors encountered in SBC operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Invalid SBC syncword.
    #[error("invalid SBC syncword: {0:#04x}")]
    InvalidSyncword(u8),
    /// Frame truncated before complete read.
    #[error("truncated frame: expected {expected} bytes, got {actual}")]
    TruncatedFrame {
        /// Expected byte length.
        expected: usize,
        /// Actual available byte length.
        actual: usize,
    },
    /// CRC-8 verification mismatch.
    #[error("CRC-8 check failed: computed {computed:#04x}, frame header had {stored:#04x}")]
    CrcMismatch {
        /// Computed CRC-8 value.
        computed: u8,
        /// Stored CRC-8 in frame header.
        stored: u8,
    },
    /// Unsupported channel count.
    #[error("unsupported channel count: {0} (must be 1 or 2)")]
    InvalidChannels(usize),
    /// Unsupported sample rate.
    #[error("unsupported sample rate: {0} Hz (must be 16000, 32000, 44100, or 48000 Hz)")]
    InvalidSampleRate(u32),
    /// Unsupported block count.
    #[error("unsupported block count: {0} (must be 4, 8, 12, or 16)")]
    InvalidBlockCount(usize),
    /// Unsupported subbands count.
    #[error("unsupported subband count: {0} (must be 4 or 8)")]
    InvalidSubbands(usize),
    /// Invalid bitpool value.
    #[error("invalid bitpool value: {0}")]
    InvalidBitpool(u8),
    /// Input exceeds safe maximum processing limit.
    #[error("input exceeds safe maximum processing limit")]
    InputTooLarge,
    /// Memory allocation failed.
    #[error("memory allocation failed")]
    AllocationFailed,
}

/// Computes CRC-8 with generator polynomial `0x1D` and initial state `0x0F` (Section 12.6.1.1).
pub fn crc8(bits: impl IntoIterator<Item = bool>) -> u8 {
    let mut crc: u8 = 0x0F;
    for bit in bits {
        let feedback = (crc >> 7) ^ (if bit { 1 } else { 0 });
        crc = crc.wrapping_shl(1) ^ (if feedback != 0 { 0x1D } else { 0 });
    }
    crc
}

struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_pos: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, byte_pos: 0, bit_pos: 7 }
    }

    fn read_bits(&mut self, n: usize) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        if n > 32 {
            return None;
        }
        let mut val = 0u32;
        for _ in 0..n {
            if self.byte_pos >= self.data.len() {
                return None;
            }
            let bit = (self.data[self.byte_pos] >> self.bit_pos) & 1;
            val = (val << 1) | u32::from(bit);
            if self.bit_pos == 0 {
                self.bit_pos = 7;
                self.byte_pos = self.byte_pos.saturating_add(1);
            } else {
                self.bit_pos -= 1;
            }
        }
        Some(val)
    }
}

struct BitWriter {
    bytes: Vec<u8>,
    cur_byte: u8,
    bit_pos: u8,
}

impl BitWriter {
    fn with_capacity(cap: usize) -> Result<Self, Error> {
        let mut bytes = Vec::new();
        bytes.try_reserve(cap).map_err(|_| Error::AllocationFailed)?;
        Ok(Self { bytes, cur_byte: 0, bit_pos: 7 })
    }

    fn write_bits(&mut self, val: u32, n: usize) -> Result<(), Error> {
        if n == 0 {
            return Ok(());
        }
        for i in (0..n).rev() {
            let bit = ((val >> i) & 1) as u8;
            self.cur_byte |= bit << self.bit_pos;
            if self.bit_pos == 0 {
                self.bytes.try_reserve(1).map_err(|_| Error::AllocationFailed)?;
                self.bytes.push(self.cur_byte);
                self.cur_byte = 0;
                self.bit_pos = 7;
            } else {
                self.bit_pos -= 1;
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Error> {
        if self.bit_pos < 7 {
            self.bytes.try_reserve(1).map_err(|_| Error::AllocationFailed)?;
            self.bytes.push(self.cur_byte);
            self.cur_byte = 0;
            self.bit_pos = 7;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<u8>, Error> {
        self.flush()?;
        Ok(self.bytes)
    }
}

fn compute_header_crc(
    b1: u8,
    b2: u8,
    channel_mode: ChannelMode,
    subbands: usize,
    channels: usize,
    join: &[bool; 8],
    scale_factors: &[[u8; 8]; 2],
) -> u8 {
    let mut crc: u8 = 0x0F;
    let mut feed_bit = |bit: bool| {
        let feedback = (crc >> 7) ^ (if bit { 1 } else { 0 });
        crc = crc.wrapping_shl(1) ^ (if feedback != 0 { 0x1D } else { 0 });
    };
    for i in (0..8).rev() {
        feed_bit(((b1 >> i) & 1) != 0);
    }
    for i in (0..8).rev() {
        feed_bit(((b2 >> i) & 1) != 0);
    }
    if channel_mode == ChannelMode::JointStereo {
        for sb in 0..(subbands.saturating_sub(1)) {
            feed_bit(join[sb]);
        }
        feed_bit(false); // RFA
    }
    for ch in 0..channels {
        for sb in 0..subbands {
            let sf = scale_factors[ch][sb];
            for i in (0..4).rev() {
                feed_bit(((sf >> i) & 1) != 0);
            }
        }
    }
    crc
}

fn compute_bit_allocation(
    subbands: usize,
    channels: usize,
    channel_mode: ChannelMode,
    freq_idx: usize,
    alloc_method: AllocationMethod,
    bitpool: u16,
    scale_factors: &[[u8; 8]; 2],
    bits: &mut [[u8; 8]; 2],
) {
    if channel_mode == ChannelMode::Mono || channel_mode == ChannelMode::DualChannel {
        for ch in 0..channels {
            let mut bitneed = [0i16; 8];
            if alloc_method == AllocationMethod::Snr {
                for sb in 0..subbands {
                    bitneed[sb] = i16::from(scale_factors[ch][sb]);
                }
            } else {
                for sb in 0..subbands {
                    let sf = scale_factors[ch][sb];
                    if sf == 0 {
                        bitneed[sb] = -5;
                    } else {
                        let off = if subbands == 4 { OFFSET4[freq_idx][sb] as i16 } else { OFFSET8[freq_idx][sb] as i16 };
                        let loudness = (sf as i16) - off;
                        bitneed[sb] = if loudness > 0 { loudness / 2 } else { loudness };
                    }
                }
            }

            let mut max_bitneed = 0i16;
            for sb in 0..subbands {
                if bitneed[sb] > max_bitneed {
                    max_bitneed = bitneed[sb];
                }
            }

            let mut bitcount = 0u16;
            let mut slicecount = 0u16;
            let mut bitslice = max_bitneed + 1;

            loop {
                bitslice -= 1;
                bitcount = bitcount.saturating_add(slicecount);
                slicecount = 0;
                for sb in 0..subbands {
                    let bn = bitneed[sb];
                    if bn > bitslice + 1 && bn < bitslice + 16 {
                        slicecount = slicecount.saturating_add(1);
                    } else if bn == bitslice + 1 {
                        slicecount = slicecount.saturating_add(2);
                    }
                }
                if bitcount.saturating_add(slicecount) >= bitpool || bitslice <= -32 {
                    break;
                }
            }

            if bitcount.saturating_add(slicecount) == bitpool {
                bitcount = bitcount.saturating_add(slicecount);
                bitslice -= 1;
            }

            for sb in 0..subbands {
                let bn = bitneed[sb];
                if bn < bitslice + 2 {
                    bits[ch][sb] = 0;
                } else {
                    let b = (bn - bitslice).clamp(0, 16);
                    bits[ch][sb] = b as u8;
                }
            }

            let mut sb = 0;
            while bitcount < bitpool && sb < subbands {
                if bits[ch][sb] >= 2 && bits[ch][sb] < 16 {
                    bits[ch][sb] = bits[ch][sb].saturating_add(1);
                    bitcount = bitcount.saturating_add(1);
                } else if bitneed[sb] == bitslice + 1 && bitpool > bitcount.saturating_add(1) {
                    bits[ch][sb] = 2;
                    bitcount = bitcount.saturating_add(2);
                }
                sb += 1;
            }

            let mut sb = 0;
            while bitcount < bitpool && sb < subbands {
                if bits[ch][sb] < 16 {
                    bits[ch][sb] = bits[ch][sb].saturating_add(1);
                    bitcount = bitcount.saturating_add(1);
                }
                sb += 1;
            }
        }
    } else {
        let mut bitneed = [[0i16; 8]; 2];
        if alloc_method == AllocationMethod::Snr {
            for ch in 0..2 {
                for sb in 0..subbands {
                    bitneed[ch][sb] = i16::from(scale_factors[ch][sb]);
                }
            }
        } else {
            for ch in 0..2 {
                for sb in 0..subbands {
                    let sf = scale_factors[ch][sb];
                    if sf == 0 {
                        bitneed[ch][sb] = -5;
                    } else {
                        let off = if subbands == 4 { OFFSET4[freq_idx][sb] as i16 } else { OFFSET8[freq_idx][sb] as i16 };
                        let loudness = (sf as i16) - off;
                        bitneed[ch][sb] = if loudness > 0 { loudness / 2 } else { loudness };
                    }
                }
            }
        }

        let mut max_bitneed = 0i16;
        for ch in 0..2 {
            for sb in 0..subbands {
                if bitneed[ch][sb] > max_bitneed {
                    max_bitneed = bitneed[ch][sb];
                }
            }
        }

        let mut bitcount = 0u16;
        let mut slicecount = 0u16;
        let mut bitslice = max_bitneed + 1;

        loop {
            bitslice -= 1;
            bitcount = bitcount.saturating_add(slicecount);
            slicecount = 0;
            for ch in 0..2 {
                for sb in 0..subbands {
                    let bn = bitneed[ch][sb];
                    if bn > bitslice + 1 && bn < bitslice + 16 {
                        slicecount = slicecount.saturating_add(1);
                    } else if bn == bitslice + 1 {
                        slicecount = slicecount.saturating_add(2);
                    }
                }
            }
            if bitcount.saturating_add(slicecount) >= bitpool || bitslice <= -32 {
                break;
            }
        }

        if bitcount.saturating_add(slicecount) == bitpool {
            bitcount = bitcount.saturating_add(slicecount);
            bitslice -= 1;
        }

        for ch in 0..2 {
            for sb in 0..subbands {
                let bn = bitneed[ch][sb];
                if bn < bitslice + 2 {
                    bits[ch][sb] = 0;
                } else {
                    let b = (bn - bitslice).clamp(0, 16);
                    bits[ch][sb] = b as u8;
                }
            }
        }

        let mut ch = 0;
        let mut sb = 0;
        while bitcount < bitpool && sb < subbands {
            if bits[ch][sb] >= 2 && bits[ch][sb] < 16 {
                bits[ch][sb] = bits[ch][sb].saturating_add(1);
                bitcount = bitcount.saturating_add(1);
            } else if bitneed[ch][sb] == bitslice + 1 && bitpool > bitcount.saturating_add(1) {
                bits[ch][sb] = 2;
                bitcount = bitcount.saturating_add(2);
            }
            if ch == 1 {
                ch = 0;
                sb += 1;
            } else {
                ch = 1;
            }
        }

        let mut ch = 0;
        let mut sb = 0;
        while bitcount < bitpool && sb < subbands {
            if bits[ch][sb] < 16 {
                bits[ch][sb] = bits[ch][sb].saturating_add(1);
                bitcount = bitcount.saturating_add(1);
            }
            if ch == 1 {
                ch = 0;
                sb += 1;
            } else {
                ch = 1;
            }
        }
    }
}

fn synth_subbands_4(v: &mut [f32; 160], s: &[f32; 8], out: &mut [i16; 8]) {
    v.copy_within(0..72, 8);
    for k in 0..8 {
        let mut sum = 0.0f32;
        for i in 0..4 {
            sum += SYNTH_COS_4[k][i] * s[i];
        }
        v[k] = sum;
    }
    let mut u = [0.0f32; 40];
    for i in 0..5 {
        for j in 0..4 {
            u[i * 8 + j] = v[i * 16 + j];
            u[i * 8 + 4 + j] = v[i * 16 + 12 + j];
        }
    }
    for j in 0..4 {
        let mut sum = 0.0f32;
        for i in 0..10 {
            sum += u[j + 4 * i] * SYNTH_D_4[j + 4 * i];
        }
        let clamped = sum.clamp(-32768.0, 32767.0).round() as i32;
        out[j] = clamped as i16;
    }
}

fn synth_subbands_8(v: &mut [f32; 160], s: &[f32; 8], out: &mut [i16; 8]) {
    v.copy_within(0..144, 16);
    for k in 0..16 {
        let mut sum = 0.0f32;
        for i in 0..8 {
            sum += SYNTH_COS_8[k][i] * s[i];
        }
        v[k] = sum;
    }
    let mut u = [0.0f32; 80];
    for i in 0..5 {
        for j in 0..8 {
            u[i * 16 + j] = v[i * 32 + j];
            u[i * 16 + 8 + j] = v[i * 32 + 24 + j];
        }
    }
    for j in 0..8 {
        let mut sum = 0.0f32;
        for i in 0..10 {
            sum += u[j + 8 * i] * SYNTH_D_8[j + 8 * i];
        }
        let clamped = sum.clamp(-32768.0, 32767.0).round() as i32;
        out[j] = clamped as i16;
    }
}

fn analysis_subbands_4(x: &mut [f32; 80], pcm_in: &[i16], s_out: &mut [f32; 8]) {
    x.copy_within(0..36, 4);
    for i in 0..4 {
        x[3 - i] = pcm_in[i] as f32;
    }
    let mut y = [0.0f32; 8];
    for i in 0..8 {
        let mut sum = 0.0f32;
        for k in 0..5 {
            let idx = i + k * 8;
            sum += PROTO_4_40[idx] * x[idx];
        }
        y[i] = sum;
    }
    for i in 0..4 {
        let mut sum = 0.0f32;
        for k in 0..8 {
            sum += ANALYSIS_COS_4[i][k] * y[k];
        }
        s_out[i] = sum;
    }
}

fn analysis_subbands_8(x: &mut [f32; 80], pcm_in: &[i16], s_out: &mut [f32; 8]) {
    x.copy_within(0..72, 8);
    for i in 0..8 {
        x[7 - i] = pcm_in[i] as f32;
    }
    let mut y = [0.0f32; 16];
    for i in 0..16 {
        let mut sum = 0.0f32;
        for k in 0..5 {
            let idx = i + k * 16;
            sum += PROTO_8_80[idx] * x[idx];
        }
        y[i] = sum;
    }
    for i in 0..8 {
        let mut sum = 0.0f32;
        for k in 0..16 {
            sum += ANALYSIS_COS_8[i][k] * y[k];
        }
        s_out[i] = sum;
    }
}

fn calculate_scalefactor(max_abs: f32) -> u8 {
    let v = max_abs.abs() as u32;
    if v == 0 {
        return 0;
    }
    let mut sf = 0u8;
    while sf < 15 && (1u32 << (sf + 1)) <= v {
        sf += 1;
    }
    sf
}

/// SBC audio decoder.
#[derive(Debug, Clone)]
pub struct Decoder {
    v: [[f32; 160]; 2],
}

impl Default for Decoder {
    fn default() -> Self {
        Self { v: [[0.0f32; 160]; 2] }
    }
}

impl Decoder {
    /// Creates a new SBC decoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets synthesis filterbank state.
    pub fn reset(&mut self) {
        self.v = [[0.0f32; 160]; 2];
    }

    /// Decodes a single SBC frame from `packet`, appending linear PCM samples to `dst`.
    /// Returns the number of bytes consumed from `packet`.
    pub fn decode_frame(&mut self, packet: &[u8], dst: &mut Vec<i16>) -> Result<usize, Error> {
        let header = Header::parse(packet)?;
        let frame_len = header.frame_length();
        if packet.len() < frame_len {
            return Err(Error::TruncatedFrame { expected: frame_len, actual: packet.len() });
        }

        let channels = header.channels();
        let subbands = header.subbands.count();
        let blocks = header.blocks.count();

        let mut reader = BitReader::new(&packet[4..frame_len]);

        let mut join = [false; 8];
        if header.channel_mode == ChannelMode::JointStereo {
            for sb in 0..(subbands.saturating_sub(1)) {
                join[sb] = reader.read_bits(1).unwrap_or(0) != 0;
            }
            let _ = reader.read_bits(1); // RFA
        }

        let mut scale_factors = [[0u8; 8]; 2];
        for ch in 0..channels {
            for sb in 0..subbands {
                scale_factors[ch][sb] = (reader.read_bits(4).unwrap_or(0) & 0x0F) as u8;
            }
        }

        // Verify CRC-8
        let expected_crc = if header.syncword == SYNCWORD_MSBC {
            compute_header_crc(0x00, 0x00, ChannelMode::Mono, 8, 1, &[false; 8], &scale_factors)
        } else {
            compute_header_crc(packet[1], packet[2], header.channel_mode, subbands, channels, &join, &scale_factors)
        };
        if expected_crc != header.crc_check {
            return Err(Error::CrcMismatch { computed: expected_crc, stored: header.crc_check });
        }

        let mut bits = [[0u8; 8]; 2];
        compute_bit_allocation(
            subbands,
            channels,
            header.channel_mode,
            header.sampling_frequency.table_index(),
            header.allocation_method,
            u16::from(header.bitpool),
            &scale_factors,
            &mut bits,
        );

        let mut levels = [[0u32; 8]; 2];
        let mut scalefactors = [[0.0f32; 8]; 2];
        for ch in 0..channels {
            for sb in 0..subbands {
                levels[ch][sb] = (1u32 << bits[ch][sb]).saturating_sub(1);
                scalefactors[ch][sb] = (1u32 << (u32::from(scale_factors[ch][sb]) + 1)) as f32;
            }
        }

        // Subband sample reconstruction
        let mut sb_sample = [[[0.0f32; 8]; 2]; 16];
        for blk in 0..blocks {
            for ch in 0..channels {
                for sb in 0..subbands {
                    let lev = levels[ch][sb];
                    if lev > 0 {
                        let raw_s = reader.read_bits(bits[ch][sb] as usize).unwrap_or(0);
                        let norm = ((raw_s as f32 * 2.0 + 1.0) / (lev as f32)) - 1.0;
                        sb_sample[blk][ch][sb] = scalefactors[ch][sb] * norm;
                    } else {
                        sb_sample[blk][ch][sb] = 0.0;
                    }
                }
            }
        }

        // Joint stereo mid/side reconstruction
        if header.channel_mode == ChannelMode::JointStereo {
            for blk in 0..blocks {
                for sb in 0..subbands {
                    if join[sb] {
                        let mid = sb_sample[blk][0][sb] + sb_sample[blk][1][sb];
                        let side = sb_sample[blk][0][sb] - sb_sample[blk][1][sb];
                        sb_sample[blk][0][sb] = mid;
                        sb_sample[blk][1][sb] = side;
                    }
                }
            }
        }

        // Synthesis filterbank
        let total_samples = blocks * subbands * channels;
        dst.try_reserve(total_samples).map_err(|_| Error::AllocationFailed)?;

        let mut block_samples = [[[0i16; 8]; 2]; 16];
        for blk in 0..blocks {
            for ch in 0..channels {
                if subbands == 4 {
                    synth_subbands_4(&mut self.v[ch], &sb_sample[blk][ch], &mut block_samples[blk][ch]);
                } else {
                    synth_subbands_8(&mut self.v[ch], &sb_sample[blk][ch], &mut block_samples[blk][ch]);
                }
            }
            for sb in 0..subbands {
                for ch in 0..channels {
                    dst.push(block_samples[blk][ch][sb]);
                }
            }
        }

        Ok(frame_len)
    }

    /// Decodes an entire buffer of concatenated SBC frames into interleaved 16-bit PCM.
    pub fn decode(&mut self, src: &[u8], dst: &mut Vec<i16>) -> Result<(), Error> {
        if src.len() > MAX_INPUT_BYTES {
            return Err(Error::InputTooLarge);
        }
        let mut offset = 0;
        while offset < src.len() {
            let consumed = self.decode_frame(&src[offset..], dst)?;
            if consumed == 0 {
                break;
            }
            offset = offset.saturating_add(consumed);
        }
        Ok(())
    }
}

/// SBC audio encoder.
#[derive(Debug, Clone)]
pub struct Encoder {
    sample_rate: u32,
    channels: usize,
    channel_mode: ChannelMode,
    blocks: BlockCount,
    subbands: Subbands,
    bitpool: u8,
    allocation_method: AllocationMethod,
    x: [[f32; 80]; 2],
}

impl Encoder {
    /// Creates a new SBC encoder with custom configuration.
    pub fn new(
        sample_rate: u32,
        channels: usize,
        blocks: usize,
        subbands: usize,
        bitpool: u8,
        allocation_method: AllocationMethod,
    ) -> Result<Self, Error> {
        if channels != 1 && channels != 2 {
            return Err(Error::InvalidChannels(channels));
        }
        let freq = SamplingFrequency::from_hz(sample_rate)?;
        let block_cnt = BlockCount::from_count(blocks)?;
        let subband_cnt = Subbands::from_count(subbands)?;
        let channel_mode = if channels == 1 { ChannelMode::Mono } else { ChannelMode::JointStereo };

        let max_bitpool = match channel_mode {
            ChannelMode::Mono | ChannelMode::DualChannel => 16 * subband_cnt.count(),
            ChannelMode::Stereo | ChannelMode::JointStereo => 32 * subband_cnt.count(),
        };
        if bitpool == 0 || (bitpool as usize) > max_bitpool {
            return Err(Error::InvalidBitpool(bitpool));
        }

        if block_cnt == BlockCount::Blocks15
            && (sample_rate != 16_000 || channels != 1 || subbands != 8 || bitpool != 26 || allocation_method != AllocationMethod::Loudness)
        {
            return Err(Error::InvalidBlockCount(15));
        }

        Ok(Self {
            sample_rate: freq.as_hz(),
            channels,
            channel_mode,
            blocks: block_cnt,
            subbands: subband_cnt,
            bitpool,
            allocation_method,
            x: [[0.0f32; 80]; 2],
        })
    }

    /// Resets analysis filterbank state.
    pub fn reset(&mut self) {
        self.x = [[0.0f32; 80]; 2];
    }

    /// Encodes a single frame of linear PCM samples into `dst`.
    /// `pcm` must contain `blocks * subbands * channels` interleaved 16-bit samples.
    pub fn encode_frame(&mut self, pcm: &[i16], dst: &mut Vec<u8>) -> Result<(), Error> {
        let subbands = self.subbands.count();
        let blocks = self.blocks.count();
        let expected_samples = blocks * subbands * self.channels;
        if pcm.len() < expected_samples {
            return Err(Error::TruncatedFrame { expected: expected_samples, actual: pcm.len() });
        }

        // Polyphase analysis filterbank
        let mut sb_samples = [[[0.0f32; 8]; 2]; 16];
        for blk in 0..blocks {
            for ch in 0..self.channels {
                let mut channel_buf = [0i16; 8];
                for i in 0..subbands {
                    let idx = (blk * subbands + i) * self.channels + ch;
                    channel_buf[i] = pcm[idx];
                }
                if subbands == 4 {
                    analysis_subbands_4(&mut self.x[ch], &channel_buf, &mut sb_samples[blk][ch]);
                } else {
                    analysis_subbands_8(&mut self.x[ch], &channel_buf, &mut sb_samples[blk][ch]);
                }
            }
        }

        // Scale factors and joint stereo decisions
        let mut scale_factors = [[0u8; 8]; 2];
        let mut join = [false; 8];

        if self.channels == 2 && self.channel_mode == ChannelMode::JointStereo {
            for sb in 0..subbands {
                let mut max_l = 0.0f32;
                let mut max_r = 0.0f32;
                for blk in 0..blocks {
                    let al = sb_samples[blk][0][sb].abs();
                    let ar = sb_samples[blk][1][sb].abs();
                    if al > max_l {
                        max_l = al;
                    }
                    if ar > max_r {
                        max_r = ar;
                    }
                }
                let sf_l = calculate_scalefactor(max_l);
                let sf_r = calculate_scalefactor(max_r);

                if sb < subbands.saturating_sub(1) {
                    let mut max_mid = 0.0f32;
                    let mut max_side = 0.0f32;
                    for blk in 0..blocks {
                        let mid = (sb_samples[blk][0][sb] + sb_samples[blk][1][sb]) * 0.5;
                        let side = (sb_samples[blk][0][sb] - sb_samples[blk][1][sb]) * 0.5;
                        let am = mid.abs();
                        let asd = side.abs();
                        if am > max_mid {
                            max_mid = am;
                        }
                        if asd > max_side {
                            max_side = asd;
                        }
                    }
                    let sf_mid = calculate_scalefactor(max_mid);
                    let sf_side = calculate_scalefactor(max_side);

                    if u16::from(sf_l) + u16::from(sf_r) > u16::from(sf_mid) + u16::from(sf_side) {
                        join[sb] = true;
                        scale_factors[0][sb] = sf_mid;
                        scale_factors[1][sb] = sf_side;
                        for blk in 0..blocks {
                            let mid = (sb_samples[blk][0][sb] + sb_samples[blk][1][sb]) * 0.5;
                            let side = (sb_samples[blk][0][sb] - sb_samples[blk][1][sb]) * 0.5;
                            sb_samples[blk][0][sb] = mid;
                            sb_samples[blk][1][sb] = side;
                        }
                        continue;
                    }
                }

                scale_factors[0][sb] = sf_l;
                scale_factors[1][sb] = sf_r;
            }
        } else {
            for ch in 0..self.channels {
                for sb in 0..subbands {
                    let mut max_val = 0.0f32;
                    for blk in 0..blocks {
                        let a = sb_samples[blk][ch][sb].abs();
                        if a > max_val {
                            max_val = a;
                        }
                    }
                    scale_factors[ch][sb] = calculate_scalefactor(max_val);
                }
            }
        }

        // Bit allocation
        let freq = SamplingFrequency::from_hz(self.sample_rate)?;
        let mut bits = [[0u8; 8]; 2];
        compute_bit_allocation(
            subbands,
            self.channels,
            self.channel_mode,
            freq.table_index(),
            self.allocation_method,
            u16::from(self.bitpool),
            &scale_factors,
            &mut bits,
        );

        // Quantization
        let mut levels = [[0u32; 8]; 2];
        let mut scalefactors = [[0.0f32; 8]; 2];
        for ch in 0..self.channels {
            for sb in 0..subbands {
                levels[ch][sb] = (1u32 << bits[ch][sb]).saturating_sub(1);
                scalefactors[ch][sb] = (1u32 << (u32::from(scale_factors[ch][sb]) + 1)) as f32;
            }
        }

        let mut quantized = [[[0u32; 8]; 2]; 16];
        for blk in 0..blocks {
            for ch in 0..self.channels {
                for sb in 0..subbands {
                    let lev = levels[ch][sb];
                    if lev > 0 {
                        let norm = sb_samples[blk][ch][sb] / scalefactors[ch][sb];
                        let val = ((norm + 1.0) * (lev as f32)) * 0.5;
                        let q = val.floor() as i32;
                        quantized[blk][ch][sb] = (q.clamp(0, lev as i32)) as u32;
                    }
                }
            }
        }

        // Frame header bytes
        let is_msbc = self.blocks == BlockCount::Blocks15;
        let (syncword, b1, b2, crc, flen) = if is_msbc {
            let crc = compute_header_crc(0x00, 0x00, ChannelMode::Mono, 8, 1, &[false; 8], &scale_factors);
            (SYNCWORD_MSBC, 0x00, 0x00, crc, 57)
        } else {
            let b1 = (freq.code() << 6)
                | (self.blocks.code() << 4)
                | (self.channel_mode.code() << 2)
                | (self.allocation_method.code() << 1)
                | self.subbands.code();
            let b2 = self.bitpool;
            let crc = compute_header_crc(b1, b2, self.channel_mode, subbands, self.channels, &join, &scale_factors);
            let flen = Header {
                syncword: SYNCWORD_SBC,
                sampling_frequency: freq,
                blocks: self.blocks,
                channel_mode: self.channel_mode,
                allocation_method: self.allocation_method,
                subbands: self.subbands,
                bitpool: self.bitpool,
                crc_check: crc,
                join,
            }
            .frame_length();
            (SYNCWORD_SBC, b1, b2, crc, flen)
        };

        let mut writer = BitWriter::with_capacity(flen)?;
        writer.write_bits(u32::from(syncword), 8)?;
        writer.write_bits(u32::from(b1), 8)?;
        writer.write_bits(u32::from(b2), 8)?;
        writer.write_bits(u32::from(crc), 8)?;

        if self.channel_mode == ChannelMode::JointStereo {
            for sb in 0..(subbands.saturating_sub(1)) {
                writer.write_bits(if join[sb] { 1 } else { 0 }, 1)?;
            }
            writer.write_bits(0, 1)?; // RFA
        }

        for ch in 0..self.channels {
            for sb in 0..subbands {
                writer.write_bits(u32::from(scale_factors[ch][sb]), 4)?;
            }
        }

        for blk in 0..blocks {
            for ch in 0..self.channels {
                for sb in 0..subbands {
                    if bits[ch][sb] > 0 {
                        writer.write_bits(quantized[blk][ch][sb], bits[ch][sb] as usize)?;
                    }
                }
            }
        }

        let frame_bytes = writer.finish()?;
        dst.try_reserve(frame_bytes.len()).map_err(|_| Error::AllocationFailed)?;
        dst.extend_from_slice(&frame_bytes);
        Ok(())
    }

    /// Encodes a stream of linear PCM samples into concatenated SBC frames.
    pub fn encode(&mut self, pcm: &[i16], dst: &mut Vec<u8>) -> Result<(), Error> {
        if pcm.len() > MAX_SAMPLES {
            return Err(Error::InputTooLarge);
        }
        let frame_samples = self.blocks.count() * self.subbands.count() * self.channels;
        if !pcm.len().is_multiple_of(frame_samples) {
            return Err(Error::TruncatedFrame { expected: frame_samples, actual: pcm.len() % frame_samples });
        }
        let mut offset: usize = 0;
        while offset.saturating_add(frame_samples) <= pcm.len() {
            self.encode_frame(&pcm[offset..offset + frame_samples], dst)?;
            offset = offset.saturating_add(frame_samples);
        }
        Ok(())
    }
}

/// Decodes an entire SBC bitstream to 16-bit linear PCM and returns stream info.
pub fn decode(src: &[u8]) -> Result<(Vec<i16>, StreamInfo), Error> {
    if src.is_empty() {
        return Err(Error::TruncatedFrame { expected: 4, actual: 0 });
    }
    let header = Header::parse(src)?;
    let info = StreamInfo {
        sample_rate: header.sample_rate(),
        channels: header.channels(),
        channel_mode: header.channel_mode,
        blocks: header.blocks.count(),
        subbands: header.subbands.count(),
        bitpool: header.bitpool,
        allocation_method: header.allocation_method,
    };
    let mut decoder = Decoder::new();
    let mut dst = Vec::new();
    let mut offset = 0;
    while offset < src.len() {
        let frame_hdr = Header::parse(&src[offset..])?;
        if frame_hdr.sample_rate() != info.sample_rate || frame_hdr.channels() != info.channels || frame_hdr.channel_mode != info.channel_mode {
            return Err(Error::InvalidSampleRate(frame_hdr.sample_rate()));
        }
        let consumed = decoder.decode_frame(&src[offset..], &mut dst)?;
        if consumed == 0 {
            break;
        }
        offset = offset.saturating_add(consumed);
    }
    Ok((dst, info))
}

/// Encodes 16-bit linear PCM audio into an SBC bitstream using standard high-quality defaults.
pub fn encode(pcm: &[i16], sample_rate: u32, channels: usize, bitpool: u8) -> Result<Vec<u8>, Error> {
    let mut encoder = Encoder::new(
        sample_rate,
        channels,
        16, // standard 16 blocks
        8,  // standard 8 subbands
        bitpool,
        AllocationMethod::Loudness,
    )?;
    let mut dst = Vec::new();
    encoder.encode(pcm, &mut dst)?;
    Ok(dst)
}
