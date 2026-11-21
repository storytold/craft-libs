//! Gradients: areas whose colour changes evenly along one direction are found before the image is
//! reduced to its palette, and each is traced as one path filled with a linear gradient instead of
//! as bands of flat colour.
//!
//! An area is a connected run of pixels that each differ little from their neighbours. It is a
//! gradient when a plane fits each of its colour channels with all three sloping the same way, the
//! colours along that way are a few stops with even ramps between them, and painting those stops
//! back gives the area's own pixels. Anything else (a flat area, a photo's sky lit two ways, two
//! flats with a soft edge between them) is left to be traced as before.

use crate::Raster;
use crate::quantize::TRANSPARENT;
use kurbo::{Point, Vec2};

/// The largest step (levels of a channel) between neighbouring pixels of one smooth area.
const STEP: i32 = 8;
/// The fewest pixels a gradient has, and the least length (px) along its direction.
const MIN_PIXELS: usize = 400;
const MIN_LENGTH: f64 = 16.0;
/// The least change (levels of a channel) from one stop to another: less is a flat colour.
const MIN_CHANGE: f64 = 24.0;
/// The most stops, and the least distance between two (as a share of the length): closer stops are
/// an edge, not a ramp.
const MAX_STOPS: usize = 6;
const MIN_SPACING: f64 = 0.08;
/// How far (levels) the stops may be from the colours along the gradient.
const STOP_TOLERANCE: f64 = 2.0;
/// How far (levels) a pixel beside a gradient may be from the gradient's colour there and join it.
const GROW: f64 = 10.0;

/// A linear gradient a traced path is filled with.
#[derive(Clone, Debug, PartialEq)]
pub struct TracedGradient {
    /// The gradient's line, in pixel coordinates: the first stop's colour is at `start` and before
    /// it, the last one's at `end` and beyond.
    pub start: Point,
    pub end: Point,
    /// The stops: how far along the line (0 to 1, ascending, the first 0 and the last 1) and the colour.
    pub stops: Vec<(f64, [u8; 3])>,
}

impl TracedGradient {
    /// The gradient's colour at `p`.
    pub fn color_at(&self, p: Point) -> [f64; 3] {
        let d = self.end - self.start;
        let t = if d.hypot2() > 0.0 { ((p - self.start).dot(d) / d.hypot2()).clamp(0.0, 1.0) } else { 0.0 };
        let color = |i: usize| self.stops.get(i).map_or([0.0; 3], |s| s.1.map(f64::from));
        let after = self.stops.iter().position(|s| s.0 >= t).unwrap_or(self.stops.len().saturating_sub(1));
        let Some(before) = after.checked_sub(1) else { return color(after) };
        let (ta, tb) = (self.stops.get(before).map_or(0.0, |s| s.0), self.stops.get(after).map_or(1.0, |s| s.0));
        let k = if tb > ta { (t - ta) / (tb - ta) } else { 1.0 };
        let (a, b) = (color(before), color(after));
        [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * k)
    }
}

/// The pixels beside pixel `i` of a `w` × `h` image.
fn beside(i: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let (x, y) = (i % w, i / w);
    [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)].into_iter().flatten()
}

