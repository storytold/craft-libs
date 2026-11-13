//! Whether a page uses overprint, read from its display list ([`DisplayList::overprint`]).

use super::{DisplayList, ImageSpace, Item, Paint};
use crate::ink::{InkColor, InkId, InkSet, Overprint};

/// Whether a page's paints use overprint, two ways (ISO 32000-2 §8.6.7, §11.7.4).
///
/// `takes_effect` implies `set`. DeviceCMYK at overprint mode 0 over process plates alone is
/// set but has no effect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PageOverprint {
    /// Some fill, stroke, glyph run, gradient or image is painted with overprint on.
    pub set: bool,
    /// At least one of those leaves a plate to the backdrop that a knockout would clear: a
    /// paint that doesn't name every plate on the page. A Separation or DeviceN color that
    /// names only some inks, a DeviceCMYK color under nonzero overprint mode with a zero
    /// component, or any process color on a page with a spot plate.
    pub takes_effect: bool,
}

impl DisplayList {
    /// Whether this page's paints use overprint, as [`plates`](crate::plates) separates them.
    /// Items inside a soft mask paint no ink and don't count. Nonzero overprint mode applies to
    /// solid process colors only; a gradient or image names every ink its color space has
    /// (§8.6.7). The `None` colorant paints nothing; a color listing no ink leaves every plate.
    pub fn overprint(&self) -> PageOverprint {
        let plates = Plates::new(&self.inks);
        let mut found = PageOverprint::default();
        // An explicit stack: group nesting comes from the file.
        let mut stack: Vec<&Item> = self.items.iter().rev().collect();
        while let Some(item) = stack.pop() {
            let (overprint, named) = match item {
                Item::Group(g) => {
                    stack.extend(g.children.iter().rev());
                    continue;
                }
                Item::Fill(f) => (f.overprint, paint_inks(&f.paint, f.overprint)),
                Item::Text(t) => (t.overprint, paint_inks(&t.paint, t.overprint)),
                Item::Image(im) => (im.overprint, image_inks(&im.space)),
            };
            if !(overprint.fill || overprint.stroke) {
                continue;
            }
            found.set = true;
            if plates.left_out(&named, &self.inks) {
                found.takes_effect = true;
                break;
            }
        }
        found
    }
}

/// The inks a paint names under overprint.
enum Named {
    All,
    Nothing,
    Inks(Vec<InkId>),
}

fn color_inks(color: &InkColor, overprint: Overprint, apply_mode: bool) -> Named {
    match color {
        InkColor::None => Named::Nothing,
        InkColor::All(_) => Named::All,
        InkColor::Tints(tints) => {
            let zero_left = apply_mode && overprint.nonzero && color.as_cmyk().is_some();
            Named::Inks(tints.iter().filter(|(_, t)| !zero_left || *t > 0.0).map(|(id, _)| *id).collect())
        }
    }
}

fn paint_inks(paint: &Paint, overprint: Overprint) -> Named {
    match paint {
        Paint::Solid(color) => color_inks(color, overprint, true),
        Paint::Gradient { stops, .. } => {
            let mut inks = Vec::new();
            for (_, color) in stops {
                match color_inks(color, overprint, false) {
                    Named::All => return Named::All,
                    Named::Nothing => {}
                    Named::Inks(i) => inks.extend(i),
                }
            }
            if inks.is_empty() { Named::Nothing } else { Named::Inks(inks) }
        }
    }
}

fn image_inks(space: &ImageSpace) -> Named {
    match space {
        // Gray and RGB samples print as process color on all four plates.
        ImageSpace::Gray | ImageSpace::Rgb | ImageSpace::Cmyk => Named::Inks(PROCESS.to_vec()),
        ImageSpace::Inks(ids) => Named::Inks(ids.clone()),
    }
}

const PROCESS: [InkId; 4] = [
    InkId::process(crate::ink::Process::Cyan),
    InkId::process(crate::ink::Process::Magenta),
    InkId::process(crate::ink::Process::Yellow),
    InkId::process(crate::ink::Process::Black),
];

/// The plates of a page: the process plates and each spot ink that prints on its own plate.
struct Plates {
    own: Vec<InkId>,
}

impl Plates {
    fn new(inks: &InkSet) -> Self {
        let spots = inks.spots().filter(|(id, _)| inks.plate_of(*id) == Some(*id)).map(|(id, _)| id);
        Self { own: PROCESS.into_iter().chain(spots).collect() }
    }

