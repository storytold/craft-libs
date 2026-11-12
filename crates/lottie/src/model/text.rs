//! Text layers: the documents, the range animators and the path options.

use super::{Animated, object};

object! {
    /// The `t` of a text layer.
    TextData {
        documents / "d": TextDocs,
        /// Range animators.
        ranges / "a": Vec<TextRange>,
        alignment / "m": TextAlign,
        follow_path / "p": TextFollowPath,
    }
}

object! {
    /// The text over time. It changes in steps: each key holds until the next.
    TextDocs {
        keys / "k": Vec<TextDocKey>,
        expression / "x": String,
    }
}

object! {
    TextDocKey {
        /// The frame.
        time / "t": f64,
        document / "s": TextDocument,
    }
}

object! {
    /// The text and how it is set.
    TextDocument {
        /// The font: the `fName` of an entry of the document's font list.
        font / "f": String,
        size / "s": f64,
        /// `[r, g, b]`, 0..=1.
        fill / "fc": Vec<f64>,
        stroke / "sc": Vec<f64>,
        stroke_width / "sw": f64,
        /// Whether the stroke is drawn over the fill.
        stroke_over_fill / "of": bool,
        line_height / "lh": f64,
        baseline_shift / "ls": f64,
        /// Thousandths of an em.
        tracking / "tr": f64,
        /// 0 left, 1 right, 2 centre, 3 to 6 justified.
        justify / "j": i64,
        /// Lines are separated by `\r`.
        text / "t": String,
        /// The size of the text box.
        box_size / "sz": Vec<f64>,
        box_position / "ps": Vec<f64>,
        /// 0 as typed, 1 all caps, 2 small caps.
        caps / "ca": i64,
    }
}

object! {
    /// A range animator: what changes (`a`) and for which characters (`s`).
    TextRange {
        name / "nm": String,
        selector / "s": TextSelector,
        style / "a": TextStyle,
    }
}

object! {
    TextSelector {
        /// 1 the range selector.
        kind / "t": i64,
        start / "s": Animated,
        end / "e": Animated,
        offset / "o": Animated,
        /// The largest amount, percent.
        amount / "a": Animated,
        /// 1 characters, 2 characters without spaces, 3 words, 4 lines.
        based_on / "b": i64,
        /// 1 percent, 2 index.
        units / "r": i64,
        /// 1 square, 2 ramp up, 3 ramp down, 4 triangle, 5 round, 6 smooth.
        shape / "sh": i64,
        randomize / "rn": i64,
        ease_high / "xe": Animated,
        ease_low / "ne": Animated,
        smoothness / "sm": Animated,
    }
}

object! {
    /// What a range animator changes.
    TextStyle {
        position / "p": Animated,
        anchor / "a": Animated,
        scale / "s": Animated,
        rotation / "r": Animated,
        rotation_x / "rx": Animated,
        rotation_y / "ry": Animated,
        skew / "sk": Animated,
        skew_axis / "sa": Animated,
        opacity / "o": Animated,
        fill / "fc": Animated,
        fill_hue / "fh": Animated,
        fill_saturation / "fs": Animated,
        fill_brightness / "fb": Animated,
        fill_opacity / "fo": Animated,
        stroke / "sc": Animated,
        stroke_width / "sw": Animated,
        stroke_opacity / "so": Animated,
        tracking / "t": Animated,
        line_spacing / "ls": Animated,
        blur / "bl": Animated,
    }
}

object! {
    TextAlign {
        /// What the anchor point is of: 1 characters, 2 words, 3 lines, 4 all.
        grouping / "g": i64,
        /// Percent.
        anchor / "a": Animated,
    }
}

object! {
    /// Text on a mask path.
    TextFollowPath {
        /// The index of the mask.
        mask / "m": i64,
        first_margin / "f": Animated,
        last_margin / "l": Animated,
        reverse / "r": Animated,
        force_alignment / "a": Animated,
        perpendicular / "p": Animated,
    }
}
