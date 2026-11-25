use super::palette::{Bin, HistKey, TracedAs, Used, labs, nearest_lab};
use super::{Raster, oklab};

/// Automatic: the most rare colours added to a flat picture's count.
const RARE_BUDGET: usize = 6;

/// How far apart (Oklab × 100) a rare colour must be from every palette colour and from the main
/// colour each stands for: a little more than the least difference one sees (about 2).
const RARE_APART: f64 = 2.5;

/// The fewest pixels of a 4-connected region that make a rare colour a colour of the picture (a
/// stem one pixel wide and eight long): scattered noise makes none so large.
const RARE_REGION: usize = 8;

/// A rare colour drawn as several small regions (highlights) is a colour of the picture too when
/// at least [`RARE_ISLANDS`] of its regions have this many pixels; noise kept by Noise removal is
/// no smaller than this either.
pub(crate) const RARE_ISLAND: usize = 4;

/// How many regions of [`RARE_ISLAND`] pixels make a rare colour coherent.
const RARE_ISLANDS: usize = 3;

/// Beside the cut edge, a color is the anti-aliasing between two others only when at least one in
/// this many of its pixels has pixels of both around it (see [`rare_colours`]).
const CUT_EDGE_MIX_TOUCH: usize = 32;

/// Beside the cut edge, a color is the anti-aliasing of a rare color only when it lies at least
/// this far from the other color toward the rare one, 0 to 1 (see [`rare_colours`]).
const CUT_EDGE_MIX_FROM: f64 = 0.45;

/// Automatic on flat art: the colours too rare to count among its colours (the count keeps colours
/// on at least about 1% of the picture), or too close to a large one to get a box of their own,
/// that are still colours of the picture: a highlight, a stem, a thin outline. A rare colour is
/// kept when it is distinct, at least [`RARE_APART`] in Oklab (Ottosson 2020) from every colour of
/// `centers` and from the main colour each stands for; flat, its pixels mostly of one histogram
/// bin; and drawn, some 4-connected region of its own (not blended) pixels of at least
/// [`RARE_REGION`] pixels, or [`RARE_ISLANDS`] regions of at least [`RARE_ISLAND`]. At most [`RARE_BUDGET`] are kept, those with the largest regions first.
///
/// `own` marks the pixels that count, and `choose` the pixels that choose the palette (see
/// [`choosing_pixels`]). A color with pixels beside a pixel that `choose` leaves out, and without an
/// inside, is not kept when it lies between two colors, one a palette color or another kept rare
/// color and the other a kept rare color, at least [`CUT_EDGE_MIX_FROM`] of the way to the rare
/// one, and at least one in [`CUT_EDGE_MIX_TOUCH`] of its pixels has pixels of both around it. The
/// pixels around it that `choose` leaves out count when their color is within [`RARE_APART`] of
/// one of the two. It is returned in the second list instead, with the indexes in `centers`
/// followed by the first list of the rare color it traces as and of the color on the other side
/// of it (the rare color again when it traces as the rare color alone).
///
/// The palette is otherwise median cut (Heckbert 1982) refined by k-means (Lloyd 1982); keeping
/// salient colours that are rare by area follows the idea of salient-colour preservation in
/// palette reduction (Fukuda et al. 2019, doi:10.1117/12.2521607). The rule and its numbers are
/// VectorCraft's own.
type ChosenRare = (usize, f64, [f64; 3], usize);

struct RareGroups {
    used: Vec<(usize, [f64; 3], f64)>,
    groups: Vec<Vec<usize>>,
    group_of: Vec<u16>,
}

struct RasterGroups<'a> {
    width: usize,
    height: usize,
    pixels: &'a [[u8; 4]],
    own: &'a [bool],
    choose: &'a [bool],
    hk: &'a HistKey,
    group_of: &'a [u16],
}

struct RegionMetrics {
    largest: Vec<usize>,
    islands: Vec<usize>,
    inner: Vec<usize>,
    beside_left_out: Vec<bool>,
}

