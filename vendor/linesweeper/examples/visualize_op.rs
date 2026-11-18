use std::path::PathBuf;

use anyhow::{anyhow, bail};
use clap::Parser;
use kurbo::BezPath;
use svg::Document;

use linesweeper::{
    BinaryOp, FillRule, binary_op,
    topology::{Contours, Topology},
};

#[derive(Parser)]
struct Cli {
    #[arg(long)]
    output: PathBuf,

    #[arg(long)]
    input: PathBuf,
}

pub fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    let input = std::fs::read_to_string(&args.input)?;
    let mut input_lines = input.lines();
    let op = input_lines.next().ok_or(anyhow!("no op line"))?;
    let op = match op {
        "union" => BinaryOp::Union,
        "intersection" => BinaryOp::Intersection,
        "difference" => BinaryOp::Difference,
        "xor" => BinaryOp::Xor,
        _ => bail!("unknown op {op}"),
    };

    let shape_a = input_lines.next().ok_or(anyhow!("no first shape"))?;
    let shape_b = input_lines.next().ok_or(anyhow!("no second shape"))?;

    for extra_line in input_lines {
        if !extra_line.trim().is_empty() {
            eprintln!("ignoring extra line: {extra_line}");
        }
    }

    let shape_a = BezPath::from_svg(shape_a)?;
    let shape_b = BezPath::from_svg(shape_b)?;

    let eps = 1e-5;
    let contours = binary_op(&shape_a, &shape_b, FillRule::EvenOdd, op)?;
    let top = Topology::from_paths_binary(&shape_a, &shape_b, eps)?;
    let bbox = top.bounding_box();
    let min_x = bbox.min_x();
    let min_y = bbox.min_y();
    let max_x = bbox.max_x();
    let max_y = bbox.max_y();
    let pad = 1.0 + eps;
    let one_width = max_x - min_x + 2.0 * pad;
    let one_height = max_y - min_y + 2.0 * pad;
    let stroke_width = (max_y - min_y).max(max_x - max_y) / 512.0;

    let mut document = svg::Document::new().set(
        "viewBox",
        (min_x - pad, min_y - pad, one_width * 2.0, one_height),
    );

    // Draw the original document.
    for (c, color) in [(&shape_a, "green"), (&shape_b, "blue")] {
        let mut data = svg::node::element::path::Data::new();
        for el in c {
            let p = |point: kurbo::Point| (point.x, point.y);
            data = match el {
                kurbo::PathEl::MoveTo(p0) => data.move_to(p(p0)),
                kurbo::PathEl::LineTo(p0) => data.line_to(p(p0)),
                kurbo::PathEl::QuadTo(p0, p1) => data.quadratic_curve_to((p(p0), p(p1))),
                kurbo::PathEl::CurveTo(p0, p1, p2) => data.cubic_curve_to((p(p0), p(p1), p(p2))),
                kurbo::PathEl::ClosePath => data.close(),
            };
        }

        let path = svg::node::element::Path::new()
            .set("stroke", "black")
            .set("stroke-width", stroke_width)
            .set("stroke-linecap", "round")
            .set("stroke-linejoin", "round")
            .set("opacity", 0.5)
            .set("fill", color)
            .set("d", data);
        document = document.add(path);
    }

    document = add_op(document, &contours, one_width, stroke_width);

    svg::save(&args.output, &document)?;

    Ok(())
}

fn add_op(mut doc: Document, contours: &Contours, x_off: f64, stroke_width: f64) -> Document {
    for group in contours.grouped() {
        let mut data = svg::node::element::path::Data::new();

        for contour_idx in group {
            let path = &contours[contour_idx].path;
            for el in path.iter() {
                data = match el {
                    kurbo::PathEl::MoveTo(p) => data.move_to((p.x + x_off, p.y)),
                    kurbo::PathEl::LineTo(p) => data.line_to((p.x + x_off, p.y)),
                    kurbo::PathEl::QuadTo(p0, p1) => {
                        data.quadratic_curve_to(((p0.x + x_off, p0.y), (p1.x + x_off, p1.y)))
                    }
                    kurbo::PathEl::CurveTo(p0, p1, p2) => data.cubic_curve_to((
                        (p0.x + x_off, p0.y),
                        (p1.x + x_off, p1.y),
                        (p2.x + x_off, p2.y),
                    )),
                    kurbo::PathEl::ClosePath => data.close(),
                };
            }
            data = data.close();
        }
        let path = svg::node::element::Path::new()
            .set("d", data)
            .set("stroke", "black")
            .set("stroke-width", stroke_width)
            .set("stroke-linecap", "round")
            .set("stroke-linejoin", "round")
            .set("fill", "pink");
        doc = doc.add(path);
    }
    doc
}