    /// Whether some plate is left to the backdrop by a paint naming `named`. A spot printed on
    /// another plate names that plate; one printed as process color names the process plates.
    fn left_out(&self, named: &Named, inks: &InkSet) -> bool {
        let ids = match named {
            Named::All | Named::Nothing => return false,
            Named::Inks(ids) => ids,
        };
        let mut hit = Vec::with_capacity(ids.len().saturating_add(PROCESS.len()));
        for &id in ids {
            match inks.plate_of(id) {
                Some(plate) => hit.push(plate),
                None if inks.get(id).is_some() => hit.extend(PROCESS),
                None => {}
            }
        }
        self.own.iter().any(|plate| !hit.contains(plate))
    }
}

#[cfg(test)]
mod tests {
    use kurbo::{Affine, Point, Rect, Shape};

    use super::*;
    use crate::flatten::{FillItem, FillRule, GradientGeometry, GroupItem, ImageItem, Mask, MaskKind};
    use crate::ink::{Alternate, Ink, Process, Spot};

    const C: InkId = InkId::process(Process::Cyan);
    const M: InkId = InkId::process(Process::Magenta);
    const Y: InkId = InkId::process(Process::Yellow);
    const K: InkId = InkId::process(Process::Black);
    const ON: Overprint = Overprint { fill: true, stroke: true, nonzero: false };
    const OPM1: Overprint = Overprint { fill: true, stroke: true, nonzero: true };

    fn fill(color: InkColor, overprint: Overprint) -> Item {
        let path = Rect::new(0.0, 0.0, 10.0, 10.0).to_path(0.1);
        Item::Fill(FillItem { path, rule: FillRule::NonZero, paint: Paint::Solid(color), overprint })
    }

    fn cmyk(c: f32, m: f32, y: f32, k: f32) -> InkColor {
        InkColor::Tints(vec![(C, c), (M, m), (Y, y), (K, k)])
    }

    fn list(inks: InkSet, items: Vec<Item>) -> DisplayList {
        DisplayList { page: Rect::new(0.0, 0.0, 100.0, 100.0), inks, items }
    }

    fn spot_set() -> (InkSet, InkId) {
        let mut inks = InkSet::new();
        let gold = Ink::Spot(Spot { name: "Gold".into(), alternate: Alternate::Cmyk([0.0, 0.3, 1.0, 0.0]), solidity: None });
        let id = inks.add(gold).unwrap_or(K);
        (inks, id)
    }

    fn group(children: Vec<Item>) -> GroupItem {
        GroupItem { opacity: 1.0, blend: Default::default(), isolated: false, knockout: false, clip: None, mask: None, actual_text: None, children }
    }

    fn answer(set: bool, takes_effect: bool) -> PageOverprint {
        PageOverprint { set, takes_effect }
    }

    #[test]
    fn overprint_is_set_and_takes_effect_only_where_it_leaves_a_plate() {
        let process = InkSet::new;
        let none = list(process(), vec![fill(cmyk(0.0, 0.0, 0.0, 1.0), Overprint::default())]);
        assert_eq!(none.overprint(), answer(false, false), "no overprint");

        let opm0 = list(process(), vec![fill(cmyk(0.0, 0.0, 0.0, 1.0), ON)]);
        assert_eq!(opm0.overprint(), answer(true, false), "DeviceCMYK at mode 0 names every process plate");

        let opm1 = list(process(), vec![fill(cmyk(0.0, 0.0, 0.0, 1.0), OPM1)]);
        assert_eq!(opm1.overprint(), answer(true, true), "mode 1 leaves the zero components");

        let opm1_full = list(process(), vec![fill(cmyk(0.1, 0.2, 0.3, 1.0), OPM1)]);
        assert_eq!(opm1_full.overprint(), answer(true, false), "no zero component, nothing left");

        let separation = list(process(), vec![fill(InkColor::Tints(vec![(K, 1.0)]), ON)]);
        assert_eq!(separation.overprint(), answer(true, true), "a Separation names one ink");

        let (inks, gold) = spot_set();
        let over_spot = list(inks.clone(), vec![fill(cmyk(0.0, 0.0, 0.0, 1.0), ON)]);
        assert_eq!(over_spot.overprint(), answer(true, true), "a process color leaves the spot plate");

        let all = list(inks.clone(), vec![fill(InkColor::All(1.0), ON), fill(InkColor::None, ON)]);
        assert_eq!(all.overprint(), answer(true, false), "All names every plate, None paints nothing");

        let nested = list(inks.clone(), vec![Item::Group(group(vec![Item::Group(group(vec![fill(InkColor::Tints(vec![(gold, 1.0)]), ON)]))]))]);
        assert_eq!(nested.overprint(), answer(true, true), "paints inside groups count");

        let mut masked = group(vec![]);
        masked.mask = Some(Box::new(Mask {
            kind: MaskKind::Alpha,
            group: group(vec![fill(InkColor::Tints(vec![(K, 1.0)]), ON)]),
            backdrop: None,
            transfer: None,
        }));
        assert_eq!(list(process(), vec![Item::Group(masked)]).overprint(), answer(false, false), "a soft mask paints no ink");
    }

