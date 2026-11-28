// SPDX-License-Identifier: MIT OR Apache-2.0
//! Holds the fast interpreter to the reference tree walk: random functions, heavy on the things whose order matters
//! (variables, loops, random numbers, nodes shared between users), must give the same results, the same errors and
//! leave the same variables and random position.

use super::*;
use crate::math;
use crate::tex::Tex;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, pct: usize) -> bool {
        self.below(100) < pct
    }
}

const VARS: [&str; 4] = ["a", "b", "c", "long_name"];

/// Text of a random function of about `n` nodes. `style` shifts what it is made of.
fn random_function(r: &mut Rng, n: usize, style: usize, callee: Option<&str>) -> String {
    let mut lines = Vec::new();
    let mut at = 2u32;
    let pick = |r: &mut Rng, at: u32| -> u32 {
        // mostly recent nodes, so chains build up and nodes get shared
        if at <= 2 {
            return 2;
        }
        if r.chance(70) { at - 1 - r.below((at as usize - 2).min(6)) as u32 } else { 2 + r.below(at as usize - 2) as u32 }
    };
    lines.push("node 2 f_float v=1.5".to_string());
    at += 1;
    for _ in 0..n {
        let (x, y, z) = (pick(r, at), pick(r, at), pick(r, at));
        let v = VARS[r.below(VARS.len())];
        let kinds = match style {
            0 => r.below(14),
            1 => [6, 7, 7, 8, 9, 9, 10, 0, 11, 8, 6, 13][r.below(12)],
            _ => r.below(30),
        };
        let line = match kinds {
            0 => format!("f_add a=@{x} b=@{y}"),
            1 => format!("f_mul a=@{x} b=@{y}"),
            2 => format!("f_float v={}", r.below(9) as f32 * 0.25 - 1.0),
            3 => "f_pos".to_string(),
            4 => format!("f_vec2 x=@{x} y=@{y}"),
            5 => format!("f_swizzle a=@{x} mask=\"{}\"", ["x", "yx", "xy", "z", "xxyy", "q"][r.below(6)]),
            6 => format!("f_set a=@{x} name=\"{v}\""),
            7 => format!("f_get name=\"{v}\""),
            8 => format!("f_sequence a=@{x} b=@{y}"),
            9 => {
                lines.push(format!("node {at} f_gt a=@{x} b=@{y}"));
                at += 1;
                format!("f_if cond=@{} a=@{y} b=@{z}", at - 1)
            }
            10 => match r.below(4) {
                0 => format!("f_random seed={} b=@{x} s=@{y} lcg={} k={}", r.below(3), [1, 2, 3, 5, 10, 12][r.below(6)], r.below(3)),
                _ => format!("f_random seed={} b=@{x}", r.below(3)),
            },
            11 => {
                if r.chance(50) {
                    format!("f_loop a=@{x} c=@{y} passes={}", r.below(4))
                } else {
                    lines.push(format!("node {at} f_gt a=@{x} b=@{z}"));
                    at += 1;
                    format!("f_loop a=@{x} b=@{} c=@{y} passes={}", at - 1, r.below(5))
                }
            }
            12 => format!("f_sub a=@{x} b=@{y}"),
            13 => format!("f_to_float a=@{x}"),
            14 => format!("f_to_int a=@{x}"),
            15 => format!("f_int v={}", r.below(6)),
            16 => format!("f_get name=\"{v}\" type={}", [0, 1, 3, 4, 5, 2][r.below(6)]),
            17 => "f_size".to_string(),
            18 => "f_index".to_string(),
            19 => format!("f_input name=\"p{}\" type={} v=0.25", r.below(3), r.below(6)),
            20 => format!("f_graph_input name=\"g{}\" type={} v=0.5", r.below(3), r.below(6)),
            21 => format!("f_sample_gray a=@{x}"),
            22 => format!("f_sample_color a=@{x} filter={}", r.below(4)),
            23 => format!("f_lerp a=@{x} b=@{y} c=@{z}"),
            24 => format!("f_div_large a=@{x} b=@{y}"),
            25 => format!("f_to_bool a=@{x}"),
            26 => match callee {
                Some(f) => format!("f_call fn=\"{f}\" a=@{x} b=@{y} d=@{z}"),
                None => "f_seed".to_string(),
            },
            27 => format!("f_string v=\"t{}\"", r.below(2)),
            28 => "f_image_avg".to_string(),
            _ => format!("f_vec3 x=@{x} z=@{y}"),
        };
        lines.push(format!("node {at} {line}"));
        at += 1;
    }
    // reach a good part of what was made, in sequence
    let mut last = at - 1;
    for _ in 0..r.below(6) {
        let x = 2 + r.below(at as usize - 2) as u32;
        lines.push(format!("node {at} f_sequence a=@{x} b=@{last}"));
        last = at;
        at += 1;
    }
    lines.push(format!("node 1 f_out in=@{last}"));
    lines.join("\n")
}

