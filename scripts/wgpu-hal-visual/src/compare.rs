use image::{GrayImage, Luma, Rgb, RgbImage};
use serde_json::{Value, json};
use std::path::Path;

const MARGIN: u32 = 40;
const RED_ALPHA: u32 = 104;

fn load(path: &Path) -> Result<RgbImage, String> {
    let reader = image::ImageReader::open(path).map_err(|e| e.to_string())?;
    let (width, height) = reader.into_dimensions().map_err(|e| e.to_string())?;
    if width <= MARGIN * 2 || height <= MARGIN * 2 || u64::from(width) * u64::from(height) > 32_000_000 {
        return Err(format!("Invalid image size: {}", path.display()));
    }
    image::open(path).map(|im| im.into_rgb8()).map_err(|e| e.to_string())
}

fn difference(a: &RgbImage, b: &RgbImage) -> Result<(GrayImage, u64), String> {
    if a.dimensions() != b.dimensions() {
        return Err("Image sizes differ".into());
    }
    let mut mask = GrayImage::new(a.width(), a.height());
    let mut count = 0;
    for ((left, right), pixel) in a.pixels().zip(b.pixels()).zip(mask.pixels_mut()) {
        if left != right {
            *pixel = Luma([255]);
            count += 1;
        }
    }
    Ok((mask, count))
}

fn overlay(after: &RgbImage, mask: &GrayImage) -> RgbImage {
    let mut result = after.clone();
    for (pixel, changed) in result.pixels_mut().zip(mask.pixels()) {
        if changed[0] != 0 {
            let keep = 255 - RED_ALPHA;
            *pixel = Rgb([
                ((u32::from(pixel[0]) * keep + 255 * RED_ALPHA + 127) / 255) as u8,
                ((u32::from(pixel[1]) * keep + 127) / 255) as u8,
                ((u32::from(pixel[2]) * keep + 127) / 255) as u8,
            ]);
        }
    }
    result
}

fn verdict(counts: &[(u64, u64, bool)]) -> (&'static str, u8) {
    if counts.is_empty() {
        return ("INCONCLUSIVE", 2);
    }
    if counts.iter().any(|(_, after, _)| *after != 0) {
        return ("FAIL", 1);
    }
    // No unpatched reproduction means no evidence that this test exercises
    // the bug. Never turn a zero/zero comparison into a successful regression.
    if counts.iter().any(|(before, _, maximized)| !maximized && *before == 0) {
        return ("INCONCLUSIVE", 2);
    }
    ("PASS", 0)
}

