//! Test-only fake backends. They are not shipped API: they show that the core and the `Backend`
//! trait take three differently shaped scanner stacks without a WIA-ism, and they are what the
//! contract tests drive through the public registry.
//!
//! * [`wia_like::WiaLike`]: native ids `{GUID}\0000`, HRESULT errors, BMP pages, status bits.
//! * [`sane_like::SaneLike`]: no preflight status, JPEG pages, an empty feeder known only when the
//!   scan fails (`SANE_STATUS_NO_DOCS`).
//! * [`escl_like::EsclLike`]: a status read with an ADF state, JPEG documents served one by one.
#![allow(dead_code)]

pub mod escl_like;
pub mod sane_like;
pub mod wia_like;

use craft_scan::{Connection, HandlingStatus, Reported, ScanSettings, Scanner, Source};

/// A scanner row with nothing published but its identity; tests set what they need.
pub fn scanner(id: &str, name: &str) -> Scanner {
    Scanner {
        id: id.to_string(),
        name: name.to_string(),
        feeder: false,
        flatbed: true,
        duplex: false,
        feeder_ready: None,
        handling_status: HandlingStatus::default(),
        feeder_capacity: None,
        connection: Connection::Unknown,
        port: None,
        optical_x: None,
        optical_y: None,
        pages: None,
        bed: None,
        sheet: None,
        glass: None,
        feeder_choices: None,
        reported: Reported::default(),
    }
}

/// Settings for `device` scanning `pages` pages from `source`, everything else default.
pub fn request(device: &str, source: Source, pages: u32) -> ScanSettings {
    ScanSettings { device: device.to_string(), source, pages, ..ScanSettings::default() }
}

/// A small uncompressed 24-bit BMP: left half black, right half white.
pub fn bmp_24(width: u32, height: u32) -> Vec<u8> {
    let stride = (width * 3).div_ceil(4) * 4;
    let size = 54 + stride * height;
    let mut out = Vec::new();
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for _ in 0..height {
        let mut row = Vec::new();
        for x in 0..width {
            let value = if x < width / 2 { 0 } else { 255 };
            row.extend_from_slice(&[value, value, value]);
        }
        row.resize(stride as usize, 0);
        out.extend_from_slice(&row);
    }
    out
}

/// A small JPEG, grey, left half dark and right half light.
pub fn jpeg_gray(width: u32, height: u32) -> Vec<u8> {
    let image = image::GrayImage::from_fn(width, height, |x, _| image::Luma([if x < width / 2 { 20 } else { 235 }]));
    let mut out = Vec::new();
    // Encoding into memory cannot fail for a small grey image.
    let _ = image.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg);
    out
}
