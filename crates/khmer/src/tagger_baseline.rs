//! Aksara feature-set v3 averaged perceptron, ported without changing features.
use std::{collections::HashMap, sync::OnceLock};
fn weights() -> &'static HashMap<&'static str, f64> {
    static WEIGHTS: OnceLock<HashMap<&'static str, f64>> = OnceLock::new();
    WEIGHTS.get_or_init(|| {
        crate::assets::weights()
            .lines()
            .filter_map(|line| {
                let (feature, weight) = line.rsplit_once('\t')?;
                Some((feature, weight.parse().ok()?))
            })
            .collect()
    })
}
fn band(f: u64) -> &'static str {
    match f {
        0 => "0",
        1 => "demoted",
        2..100 => "rare",
        100..2000 => "mid",
        2000..20000 => "high",
        _ => "top",
    }
}
fn bucket(n: usize) -> &'static str {
    match n {
        0 => "0",
        1 => "1",
        2 => "2",
        3..=4 => "3-4",
        5..=6 => "5-6",
        _ => "7+",
    }
}
pub(super) fn boundary_score(clusters: &[&str], gap: usize) -> f64 {
    let at = |offset: isize| gap.checked_add_signed(offset).and_then(|i| clusters.get(i)).copied().unwrap_or("<edge>");
    let (m3, m2, m1, p0, p1, p2) = (at(-3), at(-2), at(-1), at(0), at(1), at(2));
    let mut features = vec![
        "bias".to_owned(),
        format!("a-3={m3}"),
        format!("a-2={m2}"),
        format!("a-1={m1}"),
        format!("a+0={p0}"),
        format!("a+1={p1}"),
        format!("a+2={p2}"),
        format!("b-2={m2}|{m1}"),
        format!("bx={m1}|{p0}"),
        format!("b+1={p0}|{p1}"),
        format!("t-={m2}|{m1}|{p0}"),
        format!("t+={m1}|{p0}|{p1}"),
        format!("q={m2}|{m1}|{p0}|{p1}"),
        format!("pos={}|{}", bucket(gap), bucket(clusters.len().saturating_sub(gap))),
    ];
    let (mut left, mut right) = (0, 0);
    let (mut left_band, mut right_band) = ("0", "0");
    for n in 1..=6 {
        if let Some(start) = gap.checked_sub(n)
            && let Some(parts) = clusters.get(start..gap)
        {
            let f = crate::frequency(&parts.concat());
            if f > 0 {
                left = n;
                left_band = band(f);
                features.extend([format!("L{n}"), format!("L{n}f={left_band}")]);
            }
        }
        if let Some(parts) = clusters.get(gap..gap.saturating_add(n)) {
            let f = crate::frequency(&parts.concat());
            if f > 0 {
                right = n;
                right_band = band(f);
                features.extend([format!("R{n}"), format!("R{n}f={right_band}")]);
            }
        }
    }
    features.extend([
        format!("Lf={left_band}"),
        format!("Rf={right_band}"),
        format!("LRf={left_band}|{right_band}"),
        format!("Lmax={left}"),
        format!("Rmax={right}"),
        format!("LR={left}|{right}"),
    ]);
    if left > 0 && right > 0 {
        features.push("both".to_owned());
    }
    if left == 0 && right == 0 {
        features.push("neither".to_owned());
    }
    let mut straddles = false;
    'search: for l in 1..=6 {
        let Some(start) = gap.checked_sub(l) else { break };
        for r in 1..=6 {
            let Some(parts) = clusters.get(start..gap.saturating_add(r)) else { break };
            let f = crate::frequency(&parts.concat());
            if f > 0 {
                let band = band(f);
                features.extend([format!("X={l}|{r}"), format!("Xf={band}"), format!("Xf={band}|{left_band}|{right_band}")]);
                straddles = true;
                break 'search;
            }
        }
    }
    if straddles {
        features.push("straddled".to_owned());
    }
    if straddles && left > 0 && right > 0 {
        features.push("contested".to_owned());
    }
    features.iter().filter_map(|feature| weights().get(feature.as_str())).sum::<f64>()
}

#[cfg(all(test, craft_khmer_data))]
mod tests {
    #[test]
    fn every_weight_preserves_its_exact_f64_bits() {
        let weights = super::weights();
        assert_eq!(weights.len(), 39_127);
        for line in include_str!(concat!(env!("OUT_DIR"), "/kcc-weights.tsv")).lines() {
            let (feature, number) = line.rsplit_once('\t').unwrap();
            assert_eq!(weights.get(feature).unwrap().to_bits(), number.parse::<f64>().unwrap().to_bits(), "{feature}");
        }
    }
}
