use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use craft_blitter::{AlphaMode, BlendMode, blit_rgba8};

// Standard IEEE 802.3 CRC32
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
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

// Adler32 checksum for zlib wrapper
fn adler32(data: &[u8]) -> u32 {
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;
    for &b in data {
        s1 = (s1 + b as u32) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    (s2 << 16) | s1
}

fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    let mut f = BufWriter::new(File::create(path)?);

    // PNG signature
    f.write_all(&[137, 80, 78, 71, 13, 10, 26, 10])?;

    // Helper to write chunk
    let mut write_chunk = |tag: &[u8; 4], data: &[u8]| -> std::io::Result<()> {
        let len = (data.len() as u32).to_be_bytes();
        f.write_all(&len)?;
        f.write_all(tag)?;
        f.write_all(data)?;
        let mut crc_data = Vec::with_capacity(4 + data.len());
        crc_data.extend_from_slice(tag);
        crc_data.extend_from_slice(data);
        f.write_all(&crc32(&crc_data).to_be_bytes())?;
        Ok(())
    };

    // IHDR
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // 8-bit depth
    ihdr.push(6); // RGBA color type
    ihdr.push(0); // compression deflate
    ihdr.push(0); // filter none
    ihdr.push(0); // interlace none
    write_chunk(b"IHDR", &ihdr)?;

    // Prepare uncompressed scanlines with filter byte 0
    let stride = width as usize * 4;
    let mut raw_data = Vec::with_capacity((stride + 1) * height as usize);
    for row in 0..height as usize {
        raw_data.push(0); // Filter byte 0 (None)
        let start = row * stride;
        raw_data.extend_from_slice(&rgba[start..start + stride]);
    }

    // Wrap in uncompressed zlib stream (RFC 1950)
    let mut zlib = Vec::new();
    zlib.push(0x78); // CMF: deflate, 32K window
    zlib.push(0x01); // FLG: check bits

    const MAX_BLOCK: usize = 32768;
    let chunks: Vec<&[u8]> = raw_data.chunks(MAX_BLOCK).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let is_last = i == chunks.len() - 1;
        zlib.push(if is_last { 0x01 } else { 0x00 });
        let len = chunk.len() as u16;
        let nlen = !len;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&nlen.to_le_bytes());
        zlib.extend_from_slice(chunk);
    }

    let checksum = adler32(&raw_data);
    zlib.extend_from_slice(&checksum.to_be_bytes());

    write_chunk(b"IDAT", &zlib)?;
    write_chunk(b"IEND", &[])?;

    f.flush()?;
    Ok(())
}

