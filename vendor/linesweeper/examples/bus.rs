use kurbo::{Affine, BezPath};

use linesweeper::topology::{Topology, WindingNumber};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
struct Windings {
    main: i32,
    cutout: i32,
    modifier: i32,
}

impl std::ops::Add for Windings {
    type Output = Windings;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            main: self.main + rhs.main,
            cutout: self.cutout + rhs.cutout,
            modifier: self.modifier + rhs.modifier,
        }
    }
}

impl std::ops::AddAssign for Windings {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs
    }
}

impl WindingNumber for Windings {
    type Tag = Tag;

    fn single(tag: Self::Tag, positive: bool) -> Self {
        let sign = if positive { 1 } else { -1 };
        match tag {
            Tag::Main => Self {
                main: sign,
                ..Default::default()
            },
            Tag::Cutout => Self {
                cutout: sign,
                ..Default::default()
            },
            Tag::Modifier => Self {
                modifier: sign,
                ..Default::default()
            },
        }
    }

    fn of_tag(&self, tag: Self::Tag) -> Self {
        match tag {
            Tag::Main => Self {
                main: self.main,
                ..Default::default()
            },
            Tag::Cutout => Self {
                cutout: self.cutout,
                ..Default::default()
            },
            Tag::Modifier => Self {
                modifier: self.modifier,
                ..Default::default()
            },
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Tag {
    Main,
    Cutout,
    Modifier,
}

pub fn main() -> anyhow::Result<()> {
    let bus = "M0,80 C0,35.8 35.8,0 80,0 L432,0 C476.2,0 512,35.8 512,80 L512,368 C512,394.2 499.4,417.4 480,432 L480,480 C480,497.7 465.7,512 448,512 L416,512 C398.3,512 384,497.7 384,480 L384,448 L128,448 L128,480 C128,497.7 113.7,512 96,512 L64,512 C46.3,512 32,497.7 32,480 L32,432 C12.6,417.4 0,394.2 0,368 L0,80 Z M129.9,152.2 L112,224 L400,224 L382.1,152.2 C378.5,138 365.7,128 351,128 L161,128 C146.3,128 133.5,138 130,152.2 Z M128,320 A32 32 0 1 0 64,320 A32 32 0 1 0 128,320 Z M416,352 A32 32 0 1 0 416,288 A32 32 0 1 0 416,352 Z";
    let cutout = "M320 512V266.8C288.1 221.6 235.5 192 176 192C78.8 192 0 270.8 0 368c0 59.5 29.6 112.1 74.8 144H320z";
    let modifier = "M144 512c79.5 0 144-64.5 144-144s-64.5-144-144-144S0 288.5 0 368s64.5 144 144 144zm67.3-164.7l-72 72c-6.2 6.2-16.4 6.2-22.6 0l-40-40c-6.2-6.2-6.2-16.4 0-22.6s16.4-6.2 22.6 0L128 385.4l60.7-60.7c6.2-6.2 16.4-6.2 22.6 0s6.2 16.4 0 22.6z";

    let bus = BezPath::from_svg(bus).unwrap();
    let cutout = Affine::translate((193.0, 0.0)) * BezPath::from_svg(cutout).unwrap();
    let modifier = Affine::translate((224.0, 0.0)) * BezPath::from_svg(modifier).unwrap();

    let eps = 1e-5;
    let top = Topology::<Windings>::from_paths(
        [
            (&bus, Tag::Main),
            (&cutout, Tag::Cutout),
            (&modifier, Tag::Modifier),
        ],
        eps,
    )
    .unwrap();
    let bbox = top.bounding_box();
    let min_x = bbox.min_x();
    let min_y = bbox.min_y();
    let max_x = bbox.max_x();
    let max_y = bbox.max_y();
    let pad = 1.0 + eps;
    let one_width = max_x - min_x + 2.0 * pad;
    let one_height = max_y - min_y + 2.0 * pad;
    let mut doc = svg::Document::new().set(
        "viewBox",
        (min_x - pad, min_y - pad, one_width * 3.0, one_height * 2.0),
    );

    let contours = top.contours(|w| {
        let inside = |winding| winding != 0;

        (inside(w.main) && !inside(w.cutout)) || inside(w.modifier)
    });

    for group in contours.grouped() {
        let mut data = svg::node::element::path::Data::new();

        for contour_idx in group {
            let path = &contours[contour_idx].path;
            for el in path.iter() {
                data = match el {
                    kurbo::PathEl::MoveTo(p) => data.move_to((p.x, p.y)),
                    kurbo::PathEl::LineTo(p) => data.line_to((p.x, p.y)),
                    kurbo::PathEl::QuadTo(p0, p1) => {
                        data.quadratic_curve_to(((p0.x, p0.y), (p1.x, p1.y)))
                    }
                    kurbo::PathEl::CurveTo(p0, p1, p2) => {
                        data.cubic_curve_to(((p0.x, p0.y), (p1.x, p1.y), (p2.x, p2.y)))
                    }
                    kurbo::PathEl::ClosePath => data.close(),
                };
            }
        }
        let path = svg::node::element::Path::new()
            .set("d", data)
            .set("stroke", "black")
            .set("stroke-linecap", "round")
            .set("stroke-linejoin", "round");
        doc = doc.add(path);
    }

    svg::save("out.svg", &doc)?;

    Ok(())
}
