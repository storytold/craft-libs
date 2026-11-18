use super::TRANSPARENT;
use super::rare::RARE_ISLAND;

/// Merge whole 4-connected regions, smallest first, into their most common neighbouring label.
/// The merge order does not depend on `min_area`: a higher threshold continues the same sequence
/// instead of splitting a region already merged at a lower threshold. Transparency is a barrier.
/// It reads only the labels. A trace also reads the image, and a speck far from the color around
/// it joins the color beside it nearest its pixels instead.
pub fn denoise(labels: &mut [u16], w: usize, h: usize, min_area: usize) {
    denoise_keeping(labels, w, h, min_area, &[]);
}

/// How far apart (Oklab × 100) a small region's color and the color it shares the longest
/// border with must be before it merges into the color beside it that is nearest its source
/// pixels instead (among those with at least [`NEAREST_BORDER`] of the longest border): a dark
/// speck on the edge of a light area joins the dark outline beside it, not the light area.
pub(crate) const NEAREST_APART: f64 = 10.0;

/// See [`NEAREST_APART`].
pub(crate) const NEAREST_BORDER: f64 = 0.1;

/// The fewest pixels a region needs for [`NEAREST_APART`] to apply when two or more colors tie
/// for its longest border. Such a lone pixel merges by border alone: on an anti-aliased corner it
/// holds a mix of the colors on both sides, and joining the nearer one can cut the corner off a
/// shape.
pub(crate) const NEAREST_PIXELS: u32 = 2;

/// The colors [`denoise_nearest`] compares a small region with.
pub(crate) struct Source<'a> {
    /// Each label's color in Oklab × 100 ([`super::palette_labs`]).
    pub labs: &'a [[f64; 3]],
    /// The prepared image the labels were made from, 4 bytes a pixel.
    pub rgba: &'a [u8],
    /// The pixels along a cut edge that [`super::settle_cut_edges`] finds to be the color of an opaque pixel beside
    /// them blended with the transparent background. They are not read.
    pub blended: &'a [bool],
    /// Whether the labels are grays (Grayscale, Black and White): the pixels are compared as their gray
    /// ([`super::luma`]).
    pub gray: bool,
}

/// [`denoise`], except that a region of a label `keep` marks (a rare colour kept on purpose, see
/// [`rare_colours`]) that is at least [`RARE_ISLAND`] pixels stays whatever `min_area` is: Noise
/// clears specks, not a colour's only regions. Smaller regions of it go as any other.
pub(crate) fn denoise_keeping(labels: &mut [u16], w: usize, h: usize, min_area: usize, keep: &[bool]) {
    denoise_nearest(labels, w, h, min_area, keep, None);
}

