// SPDX-License-Identifier: MIT OR Apache-2.0
//! Setting a graph's parameters by name, for hosts that render a graph with values of their own: an exposed parameter
//! by its label ([`Graph::apply_params`]), or an `input_value` node by its `name` (the values function graphs read
//! through `f_graph_input`).

use crate::graph::Graph;

/// The keys of an `input_value` node's value: one number, or up to four lanes.
const LANES: [&str; 4] = ["x", "y", "z", "w"];

impl Graph {
    /// Sets the parameter `name` to `values`: the exposed parameter labelled `name` (one value, clamped to its range),
    /// else every `input_value` node whose `name` is `name` (one value sets `v`; two to four set the lanes `x y z w`).
    /// Non-finite values are refused. Errors name what is wrong; the graph is unchanged then.
    pub fn set_param(&mut self, name: &str, values: &[f32]) -> Result<(), String> {
        if values.is_empty() || values.len() > LANES.len() {
            return Err(format!("{name}: give one to four values, not {}", values.len()));
        }
        if values.iter().any(|v| !v.is_finite()) {
            return Err(format!("{name}: values must be finite numbers"));
        }
        if let Some(e) = self.exposed.iter().find(|e| e.label == name).cloned() {
            let [v] = values else { return Err(format!("{name}: an exposed parameter takes one value")) };
            let n = self.nodes.get_mut(&e.node).ok_or_else(|| format!("{name}: exposed node {} is missing", e.node))?;
            n.nums.insert(e.param.clone(), v.clamp(e.min.min(e.max), e.min.max(e.max)));
            return Ok(());
        }
        let ids: Vec<u32> =
            self.nodes.iter().filter(|(_, n)| n.kind == "input_value" && n.strs.get("name").is_some_and(|s| s == name)).map(|(id, _)| *id).collect();
        if ids.is_empty() {
            return Err(format!("no exposed parameter or input value named '{name}'"));
        }
        for id in ids {
            let Some(n) = self.nodes.get_mut(&id) else { continue };
            match values {
                [v] => {
                    n.nums.insert("v".to_string(), *v);
                }
                lanes => {
                    for (key, v) in LANES.iter().zip(lanes) {
                        n.nums.insert((*key).to_string(), *v);
                    }
                }
            }
        }
        Ok(())
    }

    /// Keeps only the graph outputs named in `names` (by identifier, as [`Graph::output_nodes`] gives them), so an
    /// evaluation computes nothing the others alone need. An output that a kept one reads from stays. Errors name an
    /// output the graph doesn't have.
    pub fn keep_outputs(&mut self, names: &[&str]) -> Result<(), String> {
        let have: Vec<String> = self.output_nodes().into_iter().map(|(_, ident, _)| ident).collect();
        if let Some(missing) = names.iter().find(|n| !have.iter().any(|h| h == *n)) {
            return Err(format!("no output '{missing}' (outputs: {})", have.join(", ")));
        }
        let keep = |ident: &str| names.contains(&ident);
        // an output node another kept output reads from stays too
        let roots: Vec<u32> = self.output_nodes().into_iter().filter(|(_, ident, _)| keep(ident)).map(|(_, _, id)| id).collect();
        let needed = crate::graph::reached(self, &roots);
        let drop: Vec<u32> = self.nodes.iter().filter(|(id, n)| n.kind == "output" && !needed.contains(id)).map(|(id, _)| *id).collect();
        for id in drop {
            self.nodes.remove(&id);
        }
        self.outputs.retain(|(name, _)| keep(name));
        let pins: Vec<String> = self.output_pins.keys().filter(|k| !keep(k)).cloned().collect();
        for k in pins {
            self.output_pins.remove(&k);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::graph::Graph;

    const TEXT: &str = "size 8\nnode 1 input_value name=\"level\" type=0 v=0.25\nnode 2 color r=1 g=0 b=0\noutput lum 1\noutput red 2\n";

    #[test]
    fn an_input_value_is_set_by_name_and_changes_the_render() {
        let mut g = Graph::parse(TEXT).unwrap();
        assert_eq!(g.evaluate(None).unwrap()["lum"].px[0][0], 0.25);
        g.set_param("level", &[0.75]).unwrap();
        assert_eq!(g.evaluate(None).unwrap()["lum"].px[0][0], 0.75);
        assert!(g.set_param("nothing", &[1.0]).unwrap_err().contains("nothing"));
        assert!(g.set_param("level", &[]).is_err());
        assert!(g.set_param("level", &[f32::NAN]).is_err());
        assert!(g.set_param("level", &[1.0; 5]).is_err());
    }

    #[test]
    fn kept_outputs_are_the_only_ones_evaluated() {
        let mut g = Graph::parse(TEXT).unwrap();
        g.keep_outputs(&["red"]).unwrap();
        let out = g.evaluate(None).unwrap();
        assert_eq!(out.keys().collect::<Vec<_>>(), ["red"]);
        assert!(g.keep_outputs(&["blue"]).unwrap_err().contains("blue"));
        let mut m = Graph::parse(TEXT).unwrap();
        m.migrate_outputs();
        m.keep_outputs(&["lum"]).unwrap();
        assert_eq!(m.evaluate(None).unwrap().keys().collect::<Vec<_>>(), ["lum"]);
    }

    #[test]
    fn an_output_a_kept_output_reads_is_not_dropped() {
        let text = "size 8\nnode 1 color r=1 g=0 b=0\nnode 2 output usage=\"basecolor\" identifier=\"mid\" in=@1\nnode 3 output usage=\"roughness\" identifier=\"last\" in=@2\nnode 4 output usage=\"metallic\" identifier=\"other\" in=@1\n";
        let mut g = Graph::parse(text).unwrap();
        g.keep_outputs(&["last"]).unwrap();
        assert!(!g.nodes.contains_key(&4), "an output nobody needs is dropped");
        let out = g.evaluate(None).expect("the kept output still has what it reads");
        // `mid` stays because `last` reads it, so it is evaluated and shows in the result as well
        assert_eq!(out.keys().collect::<Vec<_>>(), ["last", "mid"]);
        assert_eq!(out["last"].px[0], [1.0, 0.0, 0.0, 1.0]);
    }
}
