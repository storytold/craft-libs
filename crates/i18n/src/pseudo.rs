//! The pseudo locales: debug-only languages in which every looked-up string comes back marked, so text drawn without the
//! marks never went through translation.
//!
//! - `pseudo`: `⟦text⟧`, each line marked on its own, since multi-line labels are often drawn a line at a time.
//! - `pseudo-long`: the same marks around text with its letters accented and about 40% more characters, so a layout that only
//!   just fits English is found before a translation finds it. `{name}` placeholders are left as they are, so the filling that
//!   follows still works.

/// How a pseudo locale marks a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Pseudo {
    /// `pseudo`.
    Marked,
    /// `pseudo-long`.
    Long,
}

impl Pseudo {
    /// `text` as this pseudo locale shows it.
    pub(crate) fn show(self, text: &str) -> String {
        match self {
            Pseudo::Marked => marked(text),
            Pseudo::Long => long(text),
        }
    }
}

/// `⟦text⟧`, each line marked on its own.
pub(crate) fn marked(text: &str) -> String {
    format!("⟦{}⟧", text.replace('\n', "⟧\n⟦"))
}

/// `⟦text⟧` with the letters accented and the line padded with `·` to a length 40% over the original (counting what is outside
/// `{…}` placeholders).
fn long(text: &str) -> String {
    text.split('\n').map(long_line).collect::<Vec<_>>().join("\n")
}

fn long_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len() * 2 + 8);
    out.push('⟦');
    let mut counted = 0_usize;
    let mut in_placeholder = false;
    for c in line.chars() {
        match c {
            '{' => in_placeholder = true,
            '}' => in_placeholder = false,
            _ => {}
        }
        if in_placeholder || c == '}' {
            out.push(c);
        } else {
            counted += 1;
            out.push(accented(c));
        }
    }
    // ceil(0.4 * counted)
    out.extend(std::iter::repeat_n('·', (counted * 2).div_ceil(5)));
    out.push('⟧');
    out
}

/// `c` with an accent, for the ASCII letters; every other character as it is.
fn accented(c: char) -> char {
    const LOWER: &str = "áƀçďéƒğĥíĵķĺḿñóṕǫŕšţúṽŵẍýž";
    const UPPER: &str = "ÁƁÇĎÉƑĞĤÍĴĶĹḾÑÓṔǪŔŠŢÚṼŴẌÝŽ";
    let table = if c.is_ascii_lowercase() {
        LOWER
    } else if c.is_ascii_uppercase() {
        UPPER
    } else {
        return c;
    };
    let index = usize::from(c.to_ascii_lowercase() as u8 - b'a');
    table.chars().nth(index).unwrap_or(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every ASCII letter has an accented form, in both cases, and no other character changes.
    #[test]
    fn every_letter_is_accented_and_nothing_else() {
        for c in 'a'..='z' {
            assert_ne!(accented(c), c, "{c}");
            assert_ne!(accented(c.to_ascii_uppercase()), c.to_ascii_uppercase(), "{c}");
        }
        assert_eq!(('a'..='z').map(accented).collect::<std::collections::HashSet<_>>().len(), 26, "26 different accented letters");
        for c in ['0', ' ', '…', '{', 'é', '中'] {
            assert_eq!(accented(c), c);
        }
    }
}