/// [`denoise_keeping`]; with `source`, a region below `min_area` whose color is at least
/// [`NEAREST_APART`] from the color it shares the longest border with merges into the color
/// beside it nearest its source pixels, averaged, among those sharing at least [`NEAREST_BORDER`]
/// of the longest border ([`NEAREST_PIXELS`] limits this for a single pixel). Every region below
/// `min_area` still goes.
pub(crate) fn denoise_nearest(labels: &mut [u16], w: usize, h: usize, min_area: usize, keep: &[bool], source: Option<&Source>) {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    // Every index below is a `u32`, so a pixel costs 8 bytes and a region 18 instead of a `Vec<usize>` per region. An image
    // of `u32::MAX` pixels or more (far above the decoder's cap) is left as it is.
    if min_area <= 1 || w == 0 || h == 0 || w.checked_mul(h) != Some(labels.len()) || labels.len() >= NONE as usize {
        return;
    }
    // Region of each pixel as first found (`NONE` for transparent). A merge does not touch it: the region a pixel
    // belongs to now is `find(at[i])`.
    let mut at = vec![NONE; labels.len()];
    // Pixels of a region form a singly linked list through `next`, so absorbing a region is one pointer write.
    let mut next = vec![NONE; labels.len()];
    let mut head: Vec<u32> = Vec::new();
    let mut tail: Vec<u32> = Vec::new();
    let mut size: Vec<u32> = Vec::new();
    let mut colour: Vec<u16> = Vec::new();
    // Union-find over region ids: a region absorbed into another points at it.
    let mut parent: Vec<u32> = Vec::new();
    let neighbours = |i: usize| {
        let (x, y) = (i % w, i / w);
        [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)]
    };
    for start in 0..labels.len() {
        if at[start] != NONE || labels[start] == TRANSPARENT {
            continue;
        }
        let id = head.len() as u32;
        let label = labels[start];
        at[start] = id;
        let mut last = start;
        let mut count = 1u32;
        let mut cursor = start;
        loop {
            for j in neighbours(cursor).into_iter().flatten() {
                if at[j] == NONE && labels[j] == label {
                    at[j] = id;
                    next[last] = j as u32;
                    last = j;
                    count += 1;
                }
            }
            match next[cursor] {
                NONE => break,
                n => cursor = n as usize,
            }
        }
        head.push(start as u32);
        tail.push(last as u32);
        size.push(count);
        colour.push(label);
        parent.push(id);
    }
    // Only when it can be read: the labels' colors and the source pixels must cover every label and every pixel.
    let source = source.filter(|s| colour.iter().all(|&c| (c as usize) < s.labs.len()) && labels.len().checked_mul(4) == Some(s.rgba.len()));
    // Only regions below `min_area` are ever taken from the queue; a larger one ends the loop.
    let mut pending: BinaryHeap<_> =
        (0..size.len() as u32).filter(|&id| (size[id as usize] as usize) < min_area).map(|id| Reverse((size[id as usize], id))).collect();
    // Scratch for one merge: `(other region, shared pixel edges)` and `(colour, shared pixel edges)`.
    let mut borders: Vec<(u32, usize)> = Vec::new();
    let mut by_colour: Vec<(u16, usize)> = Vec::new();
    while let Some(Reverse((area, id))) = pending.pop() {
        if size[id as usize] != area {
            continue; // absorbed region or an old heap entry for a region that grew
        }
        if area as usize >= RARE_ISLAND && keep.get(colour[id as usize] as usize).copied().unwrap_or(false) {
            continue;
        }
        borders.clear();
        // Pairs are folded together whenever the list doubles, so a big region keeps it short.
        let mut limit = 4096;
        let mut pixel = head[id as usize];
        while pixel != NONE {
            for j in neighbours(pixel as usize).into_iter().flatten() {
                if at[j] != NONE {
                    let other = find(&mut parent, at[j]);
                    if other != id {
                        borders.push((other, 1));
                    }
                }
            }
            pixel = next[pixel as usize];
            if borders.len() >= limit {
                merge_counts(&mut borders);
                limit = limit.max(2 * borders.len());
            }
        }
        merge_counts(&mut borders);
        by_colour.clear();
        by_colour.extend(borders.iter().map(|&(other, border)| (colour[other as usize], border)));
        merge_counts(&mut by_colour);
        let Some(&(mut chosen, longest)) = by_colour.iter().max_by_key(|(label, border)| (*border, Reverse(*label))) else {
            continue; // an opaque island surrounded only by transparency stays intact
        };
        if let Some(source) = source
            && (area >= NEAREST_PIXELS || by_colour.iter().filter(|&&(_, border)| border == longest).count() == 1)
            && let Some(own) = source.labs.get(colour[id as usize] as usize)
            && distance(own, source.labs.get(chosen as usize)) >= NEAREST_APART
            && let Some(mean) = mean_lab(head[id as usize], &next, source)
        {
            let to = |c: u16| distance(&mean, source.labs.get(c as usize));
            if let Some(&(nearest, _)) = by_colour
                .iter()
                .filter(|&&(_, border)| border as f64 >= NEAREST_BORDER * longest as f64)
                .min_by(|a, b| to(a.0).total_cmp(&to(b.0)).then(b.1.cmp(&a.1)))
            {
                chosen = nearest;
            }
        }
        // All adjacent regions of the chosen colour become connected by this merge. Absorb
        // them together, keeping the largest one as the survivor.
        let Some(target) = borders
            .iter()
            .map(|&(other, _)| other)
            .filter(|&other| colour[other as usize] == chosen)
            .max_by_key(|&other| (size[other as usize], Reverse(other)))
        else {
            continue;
        };
        for source in
            std::iter::once(id).chain(borders.iter().map(|&(other, _)| other).filter(|&other| other != target && colour[other as usize] == chosen))
        {
            next[tail[target as usize] as usize] = head[source as usize];
            tail[target as usize] = tail[source as usize];
            size[target as usize] += size[source as usize];
            size[source as usize] = 0;
            parent[source as usize] = target;
        }
        if (size[target as usize] as usize) < min_area {
            pending.push(Reverse((size[target as usize], target)));
        }
    }
    for (i, &region) in at.iter().enumerate() {
        if region != NONE {
            labels[i] = colour[find(&mut parent, region) as usize];
        }
    }
}

