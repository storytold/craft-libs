use color::ColorSpace as _;
use kurbo::{Affine, BezPath, CubicBez, PathSeg, Point};
use linesweeper::curve::{Order, intersect_cubics, y_subsegment};

fn flip(c: CubicBez) -> CubicBez {
    Affine::reflect(Point::ZERO, (0.0, 1.0)) * c
}

fn shift(c: CubicBez, dx: f64) -> CubicBez {
    Affine::translate((0.0, dx)) * c
}

fn horiz_at_midpoint() -> CubicBez {
    CubicBez::new((-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0))
}

fn c() -> CubicBez {
    CubicBez::new((1.0, -1.0), (-1.0, -1.0), (-1.0, 1.0), (1.0, 1.0))
}

fn diagonal() -> CubicBez {
    CubicBez::new(
        (-1.0, -1.0),
        (-1.0 / 3.0, -1.0 / 3.0),
        (1.0 / 3.0, 1.0 / 3.0),
        (1.0, 1.0),
    )
}

fn vertical() -> CubicBez {
    CubicBez::new((0.0, -1.0), (0.0, -1.0 / 3.0), (0.0, 1.0 / 3.0), (0.0, 1.0))
}

fn to_svg(c: CubicBez) -> String {
    BezPath::from_path_segments([PathSeg::Cubic(c)].into_iter()).to_svg()
}

fn depth_color(depth: f64) -> String {
    let start = 200.0;
    let end = 382.0;
    let [r, g, b] = color::Srgb::clip(color::Oklch::convert::<color::Srgb>([
        0.6,
        0.3,
        start + (end - start) * depth as f32,
    ]));
    let c = |x: f32| (x * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", c(r), c(g), c(b))
}

fn compare_curves(
    c0: CubicBez,
    c1: CubicBez,
    eps: f64,
    max_depth: u32,
) -> svg::node::element::Group {
    let mut g = svg::node::element::Group::new();
    let cmp = intersect_cubics(c0, c1, 0.5 * eps, eps);

    g = g.add(
        svg::node::element::Path::new()
            .set("d", to_svg(c1))
            .set("fill", "none")
            .set("stroke", "black")
            .set("stroke-width", 0.01)
            .set("stroke-linecap", "round")
            .set("stroke-linejoin", "round"),
    );

    for (start, end, order) in cmp.iter() {
        let color = match order {
            Order::Right => "green",
            Order::Ish => "red",
            Order::Left => "blue",
        };
        let c = y_subsegment(c0, start, end);
        g = g.add(
            svg::node::element::Path::new()
                .set("d", to_svg(c))
                .set("fill", "none")
                .set("stroke", color)
                .set("stroke-width", 0.01)
                .set("stroke-linecap", "round")
                .set("stroke-linejoin", "round"),
        );
    }

    let mut start = -1.0;
    for m in &cmp.metrics.entries {
        if m.depth == 0 {
            start = m.end;
            continue;
        }
        let w = (m.depth as f64 / max_depth as f64).clamp(0.01, 1.0);
        let h = (m.end - start).max(0.01);
        g = g.add(
            svg::node::element::Rectangle::new()
                .set("width", w)
                .set("height", h)
                .set("x", 1.5)
                .set("y", start)
                .set("fill", depth_color(w)),
        );
        start = m.end;
    }

    g
}

pub fn main() {
    let curves = [
        (diagonal(), flip(diagonal())),
        (diagonal(), shift(diagonal(), 0.01)),
        (c(), vertical()),
        (c(), diagonal()),
        (c(), flip(c())),
        (c(), shift(c(), 0.01)),
        (horiz_at_midpoint(), diagonal()),
        (horiz_at_midpoint(), c()),
        (horiz_at_midpoint(), flip(horiz_at_midpoint())),
        (horiz_at_midpoint(), shift(horiz_at_midpoint(), 0.01)),
    ];
    let mut doc =
        svg::Document::new().set("viewBox", (0.0, 0.0, 5.5, 1.0 + 2.5 * curves.len() as f64));
    for (i, &(c0, c1)) in curves.iter().enumerate() {
        let y = i as f64 * 2.5 + 1.5;
        let g = compare_curves(c0, c1, 1e-6, 6).set("transform", format!("translate(0, {y})",));
        doc = doc.add(g);
    }
    svg::save("out.svg", &doc).unwrap();
}