fn same_result(a: &Result<Value, String>, b: &Result<Value, String>) -> bool {
    match (a, b) {
        (Ok(Value::Num(x)), Ok(Value::Num(y))) => x.lanes == y.lanes && x.int == y.int && x.v.map(f64::to_bits) == y.v.map(f64::to_bits),
        _ => a == b,
    }
}

fn sorted(s: &FnState) -> Vec<(String, Value)> {
    let mut v = s.vars.clone();
    v.sort_by(|p, q| p.0.cmp(&q.0));
    // a variable set to what an unset one reads as is the same to everything that can look
    v.retain(|(_, x)| *x != Value::float(0.0));
    v
}

fn check(src: &str, lib: &BTreeMap<String, Graph>, images: &[(&str, &Tex)], ginputs: &BTreeMap<String, Value>) -> (bool, bool) {
    let g = Graph::parse_unchecked(&format!("meta kind \"function\"\n{src}")).unwrap_or_else(|e| panic!("{e:?}\n{src}"));
    let Ok(p) = Prog::compile(&g, lib) else { return (false, false) };
    let mut env = FnEnv::new([16.0, 32.0]);
    env.seed = 5;
    env.index = 3;
    env.graph_inputs = ginputs;
    env.images = images.iter().map(|(n, t)| (*n, (*t).into())).collect();
    env = env.with_stats();
    env.args.insert("p1".into(), Value::vec(2, [0.5, -0.5, 0.0, 0.0]));
    let mut frame = p.frame();
    p.bind(&env, &mut frame);
    let mut any_ok = false;
    // the same frame over several runs: what carries over must not show
    for pos in [[0.25, 0.75], [0.5, 0.5], [0.25, 0.75], [0.0, 1.0]] {
        let want = p.ref_run(&env, pos);
        let got = p.run_bound(&env, pos, &mut frame);
        assert!(same_result(&want, &got), "at {pos:?}: reference {want:?}, got {got:?}\n{src}");
        any_ok |= want.is_ok();
    }
    // carried state: a run that starts from what the last one left
    let (mut sa, mut sb) = (FnState::default(), FnState::default());
    let mut f2 = p.frame();
    p.bind(&env, &mut f2);
    for pos in [[0.25, 0.75], [0.5, 0.5], [0.5, 0.5]] {
        let want = p.ref_run_in(&env, pos, &mut sa);
        let got = p.run_in_bound(&env, pos, &mut sb, &mut f2);
        assert!(same_result(&want, &got), "in state at {pos:?}: reference {want:?}, got {got:?}\n{src}");
        assert_eq!(sa.rng, sb.rng, "random position differs\n{src}");
        assert_eq!(sa.lcg_state, sb.lcg_state, "generator state differs\n{src}");
        assert_eq!(sorted(&sa), sorted(&sb), "variables differ\n{src}");
    }
    // a new binding: nothing from the old environment may survive
    env.seed = 9;
    env.index = 4;
    env.args.insert("p0".into(), Value::float(2.0));
    p.bind(&env, &mut frame);
    for pos in [[0.25, 0.75], [0.5, 0.5]] {
        let want = p.ref_run(&env, pos);
        let got = p.run_bound(&env, pos, &mut frame);
        assert!(same_result(&want, &got), "after rebinding at {pos:?}: reference {want:?}, got {got:?}\n{src}");
    }
    (true, any_ok)
}

