//! The crate's integration tests, linked into one test binary.

mod bmp_resolution;
mod camera_raw;
mod common;
#[cfg(feature = "dds")]
mod dds;
#[cfg(feature = "dds")]
mod dds_oracle;
#[cfg(feature = "dds")]
mod dds_write;
mod decode_warnings;
#[cfg(feature = "exr")]
mod deep_exr;
mod detect_caps;
#[cfg(feature = "dpx")]
mod dpx;
#[cfg(feature = "exr")]
mod exr_cryptomatte;
#[cfg(feature = "exr")]
mod exr_multipart;
#[cfg(feature = "exr")]
mod exr_stream;
mod fidelity;
mod gif_frames;
mod heif;
mod jpeg_inks;
#[cfg(feature = "jpegls")]
mod jpegls_oracle;
mod limits;
mod malformed;
mod metadata;
mod orientation;
mod pcx;
mod png_jpeg;
#[cfg(feature = "pnm")]
mod pnm_headers;
mod resolution;
mod roundtrip;
mod source_settings;
#[cfg(feature = "tga")]
mod tga;
mod tiff_bands;
mod tiff_bench;
mod tiff_exif;
mod tiff_lab;
mod tiff_pages;
mod tiff_photoshop;
mod tiff_pyramid;
mod tiff_regressions;
mod tiff_ycbcr;
#[cfg(feature = "webp")]
mod webp_frames;
#[cfg(feature = "webp")]
mod webp_lossy;
#[cfg(feature = "xpm")]
mod xpm;
