//! Property tests for the rounding and saturation guarantees in `craft-units`.

use craft_units::{EMU_PER_TWIP, Length};
use proptest::prelude::*;

/// A range wide enough to exercise real documents without pushing the `i64` saturation limits.
fn document_emu() -> impl Strategy<Value = i64> {
    -1_000_000_000_000_000_i64..=1_000_000_000_000_000_i64
}

proptest! {
    /// Converting to twips and back loses at most half a twip.
    #[test]
    fn twips_round_trip_is_within_half_a_twip(emu in document_emu()) {
        let original = Length::from_emu(emu);
        let restored = Length::from_twips(original.twips()).emu();
        let difference = i128::from(restored) - i128::from(emu);
        prop_assert!(
            difference.abs() <= i128::from(EMU_PER_TWIP / 2 + 1),
            "{emu} -> {restored} (difference {difference})"
        );
    }

    /// Rounding never inverts an order: a smaller length never rounds to more twips.
    #[test]
    fn twips_rounding_is_monotonic(left in document_emu(), right in document_emu()) {
        let (low, high) = if left <= right { (left, right) } else { (right, left) };
        prop_assert!(
            Length::from_emu(low).twips() <= Length::from_emu(high).twips(),
            "{low} <= {high}"
        );
    }

    /// Every unit conversion that lands on a whole unit returns exactly that unit.
    #[test]
    fn whole_unit_conversions_are_exact(units in -1_000_000_i64..=1_000_000_i64) {
        prop_assert_eq!(Length::from_twips(units).twips(), units);
        let points = units as f64;
        prop_assert_eq!(Length::from_points(points).points(), points);
        let inches = units as f64 / 100.0;
        prop_assert!((Length::from_inches(inches).inches() - inches).abs() < 1.0e-9);
    }
}
