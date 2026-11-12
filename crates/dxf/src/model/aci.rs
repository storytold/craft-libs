//! The 256-colour index palette CAD apps share (the AutoCAD Color Index), generated rather than
//! tabulated: indices 1–9 are the standard colours, 10–249 run through 24 hues at five
//! brightnesses each in a full and a half-saturated tone, and 250–255 are greys. From
//! VectorCraft's `vectorcraft-cad`.

/// The nine standard colours (index 7 shows white on a dark background and black on paper).
const STANDARD: [[u8; 3]; 9] =
    [[255, 0, 0], [255, 255, 0], [0, 255, 0], [0, 255, 255], [0, 0, 255], [255, 0, 255], [255, 255, 255], [128, 128, 128], [192, 192, 192]];

/// The brightness of the five shades of each hue.
const SHADES: [f64; 5] = [1.0, 0.65, 0.5, 0.3, 0.15];

const WHITE: [u8; 3] = [255; 3];

/// The RGB of colour index `i` (1–255; 0 is index 7's white).
#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "the channels computed here are within 0.0..=255.0")]
pub fn rgb(i: u8) -> [u8; 3] {
    match i {
        1..=9 => STANDARD.get(usize::from(i - 1)).copied().unwrap_or(WHITE),
        10..=249 => {
            let hue = f64::from(i / 10 - 1) * 15.0;
            let k = i % 10;
            let v = SHADES.get(usize::from(k / 2)).copied().unwrap_or(1.0);
            let pastel = k % 2 == 1;
            hue_rgb(hue).map(|f| {
                let f = if pastel { f + (1.0 - f) / 2.0 } else { f };
                (255.0 * v * f).floor() as u8
            })
        }
        250..=255 => [(51.0 + 204.0 * f64::from(i - 250) / 5.0).floor() as u8; 3],
        0 => WHITE,
    }
}

/// The fully saturated, full-brightness colour of `hue` degrees as 0–1 channels.
#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "h is within 0.0..6.0")]
fn hue_rgb(hue: f64) -> [f64; 3] {
    let h = hue.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    match h as u8 {
        0 => [1.0, x, 0.0],
        1 => [x, 1.0, 0.0],
        2 => [0.0, 1.0, x],
        3 => [0.0, x, 1.0],
        4 => [x, 0.0, 1.0],
        _ => [1.0, 0.0, x],
    }
}

/// The index among `candidates` nearest to `color` (squared RGB distance; ties go to the earlier
/// candidate), or 7 when there are none. Index 7 stands for both white and black, as CAD apps
/// draw it in the background's contrast. A writer limited to 8 or 16 colours passes those.
pub fn nearest(color: [u8; 3], candidates: impl IntoIterator<Item = u8>) -> u8 {
    let dist = |c: [u8; 3]| color.iter().zip(c).map(|(a, b)| (i32::from(*a) - i32::from(b)).pow(2)).sum::<i32>();
    candidates.into_iter().min_by_key(|i| if *i == 7 { dist(WHITE).min(dist([0; 3])) } else { dist(rgb(*i)) }).unwrap_or(7)
}

#[cfg(test)]
mod tests {
    use super::{nearest, rgb};

    #[test]
    fn the_palette_matches_its_known_entries() {
        let known: [(u8, [u8; 3]); 14] = [
            (1, [255, 0, 0]),
            (10, [255, 0, 0]),
            (11, [255, 127, 127]),
            (12, [165, 0, 0]),
            (13, [165, 82, 82]),
            (17, [76, 38, 38]),
            (19, [38, 19, 19]),
            (20, [255, 63, 0]),
            (21, [255, 159, 127]),
            (30, [255, 127, 0]),
            (140, [0, 191, 255]),
            (250, [51, 51, 51]),
            (252, [132, 132, 132]),
            (255, [255, 255, 255]),
        ];
        for (i, want) in known {
            assert_eq!(rgb(i), want, "index {i}");
        }
    }

    #[test]
    fn nearest_index_among_the_candidates() {
        let sixteen = || (1..=9).chain([30]).chain(250..=255);
        assert_eq!(nearest([255, 0, 0], 1..=8), 1);
        assert_eq!(nearest([0, 0, 0], 1..=255), 7, "black is index 7");
        assert_eq!(nearest([255, 255, 255], sixteen()), 7);
        assert_eq!(nearest([255, 127, 0], 1..=255), 30);
        assert_eq!(nearest([255, 127, 0], sixteen()), 30);
        assert_eq!(nearest([255, 127, 0], 1..=8), 1, "8 colours: red is nearest");
        assert_eq!(nearest([60, 60, 60], sixteen()), 250);
        assert_eq!(nearest([100, 100, 100], 1..=8), 8);
        assert_eq!(nearest([60, 60, 60], 1..=8), 7, "dark greys go to black");
        assert_eq!(nearest([1, 2, 3], []), 7);
    }
}