struct BlendContext {
    from: Vec<usize>,
    with: Vec<[f64; 3]>,
    pairs: Vec<(usize, usize)>,
    near_labs: Vec<[f64; 3]>,
    counts_as: Vec<Option<usize>>,
}

pub(super) fn rare_colours(img: &Raster, own: &[bool], choose: &[bool], hk: &HistKey, centers: &[[f64; 3]]) -> (Vec<[f64; 3]>, Vec<TracedAs>) {
    let pixels = img.rgba.as_chunks::<4>().0;
    if centers.is_empty() || own.len() != pixels.len() {
        return (vec![], vec![]);
    }
    let center_labs = labs(centers);
    let groups = rare_groups(pixels, own, hk, &center_labs);
    if groups.groups.is_empty() {
        return (vec![], vec![]);
    }
    let (width, height) = (img.width as usize, img.height as usize);
    let raster = RasterGroups { width, height, pixels, own, choose, hk, group_of: &groups.group_of };
    let metrics = region_metrics(&raster, groups.groups.len());
    let chosen = choose_rare_groups(&groups.groups, &groups.used, centers, &metrics);
    if chosen.is_empty() {
        return (vec![], vec![]);
    }
    let (onto, other_of) = trace_edge_blends(&raster, &chosen, centers, &metrics);
    reindex_traced(chosen, onto, other_of, centers)
}

fn rare_groups(pixels: &[[u8; 4]], own: &[bool], hk: &HistKey, center_labs: &[[f64; 3]]) -> RareGroups {
    // The picture's own (not blended) pixels by histogram bin.
    let mut bins = vec![Bin::default(); 1 << 15];
    for (pixel, _) in pixels.iter().zip(own).filter(|(pixel, own)| **own && pixel[3] >= 128) {
        bins[hk.key(pixel[0], pixel[1], pixel[2])].add(pixel);
    }
    let used: Vec<(usize, [f64; 3], f64)> =
        bins.iter().enumerate().filter(|(_, bin)| bin.n > 0).map(|(i, bin)| (i, bin.mean(), bin.n as f64)).collect();
    let used_labs = labs(&used.iter().map(|entry| entry.1).collect::<Vec<_>>());
    // Each palette colour's main colour: its heaviest bin.
    let mut peak: Vec<Option<(f64, [f64; 3])>> = vec![None; center_labs.len()];
    for ((_, _, count), lab) in used.iter().zip(&used_labs) {
        let index = nearest_lab(center_labs, *lab);
        if let Some(slot) = peak.get_mut(index)
            && slot.is_none_or(|(largest, _)| *count > largest)
        {
            *slot = Some((*count, *lab));
        }
    }
    let distance = |a: [f64; 3], b: [f64; 3]| (0..3).map(|channel| (a[channel] - b[channel]).powi(2)).sum::<f64>().sqrt();
    let far_from_palette =
        |lab: [f64; 3]| center_labs.iter().chain(peak.iter().flatten().map(|(_, lab)| lab)).all(|center| distance(lab, *center) >= RARE_APART);
    // Group far bins by their heaviest member, in descending weight order.
    let mut far: Vec<usize> = (0..used.len()).filter(|&index| used_labs.get(index).is_some_and(|lab| far_from_palette(*lab))).collect();
    far.sort_by(|&a, &b| used[b].2.total_cmp(&used[a].2).then(a.cmp(&b)));
    let mut group_of = vec![u16::MAX; 1 << 15];
    let mut groups: Vec<Vec<usize>> = vec![];
    for index in far {
        let (Some(lab), Some(&(bin, ..))) = (used_labs.get(index), used.get(index)) else { continue };
        let group = groups
            .iter()
            .position(|members| members.first().and_then(|&first| used_labs.get(first)).is_some_and(|first| distance(*first, *lab) < RARE_APART));
        let group = match group {
            Some(group) => group,
            None if groups.len() < 64 => {
                groups.push(vec![]);
                groups.len() - 1
            }
            None => continue,
        };
        if let Some(members) = groups.get_mut(group) {
            members.push(index);
        }
        group_of[bin] = group as u16;
    }
    RareGroups { used, groups, group_of }
}

