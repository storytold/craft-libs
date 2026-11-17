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
            let f = fit(&natural, selected, available, 40.0, 18.0, false);
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

#[test]
fn kept_selection_keeps_its_width_and_overflow_starts_at_the_end() {
    let natural = [80.0, 90.0, 85.0, 95.0, 70.0];
    for selected in 0..natural.len() {
        for available in [0.0, 30.0, 60.0, 120.0, 160.0, 200.0, 260.0, 300.0, 420.0, 500.0] {
            let f = fit(&natural, selected, available, 40.0, 18.0, true);
            assert!(f.shown.iter().any(|&(i, _)| i == selected), "{selected} at {available}: {f:?}");
            assert!(total(&f) <= available + 0.001, "{available}: {f:?}");
            let kept = f.shown.iter().find(|&&(i, _)| i == selected).map(|&(_, w)| w).unwrap_or(0.0);
            if natural[selected] + f.overflow_width <= available {
                assert!((kept - natural[selected]).abs() < 0.001, "the shown tab keeps its width: {f:?}");
            }
            for &(i, width) in &f.shown {
                if i != selected {
                    assert!(width >= natural[i].min(40.0) - 0.001, "others keep the minimum: {f:?}");
                }
            }
            let others_hidden: Vec<usize> = f.overflow.clone();
            let others: Vec<usize> = (0..natural.len()).filter(|&i| i != selected).collect();
            let first_hidden = others.iter().position(|i| others_hidden.contains(i));
            if let Some(at) = first_hidden {
                assert!(others[at..].iter().all(|i| others_hidden.contains(i)), "overflow takes the last tabs: {f:?}");
                assert!(f.overflow_width > 0.0 || available == 0.0, "the button is reserved: {f:?}");
            } else {
                assert_eq!(f.overflow_width, 0.0);
            }
        }
    }
}
