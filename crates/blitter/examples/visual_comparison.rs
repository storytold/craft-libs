use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use craft_blitter::{AlphaMode, BlendMode, CompositeOp, blit_rgba8, blit_rgba8_atop};

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;
    for &b in data {
        s1 = (s1 + u32::from(b)) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    (s2 << 16) | s1
}

fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    let mut f = BufWriter::new(File::create(path)?);
    f.write_all(&[137, 80, 78, 71, 13, 10, 26, 10])?;

    let mut write_chunk = |tag: &[u8; 4], data: &[u8]| -> std::io::Result<()> {
        f.write_all(&(data.len() as u32).to_be_bytes())?;
        f.write_all(tag)?;
        f.write_all(data)?;
        let mut crc_data = Vec::with_capacity(4 + data.len());
        crc_data.extend_from_slice(tag);
        crc_data.extend_from_slice(data);
        f.write_all(&crc32(&crc_data).to_be_bytes())
    };

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    write_chunk(b"IHDR", &ihdr)?;

    let stride = width as usize * 4;
    let mut raw_data = Vec::with_capacity((stride + 1) * height as usize);
    for row in 0..height as usize {
        raw_data.push(0);
        let start = row * stride;
        raw_data.extend_from_slice(&rgba[start..start + stride]);
    }

    let mut zlib = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = raw_data.chunks(32768).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        zlib.push(u8::from(i + 1 == chunks.len()));
        let len = chunk.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(chunk);
    }
    zlib.extend_from_slice(&adler32(&raw_data).to_be_bytes());

    write_chunk(b"IDAT", &zlib)?;
    write_chunk(b"IEND", &[])?;
    f.flush()
}

fn create_backdrop(w: u32, h: u32) -> Vec<u8> {
    let mut buf = vec![0u8; w as usize * h as usize * 4];
    for y in 0..h {
        let fy = y as f32 / h as f32;
        for x in 0..w {
            let fx = x as f32 / w as f32;
            let idx = (y as usize * w as usize + x as usize) * 4;
            let dx = fx - 0.35;
            let dy = fy - 0.40;
            let dist = (dx * dx + dy * dy).sqrt();
            let sun = (1.0 - (dist * 2.2).min(1.0)).powi(2);

            buf[idx] = ((0.15 + 0.70 * (1.0 - fy) + 0.50 * sun).clamp(0.0, 1.0) * 255.0) as u8;
            buf[idx + 1] = ((0.25 + 0.45 * (1.0 - fy) + 0.35 * sun).clamp(0.0, 1.0) * 255.0) as u8;
            buf[idx + 2] = ((0.50 + 0.35 * fy + 0.15 * sun).clamp(0.0, 1.0) * 255.0) as u8;
            buf[idx + 3] = 255;
        }
    }
    buf
}

fn create_source_overlay(w: u32, h: u32) -> Vec<u8> {
    let mut buf = vec![0u8; w as usize * h as usize * 4];
    let cx = w as f32 * 0.5;
    let cy = h as f32 * 0.5;
    let radius = w.min(h) as f32 * 0.38;
    let tau = std::f32::consts::TAU;

    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < radius {
                let idx = (y as usize * w as usize + x as usize) * 4;
                let edge_alpha = ((radius - dist) / 10.0).clamp(0.0, 1.0);
                let hue_val = dy.atan2(dx) / std::f32::consts::PI * 0.5 + 0.5;

                buf[idx] = ((0.5 + 0.5 * (hue_val * tau).cos()) * 255.0) as u8;
                buf[idx + 1] = ((0.5 + 0.5 * (hue_val * tau + tau / 3.0).cos()) * 255.0) as u8;
                buf[idx + 2] = ((0.5 + 0.5 * (hue_val * tau + 2.0 * tau / 3.0).cos()) * 255.0) as u8;
                buf[idx + 3] = (220.0 * edge_alpha) as u8;
            }
        }
    }
    buf
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::env::args_os().nth(1).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("craft-blitter-visual"));
    std::fs::create_dir_all(&out_dir)?;
    let (w, h) = (512, 384);

    let backdrop = create_backdrop(w, h);
    let source = create_source_overlay(w, h);
    write_png(&out_dir.join("01_backdrop.png"), w, h, &backdrop)?;
    write_png(&out_dir.join("02_source.png"), w, h, &source)?;

    let samples = [
        ("03_normal.png", BlendMode::Normal),
        ("04_multiply.png", BlendMode::Multiply),
        ("05_screen.png", BlendMode::Screen),
        ("06_overlay.png", BlendMode::Overlay),
        ("07_color_dodge.png", BlendMode::ColorDodge),
        ("08_vivid_light.png", BlendMode::VividLight),
        ("09_hard_mix.png", BlendMode::HardMix),
        ("10_difference.png", BlendMode::Difference),
    ];
    for (filename, mode) in samples {
        let mut out = backdrop.clone();
        blit_rgba8(&mut out, &source, mode, 1.0);
        write_png(&out_dir.join(filename), w, h, &out)?;
    }

    let mut atop = backdrop.clone();
    blit_rgba8_atop(&mut atop, &source, BlendMode::Multiply, 0.75);
    write_png(&out_dir.join("11_multiply_atop.png"), w, h, &atop)?;

    let mut par_out = backdrop.clone();
    blit_rgba8(&mut par_out, &source, BlendMode::Normal, 1.0);
    let mut serial_out = backdrop;
    craft_blitter::fixed::blit_rgba8_fixed(&mut serial_out, &source, BlendMode::Normal, 1.0, AlphaMode::Straight, CompositeOp::SourceOver);
    assert_eq!(par_out, serial_out);

    Ok(())
}