fn group_at(pixels: &[[u8; 4]], own: &[bool], hk: &HistKey, group_of: &[u16], index: usize) -> Option<u16> {
    let pixel = pixels.get(index)?;
    (own.get(index).copied().unwrap_or(false) && pixel[3] >= 128)
        .then(|| group_of.get(hk.key(pixel[0], pixel[1], pixel[2])).copied().unwrap_or(u16::MAX))
        .filter(|&group| group != u16::MAX)
}

fn region_metrics(raster: &RasterGroups<'_>, group_count: usize) -> RegionMetrics {
    let RasterGroups { width, height, pixels, own, choose, hk, group_of } = *raster;
    let mut metrics = RegionMetrics {
        largest: vec![0; group_count],
        islands: vec![0; group_count],
        inner: vec![0; group_count],
        beside_left_out: vec![false; group_count],
    };
    let left_out = |index: usize| pixels.get(index).is_some_and(|pixel| pixel[3] >= 128) && !choose.get(index).copied().unwrap_or(true);
    let mut seen = vec![false; width * height];
    let mut stack = vec![];
    for start in 0..width * height {
        let Some(group) = group_at(pixels, own, hk, group_of, start).filter(|_| !seen[start]) else { continue };
        seen[start] = true;
        stack.push(start);
        let mut size = 0;
        while let Some(index) = stack.pop() {
            size += 1;
            let (x, y) = (index % width, index / width);
            let mut alike = 0;
            for neighbour in [
                (x > 0).then(|| index - 1),
                (x + 1 < width).then(|| index + 1),
                (y > 0).then(|| index - width),
                (y + 1 < height).then(|| index + width),
            ]
            .into_iter()
            .flatten()
            {
                if group_at(pixels, own, hk, group_of, neighbour) == Some(group) {
                    alike += 1;
                    if !seen[neighbour] {
                        seen[neighbour] = true;
                        stack.push(neighbour);
                    }
                }
            }
            if alike == 4
                && let Some(count) = metrics.inner.get_mut(group as usize)
            {
                *count += 1;
            }
            if let Some(beside) = metrics.beside_left_out.get_mut(group as usize).filter(|beside| !**beside) {
                let (left, top, right, bottom) = (x.saturating_sub(1), y.saturating_sub(1), (x + 1).min(width - 1), (y + 1).min(height - 1));
                *beside = (top..=bottom).any(|near_y| (left..=right).any(|near_x| left_out(near_y * width + near_x)));
            }
        }
        if let Some(largest) = metrics.largest.get_mut(group as usize) {
            *largest = (*largest).max(size);
        }
        if let Some(islands) = metrics.islands.get_mut(group as usize).filter(|_| size >= RARE_ISLAND) {
            *islands += 1;
        }
    }
    metrics
}

fn choose_rare_groups(groups: &[Vec<usize>], used: &[(usize, [f64; 3], f64)], centers: &[[f64; 3]], metrics: &RegionMetrics) -> Vec<ChosenRare> {
    let mut chosen = Vec::new();
    for (group, members) in groups.iter().enumerate() {
        let weight: f64 = members.iter().filter_map(|&index| used.get(index)).map(|entry| entry.2).sum();
        let heaviest = members.first().and_then(|&index| used.get(index)).map_or(0.0, |entry| entry.2);
        let region = metrics.largest.get(group).copied().unwrap_or(0);
        let coherent = region >= RARE_REGION || metrics.islands.get(group).is_some_and(|&count| count >= RARE_ISLANDS);
        if !coherent || heaviest < 0.5 * weight || weight <= 0.0 {
            continue;
        }
        let mut mean = [0.0; 3];
        for entry in members.iter().filter_map(|&index| used.get(index)) {
            for (mean_channel, channel) in mean.iter_mut().zip(entry.1) {
                *mean_channel += channel * entry.2 / weight;
            }
        }
        // A color lying between two others needs a solid inside, not just a thin blended rim.
        if is_mix(mean, centers) && metrics.inner.get(group).is_none_or(|&count| count < RARE_ISLAND) {
            continue;
        }
        chosen.push((region, weight, mean, group));
    }
    chosen.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.total_cmp(&a.1)).then(a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal)));
    chosen
}