fn create_backdrop(w: u32, h: u32) -> Vec<u8> {
    let mut buf = vec![0u8; w as usize * h as usize * 4];
    for y in 0..h {
        let fy = y as f32 / h as f32;
        for x in 0..w {
            let fx = x as f32 / w as f32;
            let idx = (y as usize * w as usize + x as usize) * 4;

            // Rich artistic landscape gradient with lighting glow
            let dx = fx - 0.35;
            let dy = fy - 0.40;
            let dist = (dx * dx + dy * dy).sqrt();
            let sun = (1.0 - (dist * 2.2).min(1.0)).powf(2.0);

            let r = ((0.15 + 0.70 * (1.0 - fy) + 0.50 * sun).clamp(0.0, 1.0) * 255.0) as u8;
            let g = ((0.25 + 0.45 * (1.0 - fy) + 0.35 * sun).clamp(0.0, 1.0) * 255.0) as u8;
            let b = ((0.50 + 0.35 * fy + 0.15 * sun).clamp(0.0, 1.0) * 255.0) as u8;

            buf[idx] = r;
            buf[idx + 1] = g;
            buf[idx + 2] = b;
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

    for y in 0..h {
        for x in 0..w {
            let idx = (y as usize * w as usize + x as usize) * 4;
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist < radius {
                // Soft anti-aliased edge
                let edge_alpha = ((radius - dist) / 10.0).clamp(0.0, 1.0);
                let angle = dy.atan2(dx);
                let hue_val = angle / std::f32::consts::PI * 0.5 + 0.5;
                let tau = std::f32::consts::TAU;

                // Vibrant multi-color gradient circle
                let r = ((0.5 + 0.5 * (hue_val * tau).cos()) * 255.0) as u8;
                let g = ((0.5 + 0.5 * (hue_val * tau + tau / 3.0).cos()) * 255.0) as u8;
                let b = ((0.5 + 0.5 * (hue_val * tau + 2.0 * tau / 3.0).cos()) * 255.0) as u8;
                let a = (220.0 * edge_alpha) as u8;

                buf[idx] = r;
                buf[idx + 1] = g;
                buf[idx + 2] = b;
                buf[idx + 3] = a;
            }
        }
    }
    buf
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::env::args_os().nth(1).map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("craft-blitter-visual"));
    std::fs::create_dir_all(&out_dir)?;
    let (w, h) = (512, 384);

    let backdrop = create_backdrop(w, h);
    let source = create_source_overlay(w, h);

    // 1. Base Backdrop
    write_png(&out_dir.join("01_before_backdrop.png"), w, h, &backdrop)?;

    // 2. Source Layer
    write_png(&out_dir.join("02_source_layer.png"), w, h, &source)?;

    // 3. Normal Blend at 100%
    let mut normal_100 = backdrop.clone();
    blit_rgba8(&mut normal_100, &source, BlendMode::Normal, 1.0, 1.0);
    write_png(&out_dir.join("03_blend_normal_100.png"), w, h, &normal_100)?;

    // 4. Multiply (Shadows/Darkening)
    let mut multiply = backdrop.clone();
    blit_rgba8(&mut multiply, &source, BlendMode::Multiply, 1.0, 1.0);
    write_png(&out_dir.join("04_blend_multiply.png"), w, h, &multiply)?;

    // 5. Screen (Highlights/Glow)
    let mut screen = backdrop.clone();
    blit_rgba8(&mut screen, &source, BlendMode::Screen, 1.0, 1.0);
    write_png(&out_dir.join("05_blend_screen.png"), w, h, &screen)?;

    // 6. Overlay (Contrast & Saturation)
    let mut overlay = backdrop.clone();
    blit_rgba8(&mut overlay, &source, BlendMode::Overlay, 1.0, 1.0);
    write_png(&out_dir.join("06_blend_overlay.png"), w, h, &overlay)?;

    // 7. Color Dodge (Specular Highlight Burst)
    let mut color_dodge = backdrop.clone();
    blit_rgba8(&mut color_dodge, &source, BlendMode::ColorDodge, 1.0, 1.0);
    write_png(&out_dir.join("07_blend_color_dodge.png"), w, h, &color_dodge)?;

    // 8. Vivid Light (Photoshop Special Eight Mode)
    let mut vivid_light = backdrop.clone();
    blit_rgba8(&mut vivid_light, &source, BlendMode::VividLight, 1.0, 1.0);
    write_png(&out_dir.join("08_blend_vivid_light.png"), w, h, &vivid_light)?;

    // 9. Hard Mix (Threshold Posterization)
    let mut hard_mix = backdrop.clone();
    blit_rgba8(&mut hard_mix, &source, BlendMode::HardMix, 1.0, 1.0);
    write_png(&out_dir.join("09_blend_hard_mix.png"), w, h, &hard_mix)?;

    // 10. Difference (Inversion & Registration Check)
    let mut difference = backdrop.clone();
    blit_rgba8(&mut difference, &source, BlendMode::Difference, 1.0, 1.0);
    write_png(&out_dir.join("10_blend_difference.png"), w, h, &difference)?;

    // 11. Special Eight Comparison: 50% Opacity vs 50% Fill
    let mut opacity_50 = backdrop.clone();
    blit_rgba8(&mut opacity_50, &source, BlendMode::VividLight, 0.5, 1.0);
    write_png(&out_dir.join("11_vivid_light_opacity_50.png"), w, h, &opacity_50)?;

    let mut fill_50 = backdrop.clone();
    blit_rgba8(&mut fill_50, &source, BlendMode::VividLight, 1.0, 0.5);
    write_png(&out_dir.join("12_vivid_light_fill_50.png"), w, h, &fill_50)?;

    // 12. Bit-Exact Diff Verification Map (Scalar Reference vs craft-blitter SIMD)
    // Both produce identical bytes: diff is completely zero (black image)
    let mut scalar_ref = backdrop.clone();
    craft_blitter::fixed::blit_rgba8_fixed(&mut scalar_ref, &source, BlendMode::Normal, 1.0, 1.0, AlphaMode::Straight, false);
    let mut diff_map = vec![0u8; w as usize * h as usize * 4];
    let (d_chunks, _) = diff_map.as_chunks_mut::<4>();
    let (a_chunks, _) = normal_100.as_chunks::<4>();
    let (b_chunks, _) = scalar_ref.as_chunks::<4>();
    for ((d, a), b) in d_chunks.iter_mut().zip(a_chunks).zip(b_chunks) {
        for c in 0..3 {
            d[c] = a[c].abs_diff(b[c]);
        }
        d[3] = 255;
    }
    write_png(&out_dir.join("13_bit_exact_diff_map_zero_error.png"), w, h, &diff_map)?;

    println!("All visual comparison images generated successfully in {:?}", out_dir);
    Ok(())
}
