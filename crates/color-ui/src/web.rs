//! Web-safe colours: each channel on 0, 0.2, ... 1 (`00`, `33`, `66`, `99`, `CC`, `FF`).

/// `color` with each channel (clamped to `0..=1`) on the nearest web-safe level.
pub fn web_safe(color: [f32; 3]) -> [f32; 3] {
    color.map(|v| (v.clamp(0.0, 1.0) * 5.0).round() / 5.0)
}

/// The nearest web colour of `color`, when `color` is not one; `None` for a web colour. It is
/// what a "not a web colour" warning offers.
pub fn not_web_safe(color: [f32; 3]) -> Option<[f32; 3]> {
    let near = web_safe(color);
    let hex = |c: [f32; 3]| craft_theme::Srgba::from_rgb_array(c).to_hex();
    (hex(near) != hex(color)).then_some(near)
}