fn trace_edge_blends(
    raster: &RasterGroups<'_>,
    chosen: &[ChosenRare],
    centers: &[[f64; 3]],
    metrics: &RegionMetrics,
) -> (Vec<Option<[f64; 3]>>, Vec<Option<usize>>) {
    let RasterGroups { width, height, pixels, own, choose, hk, group_of } = *raster;
    let group_count = metrics.largest.len();
    let tested = |group: usize| {
        metrics.beside_left_out.get(group).copied().unwrap_or(false) && metrics.inner.get(group).is_none_or(|&count| count < RARE_ISLAND)
    };
    let wanted: Vec<bool> = (0..group_count).map(|group| tested(group) && chosen.iter().any(|candidate| candidate.3 == group)).collect();
    let mut pixels_of = vec![vec![]; group_count];
    if wanted.contains(&true) {
        for index in 0..width * height {
            if let Some(group) =
                group_at(pixels, own, hk, group_of, index).map(usize::from).filter(|&group| wanted.get(group).copied().unwrap_or(false))
                && let Some(pixels) = pixels_of.get_mut(group)
            {
                pixels.push(index);
            }
        }
    }
    let mut onto = vec![None; chosen.len()];
    let mut other_of = vec![None; chosen.len()];
    for index in (0..chosen.len()).rev() {
        let Some(&(_, _, color, group)) = chosen.get(index) else { continue };
        if !tested(group) {
            continue;
        }
        let Some(context) = blend_context(index, color, chosen, &onto, centers) else { continue };
        let Some(group_pixels) = pixels_of.get(group) else { continue };
        let (both, pixel_count) = blend_neighbour_counts(group_pixels, width, height, pixels, choose, &context);
        let best = best_blend_target(color, chosen, centers, metrics, &context, &both, pixel_count);
        if let Some(target) = onto.get_mut(index) {
            *target = best.map(|candidate| candidate.1);
        }
        if let Some(other) = other_of.get_mut(index) {
            *other = best.and_then(|candidate| candidate.2);
        }
    }
    (onto, other_of)
}

fn blend_context(index: usize, color: [f64; 3], chosen: &[ChosenRare], onto: &[Option<[f64; 3]>], centers: &[[f64; 3]]) -> Option<BlendContext> {
    let from: Vec<usize> = (0..centers.len())
        .chain(
            chosen
                .iter()
                .zip(onto)
                .enumerate()
                .filter(|(other, (_, target))| *other != index && target.is_none())
                .map(|(other, _)| centers.len() + other),
        )
        .collect();
    let with: Vec<[f64; 3]> = centers
        .iter()
        .copied()
        .chain(from.iter().skip(centers.len()).filter_map(|&source| chosen.get(source.checked_sub(centers.len())?)).map(|entry| entry.2))
        .collect();
    let pairs: Vec<(usize, usize)> = (0..with.len())
        .flat_map(|a| (a + 1..with.len()).map(move |b| (a, b)))
        .filter(|&(a, b)| matches!((with.get(a), with.get(b)), (Some(&left), Some(&right)) if is_mix(color, &[left, right])))
        .collect();
    if pairs.is_empty() {
        return None;
    }
    let mut near = with.clone();
    near.push(color);
    let mut counts_as: Vec<Option<usize>> = (0..with.len()).map(Some).chain([None]).collect();
    for (candidate, target) in chosen.iter().zip(onto) {
        if let Some(target) = target {
            near.push(candidate.2);
            counts_as.push(with.iter().position(|other| other == target));
        }
    }
    Some(BlendContext { from, with, pairs, near_labs: labs(&near), counts_as })
}