#[test]
fn random_functions_match_the_reference() {
    let tex = Tex::from_fn(4, 4, |u, v| [u, v, u * v, 1.0]);
    let images: Vec<(&str, &Tex)> = vec![("in", &tex)];
    let mut ginputs = BTreeMap::new();
    ginputs.insert("g1".to_string(), Value::int(3));
    ginputs.insert("g0".to_string(), Value::vec(3, [1.0, 2.0, 3.0, 0.0]));
    let mut r = Rng(0x9E3779B97F4A7C15);
    let (mut compiled, mut ran) = (0, 0);
    let rounds = std::env::var("TC_FUZZ_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(6000);
    for round in 0..rounds {
        let n = 3 + r.below(if round % 10 == 0 { 45 } else { 18 });
        let mut lib = BTreeMap::new();
        let callee_src = random_function(&mut r, 6, round % 2, None);
        lib.insert(
            "callee".to_string(),
            Graph::parse_unchecked(&format!("meta kind \"function\"\nnode 90 f_input name=\"p0\"\nnode 91 f_input name=\"p1\" type=3\n{callee_src}"))
                .unwrap_or_else(|e| panic!("{e:?}")),
        );
        let src = random_function(&mut r, n, round % 3, Some("callee"));
        let (c, k) = check(&src, &lib, &images, &ginputs);
        compiled += c as usize;
        ran += k as usize;
    }
    // a generator that only makes broken functions would prove nothing
    eprintln!("{rounds} functions: {compiled} compiled, {ran} ran");
    assert!(compiled > rounds / 2 && ran > rounds / 4, "compiled {compiled}, ran {ran}");
}

/// A sum of terms where each term sets variables from the sum before it, reading that sum twice: each node runs once.
fn doubling(terms: usize) -> String {
    let mut s = String::from("node 2 f_float v=1\nnode 3 f_set a=@2 name=\"v\"\n");
    let mut sum = 3u32;
    let mut at = 4u32;
    for k in 0..terms {
        s += &format!("node {at} f_float v={}\n", k as f32 * 0.5);
        s += &format!("node {} f_set a=@{at} name=\"v\"\n", at + 1);
        s += &format!("node {} f_set a=@{sum} name=\"w\"\n", at + 2);
        s += &format!("node {} f_sequence a=@{} b=@{}\n", at + 3, at + 1, at + 2);
        s += &format!("node {} f_add a=@{sum} b=@{}\n", at + 4, at + 3);
        sum = at + 4;
        at += 5;
    }
    s += &format!("node 1 f_out in=@{sum}\n");
    s
}

#[test]
fn chains_that_read_themselves_twice_stay_linear() {
    let lib = BTreeMap::new();
    let env = FnEnv::new([8.0, 8.0]);
    let small = Graph::parse_unchecked(&format!("meta kind \"function\"\n{}", doubling(12))).unwrap();
    let p = Prog::compile(&small, &lib).unwrap();
    assert_eq!(p.ref_run(&env, [0.5, 0.5]), p.run(&env, [0.5, 0.5]));
    let large = Graph::parse_unchecked(&format!("meta kind \"function\"\n{}", doubling(200))).unwrap();
    let p = Prog::compile(&large, &lib).unwrap();
    let mut f = p.frame();
    p.bind(&env, &mut f);
    p.run_bound(&env, [0.5, 0.5], &mut f).unwrap();
    assert!(f.counts.computed < 10_000, "{:?}", f.counts);
}

/// Not a test: `TC_COUNT_GRAPH=x.tcgraph cargo test --release --lib func::differential::count -- --ignored --nocapture` shows how
/// much a run of each function of a graph computes, which is where its time goes.
#[test]
#[ignore]
fn count_work_of_a_graph() {
    let path = std::env::var("TC_COUNT_GRAPH").unwrap_or_default();
    let g = Graph::parse(&std::fs::read_to_string(path).unwrap_or_default()).unwrap_or_else(|e| panic!("{e:?}"));
    let tex = Tex::from_fn(8, 8, |u, v| [u, v, u * v, 1.0]);
    let names: Vec<&String> = g.subgraphs.iter().filter(|(_, f)| is_function(f)).map(|(k, _)| k).collect();
    let ginputs = graph_inputs(&g);
    let mut rows = Vec::new();
    for name in names {
        let Ok(p) = compile_named(&g.subgraphs, name) else { continue };
        let mut env = FnEnv::new([64.0, 64.0]);
        env.graph_inputs = &ginputs;
        for slot in crate::graph::IMAGE_SLOTS {
            env.images.push((slot, (&tex).into()));
        }
        env = env.with_stats();
        let mut f = p.frame();
        p.bind(&env, &mut f);
        let first = p.run_bound(&env, [0.3, 0.7], &mut f);
        let c1 = f.counts;
        for k in 0..20 {
            let _ = p.run_bound(&env, [0.3 + 0.01 * k as f64, 0.2], &mut f);
        }
        let c2 = f.counts;
        let t = std::time::Instant::now();
        let mut runs = 0;
        while t.elapsed().as_secs_f64() < 0.02 && runs < 2000 {
            let _ = p.run_bound(&env, [0.3 + 0.001 * (runs % 500) as f64, 0.2], &mut f);
            runs += 1;
        }
        let per_run = t.elapsed().as_secs_f64() / runs as f64 * 1e6;
        rows.push((c2.computed - c1.computed, name.clone(), p.nodes.len(), c1, c2, per_run, first.is_ok()));
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    for r in rows.iter().take(8) {
        println!("{:>12} per 20 runs  {} nodes={} first={:?} after={:?} {:.1}us/run ok={}", r.0, r.1, r.2, r.3, r.4, r.5, r.6);
    }
}

/// Not a test by default: `TC_GRAPH_CORPUS=dir cargo test --release --lib func::differential::corpus -- --ignored --nocapture` runs
/// every function of every `.tcgraph` in `dir` with the reference and the fast form, at a few positions over one frame. Functions
/// the reference can't finish within its step budget are counted as skipped, not as matching.
#[test]
#[ignore]
fn corpus_functions_match_the_reference() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = std::env::var("TC_GRAPH_CORPUS").unwrap_or_default();
    let budget = std::env::var("TC_GRAPH_BUDGET").ok().and_then(|v| v.parse().ok()).unwrap_or(3_000_000u64);
    let threads = std::env::var("TC_GRAPH_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(8usize);
    let filter = std::env::var("TC_GRAPH_ONLY").unwrap_or_default();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "tcgraph") && p.to_string_lossy().contains(&filter))
        .collect();
    files.sort();
    let tex = Tex::from_fn(16, 16, |u, v| [u, v, math::sin(u * 37.0).abs(), 0.5 + 0.5 * math::cos(v * 23.0)]);
    let next = AtomicUsize::new(0);
    let (same, skipped, failed, errors) = (AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0));
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).stack_size(64 << 20).build().unwrap();
    pool.scope(|s| {
        for _ in 0..threads {
            s.spawn(|_| {
                loop {
                    let Some(path) = files.get(next.fetch_add(1, Ordering::Relaxed)) else { return };
                    let Ok(g) = Graph::parse(&std::fs::read_to_string(path).unwrap_or_default()) else { continue };
                    let ginputs = graph_inputs(&g);
                    let (mut ok, mut skip, mut bad, mut err) = (0, 0, 0, 0);
                    for (name, f) in g.subgraphs.iter().filter(|(_, f)| is_function(f)) {
                        let Ok(p) = compile_named(&g.subgraphs, name) else { continue };
                        let _ = f;
                        let mut env = FnEnv::new([64.0, 128.0]);
                        env.graph_inputs = &ginputs;
                        env.seed = 3;
                        for slot in crate::graph::IMAGE_SLOTS.iter().take(4) {
                            env.images.push((slot, (&tex).into()));
                        }
                        if p.uses_stats {
                            env = env.with_stats();
                        }
                        let mut frame = p.frame();
                        p.bind(&env, &mut frame);
                        for pos in [[0.5, 0.5], [0.1, 0.8], [0.73, 0.21], [0.1, 0.8]] {
                            let want = p.ref_run_budget(&env, pos, budget);
                            let got = p.run_bound(&env, pos, &mut frame);
                            if matches!(&want, Err(e) if e == reference::OVER_BUDGET) {
                                skip += 1;
                            } else if same_result(&want, &got) {
                                ok += 1;
                                err += want.is_err() as usize;
                            } else {
                                bad += 1;
                                eprintln!("DIFF {} {name} at {pos:?}: reference {want:?}, got {got:?}", path.display());
                            }
                        }
                    }
                    println!("{} same {ok} (errors {err}) skipped {skip} different {bad}", path.file_stem().unwrap_or_default().to_string_lossy());
                    same.fetch_add(ok, Ordering::Relaxed);
                    skipped.fetch_add(skip, Ordering::Relaxed);
                    failed.fetch_add(bad, Ordering::Relaxed);
                    errors.fetch_add(err, Ordering::Relaxed);
                }
            });
        }
    });
    println!(
        "TOTAL same {} (of which errors {}) skipped {} different {}",
        same.into_inner(),
        errors.into_inner(),
        skipped.into_inner(),
        failed.load(Ordering::Relaxed)
    );
    assert_eq!(failed.load(Ordering::Relaxed), 0);
}
