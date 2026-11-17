//! OpenEXR channel and layer names.
//!
//! A channel name is `layer.channel`, where the layer is everything before the last dot and may
//! itself be nested (`ViewLayer.Combined.R`); a channel without a dot is in the unnamed layer.

use std::collections::BTreeMap;

/// A channel name's layer and channel: `diffuse.R` → (`diffuse`, `R`), `ViewLayer.Combined.R`
/// → (`ViewLayer.Combined`, `R`), `Z` → (`""`, `Z`).
pub fn split_channel(name: &str) -> (&str, &str) {
    name.rsplit_once('.').unwrap_or(("", name))
}

/// The channel name without its layer (`diffuse.R` → `R`).
pub fn base_channel_name(name: &str) -> &str {
    split_channel(name).1
}

/// The full name of a channel of a named part: `part.channel`, except that a channel which
/// already starts with the part's name is kept as is (Blender 5 writes one part per pass whose
/// channels carry the full name: part `ViewLayer.Depth` holds `ViewLayer.Depth.Z`).
pub fn part_channel_name(part: Option<&str>, channel: &str) -> String {
    match part {
        Some(p) if channel.strip_prefix(p).is_some_and(|rest| rest.starts_with('.')) => channel.to_string(),
        Some(p) => format!("{p}.{channel}"),
        None => channel.to_string(),
    }
}

/// Whether a channel holds depth: its channel is `Z`, or its name mentions depth.
pub fn is_depth(name: &str) -> bool {
    base_channel_name(name).eq_ignore_ascii_case("z") || name.to_ascii_lowercase().contains("depth")
}

/// The layers of `names` (`""` for the unnamed layer), each once, in channel order.
pub fn layers<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for name in names {
        let layer = split_channel(name).0;
        if !out.contains(&layer) {
            out.push(layer);
        }
    }
    out
}

/// The channels of `layer` to show as red, green, blue and alpha, as indices into `names`: its R,
/// G, B, A channels (or red / green / blue / alpha, X / Y / Z, U / V), in any case. A layer of one
/// channel (depth, an ID) shows it in all three colours.
pub fn layer_rgba<'a>(names: impl IntoIterator<Item = &'a str>, layer: &str) -> [Option<usize>; 4] {
    let chans: Vec<(usize, &str)> =
        names.into_iter().enumerate().filter(|(_, n)| split_channel(n).0 == layer).map(|(i, n)| (i, split_channel(n).1)).collect();
    if let [(one, _)] = chans.as_slice() {
        return [Some(*one), Some(*one), Some(*one), None];
    }
    let pick = |wanted: &[&str]| chans.iter().find(|(_, c)| wanted.iter().any(|w| c.eq_ignore_ascii_case(w))).map(|(i, _)| *i);
    [pick(&["R", "red", "X", "U"]), pick(&["G", "green", "Y", "V"]), pick(&["B", "blue", "Z"]), pick(&["A", "alpha"])]
}

/// What the colour channels of a picture are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorLayout {
    /// Luminance: `Y`, or the only channel of a one-channel file.
    Y,
    /// `Y`, `A`.
    YA,
    /// `R`, `G`, `B`.
    Rgb,
    /// `R`, `G`, `B`, `A`.
    Rgba,
}

impl ColorLayout {
    /// How many channels the layout has, which is also its rank when choosing between groups.
    pub fn channels(self) -> usize {
        match self {
            ColorLayout::Y => 1,
            ColorLayout::YA => 2,
            ColorLayout::Rgb => 3,
            ColorLayout::Rgba => 4,
        }
    }
}