fn blend_neighbour_counts(
    group_pixels: &[usize],
    width: usize,
    height: usize,
    pixels: &[[u8; 4]],
    choose: &[bool],
    context: &BlendContext,
) -> (Vec<usize>, usize) {
    let distance = |a: [f64; 3], b: [f64; 3]| (0..3).map(|channel| (a[channel] - b[channel]).powi(2)).sum::<f64>().sqrt();
    let mut both = vec![0; context.pairs.len()];
    let mut pixel_count = 0;
    let mut around = vec![false; context.with.len()];
    for &index in group_pixels {
        let (x, y) = (index % width, index / width);
        pixel_count += 1;
        around.fill(false);
        for near_y in y.saturating_sub(1)..=(y + 1).min(height - 1) {
            for near_x in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                let neighbour = near_y * width + near_x;
                let Some(pixel) = pixels.get(neighbour).filter(|pixel| pixel[3] >= 128) else { continue };
                let lab = oklab([pixel[0], pixel[1], pixel[2]].map(f64::from));
                let nearest = nearest_lab(&context.near_labs, lab);
                if !choose.get(neighbour).copied().unwrap_or(false)
                    && context.near_labs.get(nearest).is_none_or(|candidate| distance(*candidate, lab) >= RARE_APART)
                {
                    continue;
                }
                if let Some(&Some(group)) = context.counts_as.get(nearest)
                    && let Some(is_near) = around.get_mut(group)
                {
                    *is_near = true;
                }
            }
        }
        for (pair_index, &(a, b)) in context.pairs.iter().enumerate() {
            if around.get(a).copied().unwrap_or(false)
                && around.get(b).copied().unwrap_or(false)
                && let Some(count) = both.get_mut(pair_index)
            {
                *count += 1;
            }
        }
    }
    (both, pixel_count)
}

fn best_blend_target(
    color: [f64; 3],
    chosen: &[ChosenRare],
    centers: &[[f64; 3]],
    metrics: &RegionMetrics,
    context: &BlendContext,
    both: &[usize],
    pixel_count: usize,
) -> Option<(f64, [f64; 3], Option<usize>)> {
    let distance = |a: [f64; 3], b: [f64; 3]| (0..3).map(|channel| (a[channel] - b[channel]).powi(2)).sum::<f64>().sqrt();
    let lab = oklab(color);
    let mut best = None;
    for (pair_index, &(a, b)) in context.pairs.iter().enumerate() {
        if both.get(pair_index).is_none_or(|&count| count == 0 || count * CUT_EDGE_MIX_TOUCH < pixel_count) {
            continue;
        }
        for (edge, other) in [(a, b), (b, a)].into_iter().filter(|&(edge, _)| edge >= centers.len()) {
            let (Some(&edge_color), Some(&other_color)) = (context.with.get(edge), context.with.get(other)) else { continue };
            let squared = (0..3).map(|channel| (edge_color[channel] - other_color[channel]).powi(2)).sum::<f64>();
            let along = if squared > 0.0 {
                (0..3).map(|channel| (color[channel] - other_color[channel]) * (edge_color[channel] - other_color[channel])).sum::<f64>() / squared
            } else {
                0.0
            };
            if along < CUT_EDGE_MIX_FROM {
                continue;
            }
            let separation = distance(oklab(edge_color), lab);
            if best.is_none_or(|candidate: (f64, [f64; 3], Option<usize>)| separation < candidate.0) {
                // A substantial rare region along the edge keeps the identity of its other side.
                let has_body = context
                    .from
                    .get(edge)
                    .and_then(|&source| chosen.get(source.checked_sub(centers.len())?))
                    .is_some_and(|rare| rare.0 >= RARE_REGION && metrics.beside_left_out.get(rare.3).copied().unwrap_or(false));
                best = Some((separation, edge_color, context.from.get(other).copied().filter(|_| has_body)));
            }
        }
    }
    best
}