    #[test]
    fn spot_plates_follow_the_ink_manager() {
        let (mut inks, gold) = spot_set();
        inks.set_to_process(gold, true);
        let k_only = vec![fill(cmyk(0.0, 0.0, 0.0, 1.0), ON)];
        assert_eq!(list(inks.clone(), k_only.clone()).overprint(), answer(true, false), "a spot printed as process has no plate");

        let gold_paint = vec![fill(InkColor::Tints(vec![(gold, 1.0)]), ON)];
        assert_eq!(list(inks, gold_paint.clone()).overprint(), answer(true, false), "it names the process plates");

        let (mut inks, gold) = spot_set();
        inks.alias(gold, K);
        assert_eq!(list(inks, gold_paint).overprint(), answer(true, true), "aliased to Black, it names one plate");

        let (mut inks, gold) = spot_set();
        let silver =
            inks.add(Ink::Spot(Spot { name: "Silver".into(), alternate: Alternate::Cmyk([0.0, 0.0, 0.0, 0.3]), solidity: None })).unwrap_or(K);
        inks.alias(gold, silver);
        let gold_and_cmyk = vec![fill(InkColor::Tints(vec![(C, 0.0), (M, 0.0), (Y, 0.0), (K, 1.0), (gold, 1.0)]), ON)];
        assert_eq!(list(inks, gold_and_cmyk).overprint(), answer(true, false), "aliased to Silver, it names the Silver plate");
    }

    #[test]
    fn gradients_name_the_inks_of_their_stops() {
        let gradient = |stops: Vec<InkColor>, overprint| {
            let stops = stops.into_iter().enumerate().map(|(i, c)| (i as f32, c)).collect();
            let geometry =
                GradientGeometry::Axial { start: Point::ZERO, end: Point::new(10.0, 0.0), extend: [false; 2], transform: Affine::IDENTITY };
            let path = Rect::new(0.0, 0.0, 10.0, 10.0).to_path(0.1);
            Item::Fill(FillItem { path, rule: FillRule::NonZero, paint: Paint::Gradient { stops, geometry }, overprint })
        };
        let page = |item| list(InkSet::new(), vec![item]).overprint();
        let black = || InkColor::Tints(vec![(K, 0.0)]);
        assert_eq!(page(gradient(vec![black(), InkColor::Tints(vec![(K, 1.0)])], ON)), answer(true, true), "a Separation gradient");
        assert_eq!(page(gradient(vec![InkColor::All(0.0), InkColor::All(1.0)], ON)), answer(true, false), "a registration gradient");
        assert_eq!(page(gradient(vec![InkColor::None, InkColor::None], ON)), answer(true, false), "a gradient of no ink");
        let cmyk_stops = vec![cmyk(0.0, 0.0, 0.0, 0.0), cmyk(0.0, 0.0, 0.0, 1.0)];
        assert_eq!(page(gradient(cmyk_stops, OPM1)), answer(true, false), "nonzero mode doesn't apply to a shading");
    }

    #[test]
    fn images_name_the_inks_of_their_space() {
        let image = |space: ImageSpace| {
            Item::Image(ImageItem {
                space,
                size: (1, 1),
                samples: vec![0; 4].into(),
                transform: Affine::IDENTITY,
                smask: None,
                overprint: ON,
                jpeg: None,
            })
        };
        assert_eq!(list(InkSet::new(), vec![image(ImageSpace::Rgb)]).overprint(), answer(true, false), "RGB prints on every process plate");
        assert_eq!(list(InkSet::new(), vec![image(ImageSpace::Gray)]).overprint(), answer(true, false), "so does gray");
        let (inks, _) = spot_set();
        assert_eq!(list(inks, vec![image(ImageSpace::Cmyk)]).overprint(), answer(true, true), "a CMYK image leaves the spot plate");
        assert_eq!(list(InkSet::new(), vec![image(ImageSpace::Inks(vec![K]))]).overprint(), answer(true, true), "a DeviceN image names its inks");
    }
}
