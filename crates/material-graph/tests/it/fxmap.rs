// SPDX-License-Identifier: MIT OR Apache-2.0
use craft_material_graph::graph::Graph;

fn eval(text: &str) -> craft_material_graph::tex::Tex {
    Graph::parse(text).unwrap().evaluate(Some(64)).unwrap().remove("o").unwrap()
}

const GRID: &str = "size 64\nsubgraph dots\nmeta kind \"fxmap\"\nnode 1 fxm_marker shape=0 w=0.6 h=0.6\nnode 2 fxm_iterate body=@1 grid_x=4 grid_y=4\nnode 3 fxm_out in=@2\nend\nnode 1 fx_map program=\"dots\"\noutput o 1\n";

#[test]
fn grid_of_dots() {
    let t = eval(GRID);
    // 4x4 grid: the middle of each 16 px cell is lit, the corner between four cells is dark
    assert!(t.px[8 * 64 + 8][0] > 0.9, "cell centre lit");
    assert!(t.px[0][0] < 0.1, "corner dark");
    assert!(t.px[16 * 64 + 16][0] < 0.1, "cell corner dark");
    // the pattern tiles: it is identical after one cell shift
    assert_eq!(t.px[8 * 64 + 8][0], t.px[24 * 64 + 40][0]);
}

#[test]
fn quadrants_recurse_and_are_deterministic() {
    let text = "size 64\nsubgraph q\nmeta kind \"fxmap\"\nnode 1 fxm_marker shape=1 w=0.8 h=0.8\nnode 2 fxm_quadrant q0=@1 q3=@1 scale=0.5\nnode 3 fxm_quadrant q0=@2 q1=@2 q2=@2 q3=@2 scale=0.5 p1=0.5 rnd_scale=0.1\nnode 4 fxm_out in=@3\nend\nnode 1 fx_map program=\"q\" seed=3\noutput o 1\n";
    let a = eval(text);
    let b = eval(text);
    assert_eq!(a.px, b.px);
    let lit = a.px.iter().filter(|p| p[0] > 0.5).count();
    assert!(lit > 100 && lit < 64 * 64, "some but not all pixels drawn, got {lit}");
    // another seed changes the random parts
    let c = eval(&text.replace("seed=3", "seed=9"));
    assert_ne!(a.px, c.px);
}

#[test]
fn switch_blend_and_errors() {
    let text = "size 32\nsubgraph s\nmeta kind \"fxmap\"\nnode 1 fxm_marker shape=1 w=1 h=1\nnode 2 fxm_marker shape=1 w=0.2 h=0.2\nnode 3 fxm_switch a=@1 b=@2 index=1\nnode 4 fxm_out in=@3\nend\nnode 1 fx_map program=\"s\"\noutput o 1\n";
    let t = eval(text);
    assert!(t.px[0][0] < 0.1, "index 1 draws the small square only");
    assert!(t.px[32 * 64 + 32][0] > 0.9);
    // missing program and a program without output are errors, each with its own cause
    let message = |text: &str| Graph::parse(text).unwrap().evaluate(Some(32)).expect_err("the evaluation must fail").to_string();
    let unknown = message("size 32\nnode 1 fx_map program=\"nope\"\noutput o 1\n");
    assert!(unknown.contains("node 1") && unknown.contains("unknown pattern program 'nope'"), "{unknown}");
    let no_output = message("size 32\nsubgraph s\nmeta kind \"fxmap\"\nnode 1 fxm_marker\nend\nnode 1 fx_map program=\"s\"\noutput o 1\n");
    assert!(no_output.contains("node 1") && no_output.contains("no Output node"), "{no_output}");
    // runaway programs stop with an error instead of hanging
    let big = "size 32\nsubgraph s\nmeta kind \"fxmap\"\nnode 1 fxm_marker w=0.01 h=0.01\nnode 2 fxm_iterate body=@1 count=100000\nnode 3 fxm_iterate body=@2 count=100000\nnode 4 fxm_out in=@3\nend\nnode 1 fx_map program=\"s\"\noutput o 1\n";
    let limit = message(big);
    assert!(limit.contains("node 1") && limit.contains("more than 500000 markers"), "{limit}");
}