fn reindex_traced(
    chosen: Vec<ChosenRare>,
    onto: Vec<Option<[f64; 3]>>,
    other_of: Vec<Option<usize>>,
    centers: &[[f64; 3]],
) -> (Vec<[f64; 3]>, Vec<TracedAs>) {
    let (output, traced) = kept_rare(chosen.iter().zip(&onto).zip(&other_of).map(|(((_, _, color, _), target), &other)| (*color, *target, other)));
    let output_index = |source: usize| -> Option<usize> {
        let Some(index) = source.checked_sub(centers.len()) else { return Some(source) };
        let (_, _, color, _) = chosen.get(index)?;
        let color = onto.get(index).copied().flatten().unwrap_or(*color);
        Some(centers.len() + output.iter().position(|candidate| *candidate == color)?)
    };
    let traced = traced
        .into_iter()
        .filter_map(|(color, target, other)| {
            let rare = centers.len() + output.iter().position(|candidate| *candidate == target)?;
            let other = match other {
                Some(source) => output_index(source)?,
                None => rare,
            };
            Some((color, rare, other))
        })
        .collect();
    (output, traced)
}

/// Of `colors` (a rare color, the rare color it traces as, and where the other color of its pair
/// comes from), largest region first: the rare colors kept, at most [`RARE_BUDGET`] and each at
/// least [`RARE_APART`] from the ones before it, and every color that traces as another, past the
/// budget too.
pub(crate) fn kept_rare(colors: impl IntoIterator<Item = ([f64; 3], Option<[f64; 3]>, Option<usize>)>) -> (Vec<[f64; 3]>, Vec<TracedAsColor>) {
    let dist = |a: [f64; 3], b: [f64; 3]| (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>().sqrt();
    let mut out: Vec<[f64; 3]> = vec![];
    let mut traced_as = vec![];
    for (c, o, from) in colors {
        if let Some(o) = o {
            traced_as.push((c, o, from));
            continue;
        }
        if out.len() >= RARE_BUDGET {
            continue;
        }
        let lab = oklab(c);
        if out.iter().all(|o| dist(oklab(*o), lab) >= RARE_APART) {
            out.push(c);
        }
    }
    (out, traced_as)
}

/// A color, the rare color it traces as, and where the other color of its pair comes from.
pub(crate) type TracedAsColor = ([f64; 3], [f64; 3], Option<usize>);

/// Whether `c` (sRGB) lies between two of `centers`, as anti-aliasing mixes them: within 12 levels
/// of the line between them, and at least an eighth of the way from each.
fn is_mix(c: [f64; 3], centers: &[[f64; 3]]) -> bool {
    centers.iter().enumerate().any(|(i, a)| {
        centers.iter().skip(i + 1).any(|b| {
            let d = [0, 1, 2].map(|k| b[k] - a[k]);
            let len2: f64 = d.iter().map(|v| v * v).sum();
            if len2 <= 0.0 {
                return false;
            }
            let t = (0..3).map(|k| (c[k] - a[k]) * d[k]).sum::<f64>() / len2;
            let off2: f64 = (0..3).map(|k| (c[k] - a[k] - t * d[k]).powi(2)).sum();
            (0.12..=0.88).contains(&t) && off2 < 144.0
        })
    })
}

/// `centers` refined over the bins `used` as k-means does, but the colours from `fixed` on (rare
/// colours, see [`rare_colours`]) stay where they are: they take their own bins from the colours
/// they were averaged into, and those move back to their own.
pub(super) fn refine_around(used: &[Used], mut centers: Vec<[f64; 3]>, fixed: usize) -> Vec<[f64; 3]> {
    let bins = labs(&used.iter().map(|u| u.1).collect::<Vec<_>>());
    for _ in 0..4 {
        let center_labs = labs(&centers);
        let mut sum = vec![[0.0; 3]; centers.len()];
        let mut cnt = vec![0.0; centers.len()];
        for ((_, c, n), lab) in used.iter().zip(&bins) {
            let j = nearest_lab(&center_labs, *lab);
            if let (Some(s), Some(k)) = (sum.get_mut(j), cnt.get_mut(j)) {
                for ch in 0..3 {
                    s[ch] += c[ch] * n;
                }
                *k += n;
            }
        }
        for (j, center) in centers.iter_mut().enumerate().take(fixed) {
            if let (Some(s), Some(&k)) = (sum.get(j), cnt.get(j))
                && k > 0.0
            {
                *center = [s[0] / k, s[1] / k, s[2] / k];
            }
        }
    }
    centers
}
