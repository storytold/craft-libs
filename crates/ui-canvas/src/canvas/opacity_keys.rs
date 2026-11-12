//! Photoshop's number keys for a canvas tool's opacity, as the [`OpacityDigit`] command set: 1 to 9 set 10 % to 90 %
//! and 0 sets 100 %, two digits typed quickly set the exact value, and Shift with a digit sets the tool's second value
//! (Flow for a brush, Fill for a layer). The app routes the fired commands to [`OpacityDigit::entry`] with its
//! [`DigitKeys`] while the current tool has an opacity, and sets the value the entry names; which value that is and its
//! undo step stay the app's.

use crate::runtime::commands::CommandSet as _;
use crate::runtime::commands::{FirePhase, Fired};
use craft_behavior::{DigitEntry, DigitKeys};

/// The number keys of a canvas tool's opacity and second value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "toolOpacity")]
#[non_exhaustive]
pub enum OpacityDigit {
    /// 1: the opacity at 10 %, or the second digit of an exact value.
    #[command(label = "Opacity 1", default = "1")]
    OpacityOne,
    /// 2: the opacity at 20 %, or the second digit of an exact value.
    #[command(label = "Opacity 2", default = "2")]
    OpacityTwo,
    /// 3: the opacity at 30 %, or the second digit of an exact value.
    #[command(label = "Opacity 3", default = "3")]
    OpacityThree,
    /// 4: the opacity at 40 %, or the second digit of an exact value.
    #[command(label = "Opacity 4", default = "4")]
    OpacityFour,
    /// 5: the opacity at 50 %, or the second digit of an exact value.
    #[command(label = "Opacity 5", default = "5")]
    OpacityFive,
    /// 6: the opacity at 60 %, or the second digit of an exact value.
    #[command(label = "Opacity 6", default = "6")]
    OpacitySix,
    /// 7: the opacity at 70 %, or the second digit of an exact value.
    #[command(label = "Opacity 7", default = "7")]
    OpacitySeven,
    /// 8: the opacity at 80 %, or the second digit of an exact value.
    #[command(label = "Opacity 8", default = "8")]
    OpacityEight,
    /// 9: the opacity at 90 %, or the second digit of an exact value.
    #[command(label = "Opacity 9", default = "9")]
    OpacityNine,
    /// 0: the opacity at 100 %, or the second digit of an exact value.
    #[command(label = "Opacity 0", default = "0")]
    OpacityZero,
    /// Shift+1: the second value at 10 %, or the second digit of an exact value.
    #[command(label = "Flow or Fill 1", default = "Shift+1")]
    SecondOne,
    /// Shift+2: the second value at 20 %, or the second digit of an exact value.
    #[command(label = "Flow or Fill 2", default = "Shift+2")]
    SecondTwo,
    /// Shift+3: the second value at 30 %, or the second digit of an exact value.
    #[command(label = "Flow or Fill 3", default = "Shift+3")]
    SecondThree,
    /// Shift+4: the second value at 40 %, or the second digit of an exact value.
    #[command(label = "Flow or Fill 4", default = "Shift+4")]
    SecondFour,
    /// Shift+5: the second value at 50 %, or the second digit of an exact value.
    #[command(label = "Flow or Fill 5", default = "Shift+5")]
    SecondFive,
    /// Shift+6: the second value at 60 %, or the second digit of an exact value.
    #[command(label = "Flow or Fill 6", default = "Shift+6")]
    SecondSix,
    /// Shift+7: the second value at 70 %, or the second digit of an exact value.
    #[command(label = "Flow or Fill 7", default = "Shift+7")]
    SecondSeven,
    /// Shift+8: the second value at 80 %, or the second digit of an exact value.
    #[command(label = "Flow or Fill 8", default = "Shift+8")]
    SecondEight,
    /// Shift+9: the second value at 90 %, or the second digit of an exact value.
    #[command(label = "Flow or Fill 9", default = "Shift+9")]
    SecondNine,
    /// Shift+0: the second value at 100 %, or the second digit of an exact value.
    #[command(label = "Flow or Fill 0", default = "Shift+0")]
    SecondZero,
}

impl OpacityDigit {
    /// The digit the command types and whether it is for the second value.
    pub const fn digit(self) -> (u8, bool) {
        match self {
            OpacityDigit::OpacityOne => (1, false),
            OpacityDigit::OpacityTwo => (2, false),
            OpacityDigit::OpacityThree => (3, false),
            OpacityDigit::OpacityFour => (4, false),
            OpacityDigit::OpacityFive => (5, false),
            OpacityDigit::OpacitySix => (6, false),
            OpacityDigit::OpacitySeven => (7, false),
            OpacityDigit::OpacityEight => (8, false),
            OpacityDigit::OpacityNine => (9, false),
            OpacityDigit::OpacityZero => (0, false),
            OpacityDigit::SecondOne => (1, true),
            OpacityDigit::SecondTwo => (2, true),
            OpacityDigit::SecondThree => (3, true),
            OpacityDigit::SecondFour => (4, true),
            OpacityDigit::SecondFive => (5, true),
            OpacityDigit::SecondSix => (6, true),
            OpacityDigit::SecondSeven => (7, true),
            OpacityDigit::SecondEight => (8, true),
            OpacityDigit::SecondNine => (9, true),
            OpacityDigit::SecondZero => (0, true),
        }
    }

    /// What `fired` sets, through the pair state `keys`, at `now` seconds (the pass time): `None` for another command,
    /// a release, or a held key's repeat, which types no second digit.
    pub fn entry(keys: &mut DigitKeys, fired: &Fired, now: f64) -> Option<DigitEntry> {
        if fired.phase != FirePhase::Press {
            return None;
        }
        let (digit, second) = OpacityDigit::from_ref(&fired.command)?.digit();
        keys.press(digit, second, now)
    }
}