/// The end of a pixel list, and the region of a transparent pixel.
const NONE: u32 = u32::MAX;

/// The root of `a` in the union-find `parent`, halving the paths on the way.
fn find(parent: &mut [u32], mut a: u32) -> u32 {
    while parent[a as usize] != a {
        let up = parent[parent[a as usize] as usize];
        parent[a as usize] = up;
        a = up;
    }
    a
}

/// The source pixels of the list that starts at `first` (linked through `next`), averaged, in Oklab × 100, read as
/// `source` says, leaving out the blended cut-edge pixels. `None` when every pixel is one of them.
fn mean_lab(first: u32, next: &[u32], source: &Source) -> Option<[f64; 3]> {
    let (mut sum, mut n) = ([0.0f64; 3], 0u32);
    let mut pixel = first;
    while pixel != NONE {
        let blended = source.blended.get(pixel as usize).copied().unwrap_or(false);
        if !blended && let Some(&[r, g, b, a]) = source.rgba.get(pixel as usize * 4..pixel as usize * 4 + 4) {
            let p = if source.gray { [f64::from(super::luma([r, g, b, a])); 3] } else { [r, g, b].map(f64::from) };
            for (s, v) in sum.iter_mut().zip(p) {
                *s += v;
            }
            n += 1;
        }
        pixel = next.get(pixel as usize).copied().unwrap_or(NONE);
    }
    if n == 0 {
        return None;
    }
    Some(super::oklab(sum.map(|s| s / f64::from(n))))
}

/// The distance between two colors in Oklab × 100; infinite when `q` is missing.
fn distance(p: &[f64; 3], q: Option<&[f64; 3]>) -> f64 {
    q.map_or(f64::INFINITY, |q| (0..3).map(|k| (p[k] - q[k]).powi(2)).sum::<f64>().sqrt())
}