/// The gradients of `img`: for each pixel the gradient it is part of (an index into the list, or
/// `u32::MAX`), and the list.
pub(crate) fn detect(img: &Raster) -> (Vec<u32>, Vec<TracedGradient>) {
    let (w, h) = (img.width as usize, img.height as usize);
    let px = |i: usize| img.rgba.get(i * 4..i * 4 + 4).filter(|p| p[3] >= 128).map(|p| [i32::from(p[0]), i32::from(p[1]), i32::from(p[2])]);
    let near = |a: [i32; 3], b: Option<[i32; 3]>| b.is_some_and(|b| (0..3).all(|k| (a[k] - b[k]).abs() <= STEP));
    // Smooth: an opaque pixel that differs little from each neighbour it has.
    let smooth: Vec<bool> = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            px(i).is_some_and(|c| {
                (x == 0 || near(c, px(i - 1)))
                    && (x + 1 == w || near(c, px(i + 1)))
                    && (y == 0 || near(c, px(i - w)))
                    && (y + 1 == h || near(c, px(i + w)))
            })
        })
        .collect();
    // Sloping: smooth, and some pixel a little way off (a 32nd of the image: a gradient across a
    // large image changes by less than a level from one pixel to the next) is another colour.
    // Flat colour is no part of a gradient, and left out it does not join a gradient to the flat
    // area beside it where the two are nearly one colour.
    let far = (w.min(h) / 32).clamp(4, 64) as i64;
    let around = [
        (far, 0),
        (-far, 0),
        (0, far),
        (0, -far),
        (far * 3 / 4, far * 3 / 4),
        (far * 3 / 4, -far * 3 / 4),
        (-far * 3 / 4, far * 3 / 4),
        (-far * 3 / 4, -far * 3 / 4),
    ];
    let sloping: Vec<bool> = (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            let differs = |(dx, dy): (i64, i64)| {
                let (nx, ny) = (x + dx, y + dy);
                let other = (nx >= 0 && ny >= 0 && nx < w as i64 && ny < h as i64).then(|| px(ny as usize * w + nx as usize)).flatten();
                px(i).zip(other).is_some_and(|(a, b)| (0..3).any(|k| (a[k] - b[k]).abs() >= 2))
            };
            smooth.get(i).copied().unwrap_or(false) && around.into_iter().any(differs)
        })
        .collect();
    drop(smooth);
    let smooth = sloping;
    let mut of = vec![u32::MAX; w * h];
    let mut seen = vec![false; w * h];
    let mut depth = vec![0u16; w * h];
    let mut found = vec![];
    let (mut members, mut stack) = (vec![], vec![]);
    // The connected smooth pixels from `seed` that `seen` has not: into `members`.
    let mut flood = |seed: usize, seen: &mut [bool], members: &mut Vec<usize>| {
        members.clear();
        stack.push(seed);
        if let Some(s) = seen.get_mut(seed) {
            *s = true;
        }
        while let Some(i) = stack.pop() {
            members.push(i);
            for j in beside(i, w, h) {
                if smooth.get(j).copied().unwrap_or(false) && !seen.get(j).copied().unwrap_or(true) {
                    if let Some(s) = seen.get_mut(j) {
                        *s = true;
                    }
                    stack.push(j);
                }
            }
        }
    };
    // The areas, largest first: a gradient that noise or a soft rim broke into several is found in
    // its largest piece and grown over the rest.
    let mut areas: Vec<(usize, usize)> = vec![];
    for seed in 0..w * h {
        if !seen.get(seed).copied().unwrap_or(true) && smooth.get(seed).copied().unwrap_or(false) {
            flood(seed, &mut seen, &mut members);
            if members.len() >= MIN_PIXELS {
                areas.push((members.len(), seed));
            }
        }
    }
    areas.sort_unstable_by(|a, b| b.cmp(a));
    seen.fill(false);
    for (_, seed) in areas {
        if seen.get(seed).copied().unwrap_or(true) || found.len() >= 1000 {
            continue;
        }
        flood(seed, &mut seen, &mut members);
        if members.len() < MIN_PIXELS {
            continue;
        }
        // The gradient is read from the area's inside, where there is one: the pixels at its rim
        // may be a soft edge to the next area, joined to this one where the two are alike.
        // (`depth`: how many pixels in from the rim each of the area's pixels is, counted inwards
        // from the pixels that have a neighbour outside it, as far as matters.)
        let rim = far as usize + 2;
        let unknown = u16::MAX;
        members.iter().for_each(|&i| depth[i] = unknown);
        let mut front: Vec<usize> = members.iter().copied().filter(|&i| beside(i, w, h).any(|j| depth.get(j).copied() == Some(0))).collect();
        front.iter().for_each(|&i| depth[i] = 1);
        for d in 2..=rim as u16 {
            let mut next = vec![];
            for &i in &front {
                for j in beside(i, w, h) {
                    if let Some(slot) = depth.get_mut(j).filter(|s| **s == unknown) {
                        *slot = d;
                        next.push(j);
                    }
                }
            }
            front = next;
        }
        let inside: Vec<usize> = members.iter().copied().filter(|&i| depth.get(i).copied() == Some(unknown)).collect();
        let fitted = fit(img, w, if inside.len() >= MIN_PIXELS { &inside } else { &members }, &members, rim);
        members.iter().for_each(|&i| depth[i] = 0);
        if let Some((g, mut part)) = fitted {
            for &i in &part {
                if let Some(o) = of.get_mut(i) {
                    *o = found.len() as u32;
                }
            }
            // Grown over every pixel next to it that has its colour there: the pixels noise put a
            // little off, a soft rim, the pieces noise cut off it.
            while let Some(i) = part.pop() {
                for j in beside(i, w, h) {
                    let near = px(j).is_some_and(|c| {
                        let own = g.color_at(Point::new((j % w) as f64 + 0.5, (j / w) as f64 + 0.5));
                        (0..3).all(|k| (f64::from(c[k]) - own[k]).abs() <= GROW)
                    });
                    if near && of.get(j).copied() == Some(u32::MAX) {
                        of[j] = found.len() as u32;
                        seen[j] = true;
                        part.push(j);
                    }
                }
            }
            found.push(g);
        }
    }
    (of, found)
}

