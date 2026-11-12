//! The number keys' pair rule.

use craft_behavior::DigitKeys;

#[test]
fn one_digit_sets_tens_and_two_quick_digits_set_the_exact_value() {
    let mut keys = DigitKeys::default();
    let mut press = |digit, second, now| keys.press(digit, second, now).map(|entry| (entry.percent, entry.second));
    assert_eq!(press(4, false, 0.0), Some((40.0, false)));
    assert_eq!(press(5, false, 0.5), Some((45.0, false)), "a second digit in time");
    assert_eq!(press(5, false, 0.6), Some((50.0, false)), "a third digit starts again");
    assert_eq!(press(3, false, 2.0), Some((30.0, false)), "too late for a pair");
    assert_eq!(press(0, true, 2.1), Some((100.0, true)), "a digit for the other value starts again; 0 is 100 %");
    assert_eq!(press(0, true, 2.2), Some((0.0, true)), "0 then 0 is 0 %");
    assert_eq!(press(10, false, 3.0), None);
}