/// Sort `(key, count)` pairs and add up the counts of equal keys.
fn merge_counts<K: Ord + Copy>(pairs: &mut Vec<(K, usize)>) {
    pairs.sort_unstable_by_key(|p| p.0);
    pairs.dedup_by(|b, a| {
        let same = a.0 == b.0;
        if same {
            a.1 += b.1;
        }
        same
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first implementation (a pixel `Vec` per region, `BTreeMap` borders), kept to check that the compact one merges
    /// in exactly the same order.
    fn denoise_reference(labels: &mut [u16], w: usize, h: usize, min_area: usize, keep: &[bool]) {
        use std::cmp::Reverse;
        use std::collections::{BTreeMap, BinaryHeap};

        if min_area <= 1 || w == 0 || h == 0 || w.checked_mul(h) != Some(labels.len()) {
            return;
        }
        struct Region {
            label: u16,
            pixels: Vec<usize>,
        }
        // Region membership is kept current after every merge. In particular, no stale `seen`
        // flags can exclude previously recoloured pixels from a subsequent connected component.
        let mut at = vec![usize::MAX; labels.len()];
        let mut regions: Vec<Region> = Vec::new();
        let neighbours = |i: usize| {
            let (x, y) = (i % w, i / w);
            [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)]
        };
        for start in 0..labels.len() {
            if at[start] != usize::MAX || labels[start] == TRANSPARENT {
                continue;
            }
            let id = regions.len();
            let label = labels[start];
            let mut pixels = vec![start];
            at[start] = id;
            let mut cursor = 0;
            while let Some(&i) = pixels.get(cursor) {
                for j in neighbours(i).into_iter().flatten() {
                    if at[j] == usize::MAX && labels[j] == label {
                        at[j] = id;
                        pixels.push(j);
                    }
                }
                cursor += 1;
            }
            regions.push(Region { label, pixels });
        }
        let mut pending: BinaryHeap<_> = regions.iter().enumerate().map(|(id, r)| Reverse((r.pixels.len(), id))).collect();
        while let Some(Reverse((area, id))) = pending.pop() {
            if area >= min_area {
                break;
            }
            if regions[id].pixels.len() != area {
                continue; // absorbed region or an old heap entry for a region that grew
            }
            if area >= RARE_ISLAND && keep.get(regions[id].label as usize).copied().unwrap_or(false) {
                continue;
            }
            let mut borders: BTreeMap<usize, usize> = BTreeMap::new();
            for &i in &regions[id].pixels {
                for j in neighbours(i).into_iter().flatten() {
                    let other = at[j];
                    if other != id && other != usize::MAX {
                        *borders.entry(other).or_default() += 1;
                    }
                }
            }
            let mut colours: BTreeMap<u16, usize> = BTreeMap::new();
            for (&other, &border) in &borders {
                *colours.entry(regions[other].label).or_default() += border;
            }
            let Some((&colour, _)) = colours.iter().max_by_key(|(label, border)| (**border, Reverse(**label))) else {
                continue; // an opaque island surrounded only by transparency stays intact
            };
            // All adjacent regions of the chosen colour become connected by this merge. Absorb
            // them together, keeping the largest one's storage to avoid repeatedly moving it.
            let Some(target) = borders
                .keys()
                .copied()
                .filter(|&other| regions[other].label == colour)
                .max_by_key(|&other| (regions[other].pixels.len(), Reverse(other)))
            else {
                continue;
            };
            let sources: Vec<_> =
                std::iter::once(id).chain(borders.keys().copied().filter(|&other| other != target && regions[other].label == colour)).collect();
            for source in sources {
                let pixels = std::mem::take(&mut regions[source].pixels);
                for &i in &pixels {
                    at[i] = target;
                }
                regions[target].pixels.extend(pixels);
            }
            pending.push(Reverse((regions[target].pixels.len(), target)));
        }
        for region in regions {
            for i in region.pixels {
                labels[i] = region.label;
            }
        }
    }

    fn noisy(w: usize, h: usize, colours: u64, transparent_one_in: u64, seed: u64) -> Vec<u16> {
        let mut state = seed;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        (0..w * h).map(|_| if transparent_one_in > 0 && next() % transparent_one_in == 0 { TRANSPARENT } else { (next() % colours) as u16 }).collect()
    }

    #[test]
    fn compact_denoise_matches_the_reference_merge_order() {
        for (seed, (w, h, colours, transparent)) in
            [(31, 17, 3, 0), (64, 64, 2, 0), (50, 70, 5, 9), (97, 41, 8, 0), (120, 90, 4, 5), (7, 300, 3, 0), (1, 40, 3, 0), (40, 1, 3, 0)]
                .into_iter()
                .enumerate()
        {
            let base = noisy(w, h, colours, transparent, seed as u64 + 1);
            for min_area in [2, 3, 5, 9, 25, 80, 400, 100_000] {
                for keep in [&[][..], &[false, true, false, true][..]] {
                    let (mut a, mut b) = (base.clone(), base.clone());
                    denoise_reference(&mut a, w, h, min_area, keep);
                    denoise_keeping(&mut b, w, h, min_area, keep);
                    assert_eq!(a, b, "{w}x{h}, {colours} colours, min_area {min_area}, keep {keep:?}");
                }
            }
        }
    }

    /// A light ground, a dark ink, a tone barely darker than the ground and a dark brown close to the ink.
    const GROUND_INK_TONE_BROWN: [[u8; 3]; 4] = [[235, 225, 210], [30, 25, 20], [228, 218, 203], [50, 42, 35]];

    /// `labels` denoised at `min_area`, reading the source image that paints each label its palette color.
    fn denoised_nearest(labels: &[u16], w: usize, h: usize, min_area: usize) -> Vec<u16> {
        let labs = crate::quantize::palette_labs(&GROUND_INK_TONE_BROWN);
        let rgba: Vec<u8> =
            labels.iter().flat_map(|&l| GROUND_INK_TONE_BROWN.get(l as usize).copied().unwrap_or([0; 3]).into_iter().chain([255])).collect();
        let mut out = labels.to_vec();
        denoise_nearest(&mut out, w, h, min_area, &[], Some(&Source { labs: &labs, rgba: &rgba, blended: &[], gray: false }));
        out
    }

    /// A ground with a dark line two pixels wide down column 12 and 13, and a 2 × 2 speck of `speck` at (10, 10) that
    /// touches the line on one side and the ground on three.
    fn speck_beside_a_line(speck: u16) -> (Vec<u16>, usize, usize) {
        let (w, h) = (30, 30);
        let mut labels = vec![0u16; w * h];
        for y in 0..h {
            labels[y * w + 12] = 1;
            labels[y * w + 13] = 1;
        }
        for (x, y) in SPECK {
            labels[y * w + x] = speck;
        }
        (labels, w, h)
    }

    const SPECK: [(usize, usize); 4] = [(10, 10), (11, 10), (10, 11), (11, 11)];

    /// A dark speck mostly on the light ground but touching a dark line joins the line, the color nearest it, not the
    /// ground it shares more border with.
    #[test]
    fn a_dark_speck_beside_a_dark_line_joins_the_line() {
        let (labels, w, h) = speck_beside_a_line(3);
        let out = denoised_nearest(&labels, w, h, 15);
        for (x, y) in SPECK {
            assert_eq!(out[y * w + x], 1, "({x}, {y})");
        }
        let mut blind = labels.clone();
        denoise_keeping(&mut blind, w, h, 15, &[]);
        assert_eq!(blind[10 * w + 10], 0, "by border alone it joins the ground");
    }

    /// A speck whose color is close to the color it shares the longest border with joins that color, as before,
    /// even though another color touches it.
    #[test]
    fn a_speck_close_to_its_longest_border_color_joins_it() {
        let (labels, w, h) = speck_beside_a_line(2);
        let out = denoised_nearest(&labels, w, h, 15);
        let mut blind = labels.clone();
        denoise_keeping(&mut blind, w, h, 15, &[]);
        assert_eq!(out, blind);
        assert_eq!(out[10 * w + 10], 0);
    }

    /// A color that touches the speck along less than a tenth of its longest border is not chosen, however near.
    #[test]
    fn a_color_with_a_short_border_is_not_chosen() {
        let (w, h) = (30, 30);
        let mut labels = vec![0u16; w * h];
        // A 3 × 3 speck of brown in the ground, and a line of ink 16 pixels long (above Noise) that touches the speck at
        // one pixel edge.
        for y in 10..13 {
            for x in 10..13 {
                labels[y * w + x] = 3;
            }
        }
        for x in 13..29 {
            labels[10 * w + x] = 1;
        }
        // The brown speck has 11 pixel edges against the ground and 1 against the ink: 1 < 0.1 × 11.
        let out = denoised_nearest(&labels, w, h, 15);
        assert_eq!(out[11 * w + 11], 0);
        assert!(out[10 * w + 13..10 * w + 29].iter().all(|&l| l == 1), "the line stays");
    }

    /// A lone pixel tied between two colors for its longest border merges by border alone, even when the other
    /// color is nearer its source pixel.
    #[test]
    fn a_lone_pixel_tied_between_colors_merges_by_border() {
        let (w, h) = (20, 20);
        // Ground in the top-left, ink everywhere else, and one brown pixel at the corner with two edges on each.
        let mut labels: Vec<u16> = (0..w * h).map(|i| u16::from(i % w >= 11 || i / w >= 11)).collect();
        labels[10 * w + 10] = 3;
        let out = denoised_nearest(&labels, w, h, 5);
        let mut blind = labels.clone();
        denoise_keeping(&mut blind, w, h, 5, &[]);
        assert_eq!(out, blind);
        assert_eq!(out[10 * w + 10], 0);
        // With the ground one column wider the pixel has three edges on the ground and one on the ink: no longer tied,
        // it joins the ink, the color nearest it.
        for y in 0..11 {
            labels[y * w + 11] = 0;
        }
        let out = denoised_nearest(&labels, w, h, 5);
        assert_eq!(out[10 * w + 10], 1);
    }

    /// Every region below Noise still goes: the nearest-color merge only changes where it goes.
    #[test]
    fn noise_still_removes_every_region_below_it() {
        let (w, h) = (60, 50);
        for seed in 1..6 {
            let labels = noisy(w, h, 4, 0, seed);
            for min_area in [2, 3, 5, 8, 15, 25] {
                let out = denoised_nearest(&labels, w, h, min_area);
                let mut seen = vec![false; w * h];
                for start in 0..w * h {
                    if seen[start] {
                        continue;
                    }
                    seen[start] = true;
                    let mut stack = vec![start];
                    let mut count = 0;
                    while let Some(i) = stack.pop() {
                        count += 1;
                        let (x, y) = (i % w, i / w);
                        for j in [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)]
                            .into_iter()
                            .flatten()
                        {
                            if !seen[j] && out[j] == out[i] {
                                seen[j] = true;
                                stack.push(j);
                            }
                        }
                    }
                    assert!(count >= min_area, "seed {seed}, Noise {min_area}: a region of {count} pixels at {start}");
                }
            }
        }
    }

    /// Without colors or a source image for every label and pixel, every region merges as it always did.
    #[test]
    fn a_source_that_does_not_cover_the_labels_is_not_read() {
        let (labels, w, h) = speck_beside_a_line(3);
        let mut blind = labels.clone();
        denoise_keeping(&mut blind, w, h, 15, &[]);
        let labs = crate::quantize::palette_labs(&GROUND_INK_TONE_BROWN[..2]);
        // Every pixel ink: read, the speck would join the line.
        let rgba: Vec<u8> = (0..w * h).flat_map(|_| [30, 25, 20, 255]).collect();
        let mut out = labels.clone();
        denoise_nearest(&mut out, w, h, 15, &[], Some(&Source { labs: &labs, rgba: &rgba, blended: &[], gray: false }));
        assert_eq!(out, blind);
        let labs = crate::quantize::palette_labs(&GROUND_INK_TONE_BROWN);
        let mut out = labels.clone();
        denoise_nearest(&mut out, w, h, 15, &[], Some(&Source { labs: &labs, rgba: &rgba[4..], blended: &[], gray: false }));
        assert_eq!(out, blind);
        let mut out = labels.clone();
        denoise_nearest(&mut out, w, h, 15, &[], Some(&Source { labs: &labs, rgba: &rgba, blended: &[], gray: false }));
        assert_eq!(out[10 * w + 10], 1, "a source that covers them is read");
    }

    /// A speck made only of cut-edge pixels that are the color beside them blended with the transparent background
    /// merges by border alone.
    #[test]
    fn a_speck_of_blended_cut_edge_pixels_merges_by_border() {
        let (labels, w, h) = speck_beside_a_line(3);
        let labs = crate::quantize::palette_labs(&GROUND_INK_TONE_BROWN);
        let rgba: Vec<u8> =
            labels.iter().flat_map(|&l| GROUND_INK_TONE_BROWN.get(l as usize).copied().unwrap_or([0; 3]).into_iter().chain([255])).collect();
        let mut blended = vec![false; w * h];
        for (x, y) in SPECK {
            blended[y * w + x] = true;
        }
        let mut out = labels.clone();
        denoise_nearest(&mut out, w, h, 15, &[], Some(&Source { labs: &labs, rgba: &rgba, blended: &blended, gray: false }));
        let mut blind = labels.clone();
        denoise_keeping(&mut blind, w, h, 15, &[]);
        assert_eq!(out, blind);
        // One pixel that is not such a blend is enough to read it.
        blended[10 * w + 10] = false;
        let mut out = labels.clone();
        denoise_nearest(&mut out, w, h, 15, &[], Some(&Source { labs: &labs, rgba: &rgba, blended: &blended, gray: false }));
        assert_eq!(out[10 * w + 10], 1);
    }

    /// A minimum so large that regions keep merging into big ones.
    #[test]
    fn compact_denoise_matches_the_reference_with_a_large_min_area() {
        let (w, h) = (320, 320);
        let base = noisy(w, h, 6, 0, 99);
        let (mut a, mut b) = (base.clone(), base);
        denoise_reference(&mut a, w, h, 50_000, &[]);
        denoise_keeping(&mut b, w, h, 50_000, &[]);
        assert_eq!(a, b);
    }
}