/// The linear gradient the pixels `members` of `img` (`w` wide) are, if they are one, and those of
/// `all` (the whole area `members` are the inside of, `rim` pixels in) that have its colours.
fn fit(img: &Raster, w: usize, members: &[usize], all: &[usize], rim: usize) -> Option<(TracedGradient, Vec<usize>)> {
    let n = members.len() as f64;
    let at = |i: usize| Point::new((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
    let rgb = |i: usize| img.rgba.get(i * 4..i * 4 + 3).map_or([0.0; 3], |p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]);
    let mut middle = Vec2::ZERO;
    let mut mean = [0.0; 3];
    for &i in members {
        middle += at(i).to_vec2();
        let c = rgb(i);
        (0..3).for_each(|k| mean[k] += c[k]);
    }
    let middle = (middle / n).to_point();
    let mean = mean.map(|v| v / n);
    // A plane through each channel, by least squares about the middle.
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    let (mut scx, mut scy) = ([0.0; 3], [0.0; 3]);
    for &i in members {
        let (d, c) = (at(i) - middle, rgb(i));
        sxx += d.x * d.x;
        sxy += d.x * d.y;
        syy += d.y * d.y;
        for k in 0..3 {
            scx[k] += d.x * (c[k] - mean[k]);
            scy[k] += d.y * (c[k] - mean[k]);
        }
    }
    let det = sxx * syy - sxy * sxy;
    if det <= 1e-6 * sxx * syy || !det.is_finite() {
        return None;
    }
    // Each channel's slope, and the one direction they share: the longer axis of the three
    // together (the shorter is what they do not share, and must be next to nothing).
    let slope = [0, 1, 2].map(|k| Vec2::new((scx[k] * syy - scy[k] * sxy) / det, (scy[k] * sxx - scx[k] * sxy) / det));
    let (mxx, mxy, myy) = slope.iter().fold((0.0, 0.0, 0.0), |m, s| (m.0 + s.x * s.x, m.1 + s.x * s.y, m.2 + s.y * s.y));
    let spread = ((mxx - myy) / 2.0).hypot(mxy);
    let (major, minor) = ((mxx + myy) / 2.0 + spread, (mxx + myy) / 2.0 - spread);
    if major <= 0.0 || minor > 0.03 * major {
        return None;
    }
    let angle = 0.5 * (2.0 * mxy).atan2(mxx - myy);
    let dir = Vec2::new(angle.cos(), angle.sin());
    // The colours along it, a pixel's width at a time.
    let along = |i: usize| (at(i) - middle).dot(dir);
    let (t0, t1) = members.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |m, &i| (m.0.min(along(i) - 0.5), m.1.max(along(i) + 0.5)));
    let length = t1 - t0;
    if !(MIN_LENGTH..1e6).contains(&length) {
        return None;
    }
    let bins = length.ceil() as usize;
    // (Each step's mean colour, at the mean place of its pixels: the few pixels at the tip of a
    // round area are not at the middle of their step.)
    let mut sums = vec![([0.0; 3], 0.0f64, 0.0f64); bins];
    for &i in members {
        let t = along(i) - t0;
        if let Some(b) = sums.get_mut((t as usize).min(bins - 1)) {
            let c = rgb(i);
            (0..3).for_each(|k| b.0[k] += c[k]);
            b.1 += 1.0;
            b.2 += t;
        }
    }
    let curve: Vec<(f64, [f64; 3])> = sums.iter().filter(|b| b.1 > 0.0).map(|b| (b.2 / b.1 / length, b.0.map(|v| v / b.1))).collect();
    // The fewest stops whose ramps pass within tolerance of every colour along the way: the two
    // ends, then again and again the colour farthest from the ramp it lies on.
    let mut keep = vec![0, curve.len().checked_sub(1)?];
    loop {
        let mut worst = (STOP_TOLERANCE, 0);
        for pair in keep.windows(2) {
            let (Some(&(ta, a)), Some(&(tb, b))) = (curve.get(pair[0]), curve.get(pair[1])) else { return None };
            for (j, &(t, c)) in curve.iter().enumerate().take(pair[1]).skip(pair[0] + 1) {
                let k = (t - ta) / (tb - ta);
                let off = (0..3).map(|ch| (c[ch] - (a[ch] + (b[ch] - a[ch]) * k)).abs()).fold(0.0, f64::max);
                if off > worst.0 {
                    worst = (off, j);
                }
            }
        }
        if worst.1 == 0 {
            break;
        }
        // (More than are let stand: those close together become one below.)
        if keep.len() >= 4 * MAX_STOPS {
            return None;
        }
        keep.push(worst.1);
        keep.sort_unstable();
    }
    // Two stops close together are one stop where the ramp turns (a turn is not sharp to the
    // pixel, least of all in a blurred image), unless their colours are far apart: that is an
    // edge, not a ramp.
    let mut stops: Vec<(f64, [f64; 3])> = vec![];
    for &(t, c) in keep.iter().filter_map(|&j| curve.get(j)) {
        match stops.last_mut() {
            Some(last) if t - last.0 < MIN_SPACING => {
                if (0..3).any(|k| (c[k] - last.1[k]).abs() >= MIN_CHANGE) {
                    return None;
                }
                *last = ((last.0 + t) / 2.0, [0, 1, 2].map(|k| (last.1[k] + c[k]) / 2.0));
            }
            _ => stops.push((t, c)),
        }
    }
    let (first, last) = (stops.first()?.0, stops.last()?.0);
    // A change of colour, not one colour.
    let change =
        stops.iter().flat_map(|a| stops.iter().map(move |b| (0..3).map(|k| (a.1[k] - b.1[k]).abs()).fold(0.0, f64::max))).fold(0.0, f64::max);
    // And ramps most of the way: two flat colours with a soft edge between them are no gradient.
    let level: f64 = stops.windows(2).filter(|s| (0..3).all(|k| (s[1].1[k] - s[0].1[k]).abs() < 4.0)).map(|s| s[1].0 - s[0].0).sum();
    if !(2..=MAX_STOPS).contains(&stops.len()) || last - first < MIN_SPACING || change < MIN_CHANGE || level > 0.3 * (last - first) {
        return None;
    }
    let gradient = TracedGradient {
        start: middle + dir * (t0 + first * length),
        end: middle + dir * (t0 + last * length),
        stops: stops.iter().map(|&(t, c)| ((t - first) / (last - first), c.map(|v| v.round().clamp(0.0, 255.0) as u8))).collect(),
    };
    // Carried on to the ends of the whole area, each end ramp going on as it goes.
    let d = gradient.end - gradient.start;
    let place = |i: usize| (at(i) - gradient.start).dot(d) / d.hypot2();
    let reach = rim as f64 * 1.5 / d.hypot();
    let (u0, u1) = all.iter().fold((0.0f64, 1.0f64), |m, &i| (m.0.min(place(i)), m.1.max(place(i))));
    let (u0, u1) = (u0.max(-reach), u1.min(1.0 + reach));
    let ramp = |a: (f64, [u8; 3]), b: (f64, [u8; 3]), u: f64| {
        [0, 1, 2].map(|k| (f64::from(a.1[k]) + (f64::from(b.1[k]) - f64::from(a.1[k])) * (u - a.0) / (b.0 - a.0)).round().clamp(0.0, 255.0) as u8)
    };
    let mut stops = gradient.stops.clone();
    if let (&[a, b, ..], &[.., y, z]) = (stops.as_slice(), stops.as_slice()) {
        if let Some(first) = stops.first_mut() {
            *first = (u0, ramp(a, b, u0));
        }
        if let Some(last) = stops.last_mut() {
            *last = (u1, ramp(y, z, u1));
        }
    }
    let gradient = TracedGradient {
        start: gradient.start + d * u0,
        end: gradient.start + d * u1,
        stops: stops.into_iter().map(|(u, c)| ((u - u0) / (u1 - u0), c)).collect(),
    };
    // Painted back, it is the area: most pixels of its inside within a few levels, and all that
    // are (the rest: noise, a soft rim, the flat colour just beyond it) on average nearer still.
    let off = |i: usize| {
        let (c, g) = (rgb(i), gradient.color_at(at(i)));
        (0..3).map(|k| (c[k] - g[k]).abs()).fold(0.0, f64::max)
    };
    let part: Vec<usize> = all.iter().copied().filter(|&i| off(i) <= 6.0).collect();
    let mean = part.iter().map(|&i| off(i)).sum::<f64>() / part.len().max(1) as f64;
    let inside = members.iter().filter(|&&i| off(i) <= 6.0).count();
    (part.len() >= MIN_PIXELS && inside * 4 >= members.len() * 3 && mean <= 3.5).then_some((gradient, part))
}