fn metadata(root: &Path, side: &str) -> Result<Value, String> {
    let path = root.join(side).join("capture.json");
    if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 64 * 1024 {
        return Err("Excessive capture metadata".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let data: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if data["status"] != "CAPTURED" || data["states"].as_array().map(|a| a.len()) != Some(4) {
        return Err("Capture did not complete all four window states".into());
    }
    Ok(data)
}

fn observed(root: &Path, side: &str, state: &str, info: &Value) -> Result<(RgbImage, RgbImage), String> {
    let source = root.join(side);
    let raw = load(&source.join(format!("{state}-internal.png")))?;
    let display = load(&source.join(format!("{state}-presented.png")))?;
    let x = info["client_offset"][0].as_i64().ok_or("Missing client X offset")? + i64::from(MARGIN);
    let y = info["client_offset"][1].as_i64().ok_or("Missing client Y offset")? + i64::from(MARGIN);
    let width = raw.width() - 2 * MARGIN;
    let height = raw.height() - 2 * MARGIN;
    if x < 0 || y < 0 || x + i64::from(width) > i64::from(display.width()) || y + i64::from(height) > i64::from(display.height()) {
        return Err("Reported client offset puts the crop outside the capture".into());
    }
    let reference = image::imageops::crop_imm(&raw, MARGIN, MARGIN, width, height).to_image();
    let captured = image::imageops::crop_imm(&display, x as u32, y as u32, width, height).to_image();
    Ok((reference, captured))
}

pub fn run(root: &Path) -> Result<u8, String> {
    let before = metadata(root, "before")?;
    let after = metadata(root, "after")?;
    if before["adapter"] != after["adapter"] {
        return Err("A/B adapters or drivers differ".into());
    }
    let mut rows = Vec::new();
    let mut counts = Vec::new();
    let mut table = String::from("| Window | Without patch | With patch |\n| --- | ---: | ---: |\n");
    for (index, state) in super::STATES.iter().enumerate() {
        let a_info = &before["states"][index];
        let b_info = &after["states"][index];
        if a_info["state"] != *state || b_info["state"] != *state || a_info["pixels_per_point"] != b_info["pixels_per_point"] {
            return Err("A/B window states or scale factors differ".into());
        }
        let (a_ref, a) = observed(root, "before", state, a_info)?;
        let (b_ref, b) = observed(root, "after", state, b_info)?;
        if difference(&a_ref, &b_ref)?.1 != 0 {
            return Err("Internal A/B framebuffers differ: scene comparison invalid".into());
        }
        let a_count = difference(&a_ref, &a)?.1;
        let b_count = difference(&b_ref, &b)?.1;
        let (mask, changed) = difference(&a, &b)?;
        let folder = root.join(state);
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        for (name, image) in [("01-before.png", a), ("02-after.png", b.clone()), ("03-difference.png", overlay(&b, &mask))] {
            image.save(folder.join(name)).map_err(|e| e.to_string())?;
            if index == 0 {
                image.save(root.join(name)).map_err(|e| e.to_string())?;
            }
        }
        let row = json!({
            "state": state, "before": a_count, "after": b_count,
            "ab_different_pixels": changed, "compared_pixels": u64::from(mask.width()) * u64::from(mask.height()),
            "before_offset": a_info["client_offset"], "after_offset": b_info["client_offset"],
        });
        println!("{row}");
        rows.push(row);
        counts.push((a_count, b_count, *state == "maximized"));
        table.push_str(&format!("| {state} | {a_count} | {b_count} |\n"));
    }
    let (status, code) = verdict(&counts);
    let explanation = match code {
        0 => "The unpatched restored/resized output reproduced the bug. Patched output matched its framebuffer exactly in all tested states.",
        2 => "The unpatched bug did not reproduce in every required state. This run does not validate the fix; zero/zero is not a regression pass.",
        _ => "Patched output differed from its framebuffer. Inspect the captures before treating this as a graphics regression or capture failure.",
    };
    let report = json!({
        "status": status, "explanation": explanation, "adapter": after["adapter"],
        "renderer": after["renderer"], "states": rows, "difference_threshold": 0,
        "edge_margin": MARGIN, "red_opacity": RED_ALPHA as f32 / 255.0,
    });
    std::fs::write(root.join("results.json"), serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let markdown = format!(
        "# DX12 presentation: {status}\n\n{explanation}\n\n{table}\nThe counts compare Windows captures with identical internal GPU framebuffers. Only the reported integer client offset and a 40-pixel edge crop are applied; no resizing or nonzero threshold.\n\n[Before](01-before.png) · [After](02-after.png) · [Red difference overlay](03-difference.png)\n\n![Difference](03-difference.png)\n\nThis run does not test mouse-drag resizing, mixed-DPI monitor moves or composition targets.\n"
    );
    std::fs::write(root.join("RESULTS.md"), markdown).map_err(|e| e.to_string())?;
    println!("{status}: {explanation}");
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_zero_is_inconclusive() {
        assert_eq!(verdict(&[(0, 0, false), (0, 0, true)]), ("INCONCLUSIVE", 2));
    }

    #[test]
    fn missing_measurements_cannot_pass() {
        assert_eq!(verdict(&[]), ("INCONCLUSIVE", 2));
    }

    #[test]
    fn patched_difference_fails_even_when_baseline_reproduces() {
        assert_eq!(verdict(&[(500, 1, false)]), ("FAIL", 1));
    }

    #[test]
    fn restored_reproduction_and_exact_patch_pass() {
        assert_eq!(verdict(&[(500, 0, false), (0, 0, true)]), ("PASS", 0));
    }

    #[test]
    fn one_channel_change_is_counted_and_masked() {
        let before = RgbImage::from_pixel(1, 1, Rgb([0, 255, 255]));
        let after = RgbImage::from_pixel(1, 1, Rgb([1, 255, 255]));
        let (mask, count) = difference(&before, &after).expect("same size");
        assert_eq!(count, 1);
        assert_eq!(overlay(&before, &mask).get_pixel(0, 0), &Rgb([104, 151, 151]));
    }

    #[test]
    fn different_image_sizes_are_rejected() {
        assert!(difference(&RgbImage::new(1, 1), &RgbImage::new(2, 1)).is_err());
    }
}
