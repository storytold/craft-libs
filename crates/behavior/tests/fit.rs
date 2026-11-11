//! The fit rules: tab widths (`fit`).

use craft_behavior::fit::{TabFit, fit};

fn total(fit: &TabFit) -> f32 {
    fit.shown.iter().map(|&(_, width)| width).sum::<f32>() + fit.overflow_width
}

#[test]
fn overflow_retains_selection_and_partitions_all_tabs() {
    let natural = [80.0, 90.0, 85.0, 95.0];
    for selected in [0, 1, 2, 3, usize::MAX] {
        for available in [0.0, 1.0, 5.0, 18.0, 19.0, 55.0, 120.0, 200.0, 500.0] {
            let f = fit(&natural, selected, available, 40.0, 18.0);
            let selected = selected.min(natural.len() - 1);
            assert!(f.shown.iter().any(|&(i, _)| i == selected), "{selected} at {available}: {f:?}");
            assert!(!f.overflow.contains(&selected));
            assert!(f.shown.windows(2).all(|w| w[0].0 < w[1].0));
            assert!(f.overflow.windows(2).all(|w| w[0] < w[1]));
            let mut all: Vec<usize> = f.shown.iter().map(|&(i, _)| i).chain(f.overflow.iter().copied()).collect();
            all.sort_unstable();
            assert_eq!(all, [0, 1, 2, 3]);
            assert!(total(&f) <= available + 0.001, "{available}: {f:?}");
        }
    }
}
