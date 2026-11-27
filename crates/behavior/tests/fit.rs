//! The fit rules: tab widths (`fit`) and strip items (`fit_items`).

use craft_behavior::fit::{FitItem, FitKeep, TabFit, fit, fit_items};

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

/// Guards the shortening stage of `fit_items`: short forms go last first, before any free item leaves, a leaving item
/// is not reported short, and items without a short form fit exactly as before (the menu bar, bar items and
/// breadcrumbs build no short forms).
#[test]
fn short_items_shorten_from_the_end_before_any_item_leaves() {
    let item = |extent| FitItem::new(extent, 0, FitKeep::Free);
    let shortening = [item(50.0).short(20.0), item(50.0), item(50.0).short(20.0)];
    let plain = [item(50.0), item(50.0), item(50.0)];
    let at = |items: &[FitItem], available| fit_items(items, available, 0.0, 0.0, 10.0);

    let wide = at(&shortening, 150.0);
    assert!(wide.short.is_empty() && wide.overflow.is_empty(), "a run that fits keeps every full form: {wide:?}");

    let narrower = at(&shortening, 120.0);
    assert!(narrower.overflow.is_empty(), "shortening comes before any item leaves: {narrower:?}");
    assert!(narrower.short.contains(&2) && !narrower.short.contains(&0), "the last short item goes first: {narrower:?}");

    let narrow = at(&shortening, 90.0);
    assert!(narrow.overflow.is_empty(), "every short form is taken before an item leaves: {narrow:?}");
    assert!(narrow.short.contains(&0) && narrow.short.contains(&2), "then the earlier ones: {narrow:?}");

    let narrowest = at(&shortening, 80.0);
    assert!(narrowest.overflow.contains(&2) && narrowest.shown.contains(&0), "then the last free item leaves: {narrowest:?}");
    assert!(narrowest.short.iter().all(|i| narrowest.shown.contains(i)), "a leaving item is not short: {narrowest:?}");

    for available in [0.0, 40.0, 80.0, 90.0, 120.0, 150.0] {
        let fit = at(&plain, available);
        assert!(fit.short.is_empty(), "no short form, no short item: {fit:?}");
        let shown = shortening.len() - fit.overflow.len();
        assert_eq!(fit.shown, (0..shown).collect::<Vec<_>>(), "items without short forms leave from the end: {fit:?}");
    }
    assert!(!at(&plain, 120.0).overflow.is_empty(), "without short forms the narrower run overflows");

    let pinned = [FitItem::new(80.0, 0, FitKeep::Pinned).short(30.0), FitItem::new(90.0, 0, FitKeep::Pinned).short(30.0)];
    let squeezed = fit_items(&pinned, 40.0, 0.0, 0.0, 0.0);
    assert!(squeezed.overflow.is_empty(), "pinned items never leave: {squeezed:?}");
    assert_eq!(squeezed.short.len(), 2, "pinned items shorten as free ones do: {squeezed:?}");
}

/// Guards the host's order of short forms: a lower `short_order` shortens first wherever its item sits, and a
/// `short_late` item keeps its full form until every free item has gone to the menu, then shortens in rank order.
#[test]
fn short_forms_follow_their_rank_and_late_ones_wait_for_the_free_items() {
    let item = |extent| FitItem::new(extent, 0, FitKeep::Free);
    let ranked = [item(50.0).short(20.0), item(50.0), item(50.0).short(20.0).short_order(1)];
    let fit = fit_items(&ranked, 120.0, 0.0, 0.0, 10.0);
    assert_eq!(fit.short, [0], "the lower rank shortens before the later item: {fit:?}");
    assert!(fit.overflow.is_empty(), "{fit:?}");

    // Two free items in one section, then two pinned ones that shorten only late, the first ranked before the second.
    let pinned = |extent| FitItem::new(extent, 1, FitKeep::Pinned).short(20.0).short_late();
    let bar = [item(40.0), item(40.0), pinned(60.0), pinned(60.0).short_order(1)];
    let at = |available| fit_items(&bar, available, 0.0, 0.0, 10.0);

    let roomy = at(180.0);
    assert_eq!(roomy.overflow, [1], "a free item leaves before any late item shortens: {roomy:?}");
    assert!(roomy.short.is_empty(), "{roomy:?}");

    let tight = at(130.0);
    assert_eq!(tight.overflow, [0, 1], "every free item leaves first: {tight:?}");
    assert!(tight.short.is_empty(), "{tight:?}");

    let narrow = at(100.0);
    assert_eq!(narrow.short, [2], "then the lower-ranked late item shortens: {narrow:?}");
    assert_eq!(narrow.overflow, [0, 1], "items that left stay in the menu: {narrow:?}");

    let narrowest = at(50.0);
    assert_eq!(narrowest.short, [2, 3], "then the next rank: {narrowest:?}");
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
