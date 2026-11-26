// SPDX-License-Identifier: MIT OR Apache-2.0
use craft_material_graph::func::*;
use craft_material_graph::graph::{Graph, Pin};
use std::collections::BTreeMap;

fn fg(body: &str) -> Graph {
    Graph::parse_unchecked(&format!("meta kind \"function\"\n{body}")).unwrap()
}
fn run(body: &str) -> Value {
    let g = fg(body);
    let lib = BTreeMap::new();
    Prog::compile(&g, &lib).unwrap().run(&FnEnv::new([64.0, 64.0]), [0.25, 0.75]).unwrap()
}

#[test]
fn math_and_vectors() {
    assert_eq!(run("node 1 f_float v=2\nnode 2 f_float v=3\nnode 3 f_add a=@1 b=@2\nnode 4 f_out in=@3\n"), Value::float(5.0));
    // integer division truncates, float division by zero is 0
    assert_eq!(run("node 1 f_int v=7\nnode 2 f_int v=2\nnode 3 f_div a=@1 b=@2\nnode 4 f_out in=@3\n"), Value::int(3));
    assert_eq!(run("node 1 f_float v=7\nnode 2 f_float v=0\nnode 3 f_div a=@1 b=@2\nnode 4 f_out in=@3\n"), Value::float(0.0));
    // vector + scalar, swizzle
    let v = run(
        "node 1 f_float x=1 y=2 z=3\nnode 5 f_vec3 x=1 y=2 z=3\nnode 2 f_float v=10\nnode 3 f_add a=@5 b=@2\nnode 4 f_swizzle a=@3 mask=\"zy\"\nnode 6 f_out in=@4\n",
    );
    assert_eq!(v, Value::vec(2, [13.0, 12.0, 0.0, 0.0]));
    // negative sqrt / log never give NaN
    assert_eq!(run("node 1 f_float v=-4\nnode 2 f_sqrt a=@1\nnode 3 f_out in=@2\n"), Value::float(0.0));
    assert_eq!(run("node 1 f_float v=0\nnode 2 f_log a=@1\nnode 3 f_out in=@2\n"), Value::float(0.0));
    // mod has the sign of the divisor
    assert_eq!(run("node 1 f_float v=-1\nnode 2 f_float v=3\nnode 3 f_mod a=@1 b=@2\nnode 4 f_out in=@3\n"), Value::float(2.0));
}

#[test]
fn logic_variables_and_position() {
    // if only runs the chosen branch; comparison gives a bool
    let v = run(
        "node 1 f_float v=1\nnode 2 f_float v=2\nnode 3 f_lt a=@1 b=@2\nnode 4 f_string v=\"yes\"\nnode 5 f_string v=\"no\"\nnode 6 f_if cond=@3 a=@4 b=@5\nnode 7 f_out in=@6\n",
    );
    assert_eq!(v, Value::text("yes"));
    // variables: set 5, then read it back through a sequence
    let v = run("node 1 f_float v=5\nnode 2 f_set a=@1 name=\"k\"\nnode 3 f_get name=\"k\"\nnode 4 f_sequence a=@2 b=@3\nnode 5 f_out in=@4\n");
    assert_eq!(v, Value::float(5.0));
    // position is the sample position
    assert_eq!(run("node 1 f_pos\nnode 2 f_out in=@1\n"), Value::vec(2, [0.25, 0.75, 0.0, 0.0]));
}

#[test]
fn types_are_checked() {
    let lib = BTreeMap::new();
    let g = fg("node 1 f_float v=1\nnode 2 f_vec2 x=1 y=2\nnode 5 f_vec3 x=1 y=2 z=3\nnode 3 f_add a=@2 b=@5\nnode 4 f_out in=@3\n");
    let p = Prog::compile(&g, &lib).unwrap();
    assert!(p.result_type().is_err(), "float2 + float3 must be refused");
    let g = fg("node 2 f_vec2 x=1 y=2\nnode 4 f_out in=@2\n");
    assert_eq!(Prog::compile(&g, &lib).unwrap().result_type().unwrap(), Pin::Float2);
    // missing output, cycle
    assert!(Prog::compile(&fg("node 1 f_float v=1\n"), &lib).is_err());
    assert!(Prog::compile(&fg("node 1 f_neg a=@2\nnode 2 f_neg a=@1\nnode 3 f_out in=@1\n"), &lib).is_err());
}

#[test]
fn calls_and_inputs() {
    let mut lib = BTreeMap::new();
    lib.insert("twice".to_string(), fg("node 1 f_input name=\"x\" type=0 v=1\nnode 2 f_float v=2\nnode 3 f_mul a=@1 b=@2\nnode 4 f_out in=@3\n"));
    let main = fg("node 1 f_float v=21\nnode 2 f_call fn=\"twice\" a=@1\nnode 3 f_out in=@2\n");
    assert_eq!(Prog::compile(&main, &lib).unwrap().run(&FnEnv::new([8.0, 8.0]), [0.0, 0.0]).unwrap(), Value::float(42.0));
    // default used when nothing is bound
    assert_eq!(eval_once(&lib, "twice", &FnEnv::new([8.0, 8.0])).unwrap(), Value::float(2.0));
    // self call is refused instead of overflowing the stack
    lib.insert("loop".to_string(), fg("node 1 f_call fn=\"loop\"\nnode 2 f_out in=@1\n"));
    assert!(compile_named(&lib, "loop").is_err());
}

