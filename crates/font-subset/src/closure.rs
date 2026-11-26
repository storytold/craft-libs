//! The shaping closure: characters a shaper may need beyond the text itself.
//!
//! HarfBuzz decomposes some characters before shaping (Thai and Lao SARA AM
//! become two characters), and inserts U+25CC for marks without a base.

use std::collections::BTreeSet;

use unicode_normalization::char::decompose_compatible;

/// Characters every subset carries: space, ZWNJ, ZWJ and the dotted circle.
pub const ALWAYS: [u32; 4] = [0x20, 0x200c, 0x200d, 0x25cc];

/// `codepoints` plus each one's NFKD decomposition plus [`ALWAYS`].
pub fn closure(codepoints: &[u32]) -> BTreeSet<u32> {
    let mut set: BTreeSet<u32> = ALWAYS.into_iter().collect();
    for &cp in codepoints {
        set.insert(cp);
        if let Some(c) = char::from_u32(cp) {
            decompose_compatible(c, |d| {
                set.insert(u32::from(d));
            });
        }
    }
    set
}