/// Settles the pixels along the edge of each gradient (they are part gradient and part what lies
/// beyond, and no part of the smooth area the gradient was found in): each joins the gradient
/// (`of`) when more than half of it is the gradient's colour there, and else takes the label of
/// what lies beyond. `labels` are the image's palette labels, the gradients' pixels not among them.
pub(crate) fn settle_edges(img: &Raster, of: &mut [u32], gradients: &[TracedGradient], labels: &mut [u16]) {
    let (w, h) = (img.width as usize, img.height as usize);
    let rgb = |i: usize| img.rgba.get(i * 4..i * 4 + 4).filter(|p| p[3] >= 128).map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]);
    // Twice: a soft edge is two pixels wide.
    for _ in 0..2 {
        let mut joins: Vec<(usize, u32)> = vec![];
        let mut takes: Vec<(usize, u16)> = vec![];
        for i in 0..w * h {
            if of.get(i).copied() != Some(u32::MAX) {
                continue;
            }
            let Some(c) = rgb(i) else { continue };
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            let index = |x: i64, y: i64| (x >= 0 && y >= 0 && x < w as i64 && y < h as i64).then(|| y as usize * w + x as usize);
            // The gradient on one side, and what lies two pixels out on the other.
            let Some((g, dx, dy)) = [(1, 0), (-1, 0), (0, 1), (0, -1)].into_iter().find_map(|(dx, dy)| {
                let g = index(x + dx, y + dy).and_then(|j| of.get(j).copied()).filter(|&g| g != u32::MAX)?;
                Some((g, dx, dy))
            }) else {
                continue;
            };
            let Some(gradient) = gradients.get(g as usize) else { continue };
            let own = gradient.color_at(Point::new(x as f64 + 0.5, y as f64 + 0.5));
            let off = (0..3).map(|k| (c[k] - own[k]).abs()).fold(0.0, f64::max);
            let beyond = index(x - 2 * dx, y - 2 * dy).filter(|&j| of.get(j).copied() == Some(u32::MAX));
            let other = beyond.and_then(|j| Some((rgb(j)?, labels.get(j).copied()?)));
            match other {
                Some((o, label)) if (0..3).map(|k| (o[k] - own[k]).abs()).fold(0.0, f64::max) > 16.0 => {
                    // How much of the pixel is the other colour.
                    let d = [0, 1, 2].map(|k| o[k] - own[k]);
                    let share = (0..3).map(|k| (c[k] - own[k]) * d[k]).sum::<f64>() / d.iter().map(|v| v * v).sum::<f64>();
                    if share < 0.5 {
                        joins.push((i, g));
                    } else if label != TRANSPARENT {
                        takes.push((i, label));
                    }
                }
                // Gradient beyond too, or nothing to tell it from: part of it if it has its colour.
                _ if off <= 16.0 => joins.push((i, g)),
                _ => {}
            }
        }
        for (i, g) in joins {
            if let Some(o) = of.get_mut(i) {
                *o = g;
            }
        }
        for (i, l) in takes {
            if let Some(slot) = labels.get_mut(i) {
                *slot = l;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(w: u32, h: u32, f: impl Fn(f64, f64) -> [f64; 3]) -> Raster {
        Raster::from_fn(w, h, |x, y| {
            let c = f(f64::from(x) + 0.5, f64::from(y) + 0.5);
            [c[0].round() as u8, c[1].round() as u8, c[2].round() as u8, 255]
        })
    }

    #[test]
    fn a_linear_ramp_is_one_gradient_with_its_direction_and_ends() {
        // Dark blue to yellow, 30° down from level.
        let dir = Vec2::new(30f64.to_radians().cos(), 30f64.to_radians().sin());
        let reach = Vec2::new(120.0, 80.0).dot(dir);
        let img = ramp(120, 80, |x, y| {
            let t = Vec2::new(x, y).dot(dir) / reach;
            [20.0 + 220.0 * t, 40.0 + 180.0 * t, 160.0 - 140.0 * t]
        });
        let (of, found) = detect(&img);
        assert_eq!(found.len(), 1);
        assert!(of.iter().all(|&g| g == 0));
        let g = &found[0];
        let d = g.end - g.start;
        assert!((d.atan2().to_degrees() - 30.0).abs() < 1.0, "{d:?}");
        assert_eq!(g.stops.len(), 2);
        for (stop, truth) in g.stops.iter().zip([[20.0, 40.0, 160.0], [240.0, 220.0, 20.0]]) {
            assert!((0..3).all(|k| (f64::from(stop.1[k]) - truth[k]).abs() < 6.0), "{:?}", g.stops);
        }
    }

    #[test]
    fn flat_colour_soft_edges_and_two_way_shading_are_no_gradient() {
        // One colour.
        assert!(detect(&ramp(64, 64, |_, _| [200.0, 30.0, 30.0])).1.is_empty());
        // Two flat colours with a soft edge between them.
        let soft = ramp(96, 64, |x, _| [100.0 + 30.0 / (1.0 + (-(x - 48.0)).exp()); 3]);
        assert!(detect(&soft).1.is_empty());
        // Red changing across and green changing down.
        assert!(detect(&ramp(96, 96, |x, y| [40.0 + 2.0 * x, 40.0 + 2.0 * y, 90.0])).1.is_empty());
    }

    #[test]
    fn three_stops_are_three_stops() {
        let img = ramp(200, 40, |x, _| {
            let t = x / 200.0;
            if t < 0.5 { [255.0 - 400.0 * t, 40.0, 40.0 + 300.0 * t] } else { [55.0 + 300.0 * (t - 0.5), 40.0 + 400.0 * (t - 0.5), 190.0] }
        });
        let (_, found) = detect(&img);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].stops.len(), 3, "{:?}", found[0].stops);
        assert!((found[0].stops[1].0 - 0.5).abs() < 0.03);
    }
}
