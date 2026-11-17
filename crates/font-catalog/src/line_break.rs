//! Choose a line break after measuring candidate lines as drawn.

/// Choose the latest candidate that fits when a tab changes the line's drawn width.
///
/// `breaks` must be sorted by glyph index and the boolean marks hyphenated breaks.
/// `checks` limits the candidates measured individually from the end; older non-hyphenated
/// candidates are bisected, assuming their drawn widths are nondecreasing. If none fits,
/// the earliest eligible break is returned. Without a tab or a measurement callback the
/// original `chosen` break is returned.
pub fn settle_line_break(
    tabbed: bool,
    checks: usize,
    start: usize,
    chosen: (usize, bool),
    breaks: &[(usize, bool)],
    fits: Option<&dyn Fn(usize, bool) -> bool>,
) -> (usize, bool) {
    let Some(fits) = fits.filter(|_| tabbed) else { return chosen };
    if fits(chosen.0, chosen.1) {
        return chosen;
    }
    let earlier = breaks.get(breaks.partition_point(|b| b.0 <= start)..breaks.partition_point(|b| b.0 < chosen.0)).unwrap_or_default();
    if let Some(&b) = earlier.iter().rev().take(checks).find(|b| fits(b.0, b.1)) {
        return b;
    }
    let rest = earlier.get(..earlier.len().saturating_sub(checks)).unwrap_or_default();
    let plain: Vec<&(usize, bool)> = rest.iter().filter(|b| !b.1).collect();
    let fitting = plain.partition_point(|b| fits(b.0, b.1));
    fitting.checked_sub(1).and_then(|k| plain.get(k).copied()).or(earlier.first()).copied().unwrap_or(chosen)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measures_latest_candidates_and_preserves_hyphenation() {
        let breaks = [(1, false), (2, false), (3, true), (4, false)];
        let fits = |i, hyphen| i <= 2 || (i == 3 && hyphen);
        assert_eq!(settle_line_break(true, 2, 0, (4, false), &breaks, Some(&fits)), (3, true));
        assert_eq!(settle_line_break(false, 2, 0, (4, false), &breaks, Some(&fits)), (4, false));
    }
    #[test]
    fn bisects_older_plain_breaks_and_falls_back_to_first_eligible() {
        let breaks = [(1, false), (2, true), (3, false), (4, false), (5, false)];
        assert_eq!(settle_line_break(true, 1, 0, (5, false), &breaks, Some(&|i, _| i <= 3)), (3, false));
        assert_eq!(settle_line_break(true, 1, 1, (5, false), &breaks, Some(&|_, _| false)), (2, true));
        assert_eq!(settle_line_break(true, 1, 5, (5, false), &breaks, Some(&|_, _| false)), (5, false));
    }
}