/// The channels of `names` that make a colour picture, as a layout and the indices of its
/// channels in layout order. `None` when there is none (for example `Z` beside `A`).
///
/// One policy for ranking parts, probing and decoding: RGB(A) or YA is never put together from
/// separate layers. The colour and alpha channels must share a whole layer prefix; a leading or
/// trailing dot alone does not name a layer. The group with the most channels wins; equal groups
/// prefer the unnamed layer, then the first prefix alphabetically. A file of exactly one channel,
/// whatever its name, is shown as luminance.
pub fn color_channels<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<(ColorLayout, Vec<usize>)> {
    #[derive(Default)]
    struct Group {
        r: Option<usize>,
        g: Option<usize>,
        b: Option<usize>,
        a: Option<usize>,
        y: Option<usize>,
    }

    // Sorted prefixes make rank ties deterministic independently of channel order, with the
    // unnamed group first. Other channels cannot complete a group.
    let mut groups = BTreeMap::<&str, Group>::new();
    let mut count = 0usize;
    for (index, name) in names.into_iter().enumerate() {
        count = count.saturating_add(1);
        let (prefix, base) = name.rsplit_once('.').filter(|(prefix, base)| !prefix.is_empty() && !base.is_empty()).unwrap_or(("", name));
        if !matches!(base, "R" | "G" | "B" | "A" | "Y") {
            continue;
        }
        let group = groups.entry(prefix).or_default();
        let component = match base {
            "R" => &mut group.r,
            "G" => &mut group.g,
            "B" => &mut group.b,
            "A" => &mut group.a,
            _ => &mut group.y,
        };
        component.get_or_insert(index);
    }

    let mut best = None;
    let mut best_rank = 0;
    for group in groups.values() {
        let candidate = if let (Some(r), Some(g), Some(b)) = (group.r, group.g, group.b) {
            match group.a {
                Some(a) => (ColorLayout::Rgba, vec![r, g, b, a]),
                None => (ColorLayout::Rgb, vec![r, g, b]),
            }
        } else if let Some(y) = group.y {
            match group.a {
                Some(a) => (ColorLayout::YA, vec![y, a]),
                None => (ColorLayout::Y, vec![y]),
            }
        } else {
            continue;
        };
        let rank = candidate.0.channels();
        if rank > best_rank {
            best = Some(candidate);
            best_rank = rank;
        }
    }
    best.or_else(|| (count == 1).then(|| (ColorLayout::Y, vec![0])))
}

