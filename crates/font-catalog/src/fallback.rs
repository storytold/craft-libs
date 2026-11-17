//! Face choice for a grapheme cluster.

/// Characters requiring their own glyph, after numeral or other caller-supplied mapping.
/// Controls, joiners and variation selectors affect shaping but do not need a separate glyph.
pub fn drawable_characters<'a>(cluster: &'a str, map: impl Fn(char) -> char + Copy + 'a) -> impl Iterator<Item = char> + 'a {
    cluster
        .chars()
        .map(map)
        .filter(|&c| !(c.is_control() || matches!(c, '\u{200C}' | '\u{200D}' | '\u{2028}' | '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}')))
}

/// Select one face for a cluster: prefer the primary if it covers every drawable character;
/// otherwise try each character's fallback until a face covers the whole cluster. If none does,
/// use the first character's fallback when the primary lacks it, or keep the primary.
/// The caller controls the font catalog, ordering and style matching.
pub fn cluster_face<F: Clone>(
    cluster: &str,
    primary: &F,
    map: impl Fn(char) -> char + Copy,
    primary_covers: impl Fn(char) -> bool,
    mut fallback: impl FnMut(char) -> Option<F>,
    covers: impl Fn(&F, char) -> bool,
) -> F {
    let characters = || drawable_characters(cluster, map);
    if characters().all(&primary_covers) {
        return primary.clone();
    }
    for c in characters() {
        if let Some(face) = fallback(c)
            && characters().all(|ch| covers(&face, ch))
        {
            return face;
        }
    }
    if let Some(c) = characters().next()
        && !primary_covers(c)
        && let Some(face) = fallback(c)
    {
        return face;
    }
    primary.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chooses_one_face_for_a_base_and_mark() {
        let fallback = |c| match c {
            'a' => Some(1),
            '\u{301}' => Some(2),
            _ => None,
        };
        let covers = |face: &u8, c| *face == 2 || (*face == 1 && c == 'a');
        assert_eq!(cluster_face("a\u{301}\u{200d}", &0, |c| c, |_| false, fallback, covers), 2);
        assert_eq!(drawable_characters("a\u{200d}\u{fe0f}", |c| c).collect::<String>(), "a");
    }
    #[test]
    fn uses_first_character_fallback_when_no_face_covers_cluster() {
        assert_eq!(cluster_face("ab", &0, |c| c, |_| false, |c| if c == 'a' { Some(1) } else { Some(2) }, |face, c| (*face, c) == (1, 'a')), 1);
    }
}
