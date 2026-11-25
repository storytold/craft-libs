//! [`DigitKeys`]: Photoshop's number keys for a tool's opacity. 1 sets 10 % … 9 sets 90 %, 0 sets 100 %; two digits
//! typed within [`DigitKeys::TWO_DIGITS`] seconds set the exact value (4 then 5 is 45 %, 0 then 0 is 0 %). The second
//! value's keys (Shift with a digit) set the tool's second value the same way: Flow for a brush, Fill for a layer.

/// What a number key sets: the percentage and which value.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct DigitEntry {
    /// 0..=100.
    pub percent: f64,
    /// The tool's second value (Flow, Fill) rather than its opacity.
    pub second: bool,
}

/// The number keys' state between presses: the first digit of a pair. The app keeps one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DigitKeys {
    first: Option<(u8, bool, f64)>,
}

impl DigitKeys {
    /// Seconds within which a second digit makes an exact value.
    pub const TWO_DIGITS: f64 = 0.8;

    /// One press of `digit` (0..=9) for the second value or not, at `now` seconds: 10 × the digit (0: 100) on its own,
    /// or the two-digit value when it follows a first digit for the same value in time. `None` for a digit over 9.
    pub fn press(&mut self, digit: u8, second: bool, now: f64) -> Option<DigitEntry> {
        if digit > 9 {
            return None;
        }
        let percent = match self.first.take() {
            Some((first, was_second, at)) if was_second == second && (0.0..Self::TWO_DIGITS).contains(&(now - at)) => f64::from(first * 10 + digit),
            _ => {
                self.first = Some((digit, second, now));
                if digit == 0 { 100.0 } else { f64::from(digit) * 10.0 }
            }
        };
        Some(DigitEntry { percent, second })
    }
}