#[test]
fn random_is_deterministic() {
    let body = "node 1 f_random seed=3\nnode 2 f_out in=@1\n";
    assert_eq!(run(body), run(body));
    if let Value::Num(n) = run(body) {
        assert!((0.0..=1.0).contains(&n.v[0]));
    } else {
        panic!("random must be a number");
    }
}

#[test]
fn function_drives_parameter_and_graph_input() {
    // a function reads the graph's Input Value node and drives a colour node's red channel
    let text = "size 16\nsubgraph half\nmeta kind \"function\"\nnode 1 f_graph_input name=\"level\" type=0 v=0\nnode 2 f_float v=0.5\nnode 3 f_mul a=@1 b=@2\nnode 4 f_out in=@3\nend\nnode 1 input_value name=\"level\" type=0 v=0.8\nnode 2 color r=0 g=0 b=0 fn.r=\"half\"\noutput o 2\n";
    let g = Graph::parse(text).unwrap();
    let out = g.evaluate(Some(16)).unwrap();
    let px = out["o"].px[0];
    assert!((px[0] - 0.4).abs() < 1e-5, "red {}", px[0]);
    // changing the input value changes the result (cache invalidation)
    let g2 = Graph::parse(&text.replace("v=0.8", "v=0.2")).unwrap();
    assert!((g2.evaluate(Some(16)).unwrap()["o"].px[0][0] - 0.1).abs() < 1e-5);
    // a broken function is a clean error
    let bad = text.replace("fn.r=\"half\"", "fn.r=\"nope\"");
    assert!(Graph::parse(&bad).unwrap().evaluate(Some(16)).is_err());
}

#[test]
fn pixel_and_value_processors_run_functions() {
    // pixel processor: brightness = u (left to right ramp); value processor: average of the input
    let text = "size 16\nsubgraph ramp\nmeta kind \"function\"\nnode 1 f_pos\nnode 2 f_swizzle a=@1 mask=\"x\"\nnode 3 f_out in=@2\nend\nsubgraph avg\nmeta kind \"function\"\nnode 1 f_image_avg\nnode 2 f_swizzle a=@1 mask=\"x\"\nnode 3 f_out in=@2\nend\nnode 1 pixel_processor fn=\"ramp\"\nnode 2 value_processor fn=\"avg\" in=@1\noutput o 1\noutput v 2\n";
    let g = Graph::parse(text).unwrap();
    let out = g.evaluate(Some(16)).unwrap();
    let o = &out["o"];
    assert!(o.px[15][0] > o.px[0][0], "ramp must rise to the right");
    let v = out["v"].px[0][0];
    let mean: f32 = o.px.iter().map(|p| p[0]).sum::<f32>() / o.px.len() as f32;
    assert!((v - mean).abs() < 1e-4, "value {v} vs mean {mean}");
    // a function that returns a string is refused
    let bad = text.replace("node 3 f_out in=@2\nend\nsubgraph avg", "node 4 f_string v=\"x\"\nnode 3 f_out in=@4\nend\nsubgraph avg");
    assert!(Graph::parse(&bad).unwrap().evaluate(Some(16)).is_err());
}

#[test]
fn graphs_round_trip_and_preview() {
    let text = "size 32\nsubgraph half\nmeta kind \"function\"\nnode 1 f_pos\nnode 2 f_swizzle a=@1 mask=\"y\"\nnode 3 f_out in=@2\nend\nsubgraph dots\nmeta kind \"fxmap\"\nnode 1 fxm_marker\nnode 2 fxm_out in=@1\nend\nnode 1 pixel_processor fn=\"half\"\noutput o 1\n";
    let g = Graph::parse(text).unwrap();
    let again = Graph::parse(&g.to_text()).unwrap();
    assert_eq!(again.subgraphs["half"].graph_kind(), "function");
    assert_eq!(again.subgraphs["dots"].graph_kind(), "fxmap");
    assert_eq!(g.graph_kind(), "texture");
    assert_eq!(again.evaluate(Some(16)).unwrap()["o"].px, g.evaluate(Some(16)).unwrap()["o"].px);
    // a function or program edited on its own previews through a wrapper graph
    let mut f = again.subgraphs["half"].clone();
    f.size = 16;
    f.subgraphs = again.subgraphs.clone();
    let (host, show) = f.preview_host().unwrap();
    let img = host.evaluate_node(show, 16).unwrap();
    assert!(img.px[15 * 16][0] > img.px[0][0], "the preview shows a vertical ramp");
    assert!(g.preview_host().is_none());
    // typed readout for the editor
    let lib = again.subgraphs.clone();
    let v = probe_value(&lib["half"], &lib, 1, &BTreeMap::new()).unwrap();
    assert_eq!(v.pin(), Pin::Float2);
    assert!(probe_value(&lib["half"], &lib, 99, &BTreeMap::new()).is_err());
}
