use std::path::PathBuf;

use clap::Parser;

use linesweeper::topology::Topology;
use linesweeper_util::svg_to_bezpaths;

#[derive(Parser)]
struct Args {
    input: PathBuf,
    output: PathBuf,

    #[arg(long)]
    epsilon: Option<f64>,
}

pub fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let input = std::fs::read_to_string(&args.input)?;
    let tree = usvg::Tree::from_str(&input, &usvg::Options::default())?;
    let contours = svg_to_bezpaths(&tree);

    let eps = args.epsilon.unwrap_or(0.1);
    let top =
        Topology::from_paths_binary(&contours[0], &contours[1..].iter().flatten().collect(), eps)?;

    let bbox = top.bounding_box();
    let min_x = bbox.min_x();
    let min_y = bbox.min_y();
    let max_x = bbox.max_x();
    let max_y = bbox.max_y();
    let pad = 8.0 + eps;
    let stroke_width = (max_y - min_y).max(max_x - max_y) / 512.0;
    let dot_radius = stroke_width * 1.5;
    let mut document = svg::Document::new().set(
        "viewBox",
        (
            min_x - pad,
            min_y - pad,
            max_x - min_x + 2.0 * pad,
            max_y - min_y + 2.0 * pad,
        ),
    );

    let text_size = "8px";

    for seg in top.segment_indices() {
        let p0 = &top.point(seg.first_half());
        let p1 = &top.point(seg.second_half());
        let (x0, y0) = (p0.x, p0.y);
        let (x1, y1) = (p1.x, p1.y);
        let c = svg::node::element::Circle::new()
            .set("r", dot_radius)
            .set("cy", y0)
            .set("cx", x0)
            .set("opacity", 0.5)
            .set("fill", "blue");
        document = document.add(c);

        let c = svg::node::element::Circle::new()
            .set("r", dot_radius)
            .set("cy", y1)
            .set("cx", x1)
            .set("opacity", 0.5)
            .set("fill", "blue");
        document = document.add(c);

        let data = svg::node::element::path::Data::new()
            .move_to((x0, y0))
            .line_to((x1, y1));
        let path = svg::node::element::Path::new()
            .set("stroke", "black")
            .set("stroke-width", stroke_width / 2.0)
            .set("stroke-opacity", "0.5")
            .set("d", data);
        document = document.add(path);

        let nx = y1 - y0;
        let ny = x0 - x1;
        let norm = ((nx * nx) + (ny * ny)).sqrt();

        let nx = nx / norm * 3.0 * dot_radius;
        let ny = ny / norm * 3.0 * dot_radius;

        let text = svg::node::element::Text::new(format!(
            "{:?}",
            top.winding_counter_clockwise(seg.first_half())
        ))
        .set("font-size", text_size)
        .set("text-anchor", "start")
        .set("x", (x0 + x1) / 2.0 + nx)
        .set("y", (y0 + y1) / 2.0 + ny);
        document = document.add(text);

        let text =
            svg::node::element::Text::new(format!("{:?}", top.winding_clockwise(seg.first_half())))
                .set("font-size", text_size)
                .set("text-anchor", "end")
                .set("x", (x0 + x1) / 2.0 - nx)
                .set("y", (y0 + y1) / 2.0 - ny);
        document = document.add(text);
    }

    svg::save(&args.output, &document)?;

    Ok(())
}
