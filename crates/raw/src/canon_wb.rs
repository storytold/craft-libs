//! Canon `ColorData` white-balance records shared by the CR2 and CR3 decoders: plausible levels as multipliers, and
//! the planckian presets (Daylight, Shade, Cloudy, Tungsten, Flash) as a Kelvin curve. Record layout from the ExifTool
//! Canon tag-name documentation and observation of files; see [`crate::cr2`] for the per-version positions.

/// WB multipliers (R, G, B; G = 1) from one set of levels, when they are plausible.
pub(crate) fn wb_from_levels(r: f32, g1: f32, g2: f32, b: f32) -> Option<[f32; 3]> {
    if g1 < 256.0 || g2 < 256.0 || r < 64.0 || b < 64.0 || g1 > 16384.0 || (g1 - g2).abs() > 0.05 * g1 {
        return None;
    }
    let g = (g1 + g2) / 2.0;
    let (mr, mb) = (r / g, b / g);
    ((0.25..6.0).contains(&mr) && (0.25..6.0).contains(&mb)).then_some([mr, 1.0, mb])
}

/// The five preset records (Daylight, Shade, Cloudy, Tungsten, Flash) of a `ColorData` array whose Daylight record
/// starts at word `daylight` and Flash record at `flash`: Daylight, Shade, Cloudy and Tungsten follow each other
/// in 5-word records `R G G B Kelvin`. Also used by CR3 (`ColorData` words of the same record layout).
pub(crate) const fn preset_records(daylight: usize, flash: usize) -> [(usize, usize); 5] {
    [(daylight, daylight + 4), (daylight + 5, daylight + 9), (daylight + 10, daylight + 14), (daylight + 15, daylight + 19), (flash, flash + 4)]
}

/// The Kelvin curve of the preset records at `rows` ((levels word, temperature word) of each) in `v`.
pub(crate) fn presets_at(v: &[u64], rows: &[(usize, usize); 5]) -> Option<crate::kelvin::KelvinPresets> {
    let mut points = Vec::with_capacity(rows.len());
    for &(at, temp_at) in rows {
        let q = v.get(at..at + 4)?;
        let temp = *v.get(temp_at)? as f32;
        if let Some(wb) = wb_from_levels(q[0] as f32, q[1] as f32, q[2] as f32, q[3] as f32)
            && (1000.0..=20000.0).contains(&temp)
        {
            points.push((temp, wb));
        }
    }
    crate::kelvin::KelvinPresets::from_unordered(points)
}