/// The main picture of a file by the OpenEXR convention, the one compositing applications show
/// when a file is imported: the channels without a layer prefix, `R`, `G`, `B` (and `A` when
/// present), of the first part that has all three; else the first part's unprefixed `Y` (and
/// `A`). Named layers never stand in for it: a file of named layers only (Blender's
/// `ViewLayer.Combined.R`, a part that stores `beauty.R`) has no main picture, and its layers
/// are reached by name, as in After Effects (EXtractoR) and Nuke (the `rgba` layer stays empty).
///
/// `parts` gives each part's channel names as stored, in file order. Returns the part's index
/// and the channels' indices in that part, in layout order (R, G, B, A or Y, A). The names are
/// matched exactly, as the specification spells them; the first of duplicate names counts.
pub fn main_picture<'a, P, C>(parts: P) -> Option<(usize, ColorLayout, Vec<usize>)>
where
    P: IntoIterator<Item = C>,
    C: IntoIterator<Item = &'a str>,
{
    let mut luminance: Option<(usize, ColorLayout, Vec<usize>)> = None;
    for (part, names) in parts.into_iter().enumerate() {
        let (mut r, mut g, mut b, mut a, mut y) = (None, None, None, None, None);
        for (index, name) in names.into_iter().enumerate() {
            let slot = match name {
                "R" => &mut r,
                "G" => &mut g,
                "B" => &mut b,
                "A" => &mut a,
                "Y" => &mut y,
                _ => continue,
            };
            slot.get_or_insert(index);
        }
        if let (Some(r), Some(g), Some(b)) = (r, g, b) {
            return Some(match a {
                Some(a) => (part, ColorLayout::Rgba, vec![r, g, b, a]),
                None => (part, ColorLayout::Rgb, vec![r, g, b]),
            });
        }
        if luminance.is_none()
            && let Some(y) = y
        {
            luminance = Some(match a {
                Some(a) => (part, ColorLayout::YA, vec![y, a]),
                None => (part, ColorLayout::Y, vec![y]),
            });
        }
    }
    luminance
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn names_split_at_the_last_dot() {
        assert_eq!(split_channel("diffuse.R"), ("diffuse", "R"));
        assert_eq!(split_channel("ViewLayer.Combined.R"), ("ViewLayer.Combined", "R"));
        assert_eq!(split_channel("Z"), ("", "Z"));
        assert_eq!((base_channel_name("diffuse.R"), base_channel_name("R")), ("R", "R"));
    }

    #[test]
    fn part_names_prefix_channels_once() {
        assert_eq!(part_channel_name(None, "R"), "R");
        assert_eq!(part_channel_name(Some("diffuse"), "R"), "diffuse.R");
        assert_eq!(part_channel_name(Some("ViewLayer.Depth"), "ViewLayer.Depth.Z"), "ViewLayer.Depth.Z");
        // A shared prefix that is not a whole layer is not the part's name.
        assert_eq!(part_channel_name(Some("View"), "ViewLayer.Z"), "View.ViewLayer.Z");
    }

    #[test]
    fn depth_channels() {
        for name in ["Z", "z", "ViewLayer.Depth.Z", "depth.V", "CameraDepth"] {
            assert!(is_depth(name), "{name}");
        }
        for name in ["R", "normal.Y", "Zebra"] {
            assert!(!is_depth(name), "{name}");
        }
    }

    #[test]
    fn layers_keep_channel_order() {
        let names = ["R", "G", "diffuse.R", "Z", "a.b.X", "diffuse.G"];
        assert_eq!(layers(names), ["", "diffuse", "a.b"]);
        assert!(layers([]).is_empty());
    }

    #[test]
    fn a_layer_shows_its_colour_channels_or_its_only_channel() {
        let names = [
            "A",
            "B",
            "G",
            "R",
            "normal.X",
            "normal.Y",
            "normal.Z",
            "depth.Z",
            "uv.U",
            "uv.V",
            "crypto.red",
            "crypto.GREEN",
            "crypto.blue",
            "crypto.alpha",
        ];
        assert_eq!(layer_rgba(names, ""), [Some(3), Some(2), Some(1), Some(0)]);
        assert_eq!(layer_rgba(names, "normal"), [Some(4), Some(5), Some(6), None]);
        assert_eq!(layer_rgba(names, "depth"), [Some(7), Some(7), Some(7), None]);
        assert_eq!(layer_rgba(names, "uv"), [Some(8), Some(9), None, None]);
        assert_eq!(layer_rgba(names, "crypto"), [Some(10), Some(11), Some(12), Some(13)]);
        assert_eq!(layer_rgba(names, "missing"), [None; 4]);
    }

    #[test]
    fn colour_channels_come_from_one_layer() {
        let pick = |names: &[&str]| color_channels(names.iter().copied());
        assert_eq!(pick(&["A", "B", "G", "R"]), Some((ColorLayout::Rgba, vec![3, 2, 1, 0])));
        assert_eq!(pick(&["B", "G", "R", "Z"]), Some((ColorLayout::Rgb, vec![2, 1, 0])));
        assert_eq!(pick(&["A", "Y"]), Some((ColorLayout::YA, vec![1, 0])));
        assert_eq!(pick(&["Y", "Z"]), Some((ColorLayout::Y, vec![0])));
        // Never RGB from one layer and alpha from another.
        assert_eq!(pick(&["A", "diffuse.B", "diffuse.G", "diffuse.R"]), Some((ColorLayout::Rgb, vec![3, 2, 1])));
        assert_eq!(pick(&["a.R", "b.G", "c.B"]), None);
        assert_eq!(pick(&["A", "Z"]), None);
        // The group with the most channels wins; ties go to the unnamed layer, then alphabetically.
        assert_eq!(pick(&["R", "G", "B", "beauty.R", "beauty.G", "beauty.B", "beauty.A"]), Some((ColorLayout::Rgba, vec![3, 4, 5, 6])));
        assert_eq!(pick(&["z.R", "z.G", "z.B", "R", "G", "B"]), Some((ColorLayout::Rgb, vec![3, 4, 5])));
        assert_eq!(pick(&["z.R", "z.G", "z.B", "m.R", "m.G", "m.B"]), Some((ColorLayout::Rgb, vec![3, 4, 5])));
        // Nested layers use the whole prefix; a leading or trailing dot names no layer.
        assert_eq!(pick(&["a.b.R", "a.b.G", "a.b.B", "a.R"]), Some((ColorLayout::Rgb, vec![0, 1, 2])));
        assert_eq!(pick(&[".R", ".G", ".B"]), None);
        // The first of duplicate names is used, and a lone channel of any name is luminance.
        assert_eq!(pick(&["Y", "Y"]), Some((ColorLayout::Y, vec![0])));
        assert_eq!(pick(&["depth.Z"]), Some((ColorLayout::Y, vec![0])));
        assert_eq!(pick(&[]), None);
        assert_eq!(pick(&["r", "g", "b"]), None);
    }

    #[test]
    fn the_main_picture_is_the_unprefixed_colour_of_the_first_part_that_has_it() {
        let main = |parts: &[&[&str]]| main_picture(parts.iter().map(|p| p.iter().copied()));
        // One part: the bare channels, whatever else is there, also before them alphabetically.
        assert_eq!(
            main(&[&["A", "B", "CryptoMaterial00.A", "CryptoMaterial00.B", "CryptoMaterial00.G", "CryptoMaterial00.R", "G", "R", "Z"]]),
            Some((0, ColorLayout::Rgba, vec![7, 6, 1, 0]))
        );
        assert_eq!(main(&[&["B", "G", "R", "diffuse.A", "diffuse.B", "diffuse.G", "diffuse.R"]]), Some((0, ColorLayout::Rgb, vec![2, 1, 0])));
        // Named layers only (Blender multilayer): no main picture, however good a layer looks.
        assert_eq!(
            main(&[&[
                "ViewLayer.AO.B",
                "ViewLayer.AO.G",
                "ViewLayer.AO.R",
                "ViewLayer.Combined.A",
                "ViewLayer.Combined.B",
                "ViewLayer.Combined.G",
                "ViewLayer.Combined.R"
            ]]),
            None
        );
        assert_eq!(
            main(&[&["ViewLayer.Combined.A", "ViewLayer.Combined.B", "ViewLayer.Combined.G", "ViewLayer.Combined.R"], &["ViewLayer.Depth.Z"]]),
            None
        );
        // Multi-part: the first part with bare R, G and B, not the first part.
        assert_eq!(main(&[&["Z"], &["A", "B", "G", "R"], &["B", "G", "R"]]), Some((1, ColorLayout::Rgba, vec![3, 2, 1, 0])));
        assert_eq!(main(&[&["B", "G", "R"], &["A", "B", "G", "R"]]), Some((0, ColorLayout::Rgb, vec![2, 1, 0])));
        // Colour wins over an earlier luminance part; luminance when there is no colour.
        assert_eq!(main(&[&["Y"], &["B", "G", "R"]]), Some((1, ColorLayout::Rgb, vec![2, 1, 0])));
        assert_eq!(main(&[&["Z"], &["A", "Y"], &["Y"]]), Some((1, ColorLayout::YA, vec![1, 0])));
        // Incomplete colour, other spellings and lone data channels are no picture.
        assert_eq!(main(&[&["G", "R"]]), None);
        assert_eq!(main(&[&["b", "g", "r"]]), None);
        assert_eq!(main(&[&["Z"]]), None);
        assert_eq!(main(&[&["A"]]), None);
        assert_eq!(main(&[]), None);
        // The first of duplicate names counts.
        assert_eq!(main(&[&["R", "G", "B", "R"]]), Some((0, ColorLayout::Rgb, vec![0, 1, 2])));
    }
}
