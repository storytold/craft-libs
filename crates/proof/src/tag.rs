//! Language tags as apps receive them (`w:lang`, `a:rPr/@lang`, `LANG`, `navigator.language`):
//! canonical BCP 47 form and the primary language subtag. Which languages an app proofs, and
//! with which rules, stays in the app.

/// A BCP 47 tag in canonical case (`pl-PL`, `en-GB`, `zh-Hant-TW`, `x-none`), or `None` when
/// `tag` isn't shaped like one: 1–35 ASCII letters, digits and `-`/`_` separators, starting with
/// a 2–8 letter language (or `x`/`i` for private and grandfathered tags). `_` becomes `-`.
pub fn normalize_tag(tag: &str) -> Option<String> {
    let tag = tag.trim();
    if tag.is_empty() || tag.len() > 35 || !tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let parts: Vec<&str> = tag.split(['-', '_']).collect();
    if parts.iter().any(|p| p.is_empty() || p.len() > 8) {
        return None;
    }
    let first = parts.first()?;
    let language = (2..=8).contains(&first.len()) && first.chars().all(|c| c.is_ascii_alphabetic());
    if !language && !first.eq_ignore_ascii_case("x") && !first.eq_ignore_ascii_case("i") {
        return None;
    }
    let letters = |p: &str| p.chars().all(|c| c.is_ascii_alphabetic());
    // After a singleton (`x-…` private use, `u-…` extensions) subtags are lower case.
    let singleton = parts.iter().position(|p| p.len() == 1).unwrap_or(usize::MAX);
    let out: Vec<String> = parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if i >= singleton {
                p.to_ascii_lowercase()
            } else if i > 0 && p.len() == 2 && letters(p) {
                p.to_ascii_uppercase() // region
            } else if i > 0 && p.len() == 4 && letters(p) {
                let lower = p.to_ascii_lowercase();
                let mut c = lower.chars();
                c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default() // script
            } else {
                p.to_ascii_lowercase()
            }
        })
        .collect();
    Some(out.join("-"))
}

/// The primary language subtag of a tag or POSIX locale, lower case (`pl` for `pl-PL`,
/// `pl_PL.UTF-8` and `PL`); `None` when there is none.
pub fn primary_subtag(tag: &str) -> Option<String> {
    let primary = tag.trim().split(['-', '_', '.', '@']).next().unwrap_or("");
    (!primary.is_empty() && primary.chars().all(|c| c.is_ascii_alphabetic())).then(|| primary.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_normalize() {
        assert_eq!(normalize_tag("PL-pl").as_deref(), Some("pl-PL"));
        assert_eq!(normalize_tag(" pl_pl ").as_deref(), Some("pl-PL"));
        assert_eq!(normalize_tag("pl").as_deref(), Some("pl"));
        assert_eq!(normalize_tag("zh-hant-tw").as_deref(), Some("zh-Hant-TW"));
        assert_eq!(normalize_tag("es-419").as_deref(), Some("es-419"));
        assert_eq!(normalize_tag("x-none").as_deref(), Some("x-none"));
        assert_eq!(normalize_tag("de-DE-u-co-PHONEBK").as_deref(), Some("de-DE-u-co-phonebk"));
        for bad in ["", " ", "p", "1pl", "pl--PL", "pl PL", "pl-PL;", "polski język", "-pl", "pl-", "abcdefghi", &"a".repeat(40)] {
            assert_eq!(normalize_tag(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn primary_subtags() {
        for t in ["pl", "pl-PL", "PL_pl", "pl_PL.UTF-8", " pl-PL ", "pl_PL@euro"] {
            assert_eq!(primary_subtag(t).as_deref(), Some("pl"), "{t}");
        }
        assert_eq!(primary_subtag("zh-Hant-TW").as_deref(), Some("zh"));
        for t in ["", " ", "-pl", "1pl"] {
            assert_eq!(primary_subtag(t), None, "{t:?}");
        }
    }
}
