//! Libhyphen/Huhyphn replacement patterns; text stays in its source spelling.
use super::{FxMap, MAX_PATTERN_LEN, Patterns, pack};

/// A break before source character `at`. `suffix` is displayed immediately before
/// the hyphen only when this break is selected; it is never added to source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opportunity {
    pub at: usize,
    pub suffix: Box<str>,
}

#[derive(Clone, Debug)]
pub(super) struct Change {
    start: usize,
    cut: usize,
    locus: usize,
    left: Box<str>,
    right: Box<str>,
}

impl Patterns {
    /// Report whether patterns contain display-letter replacement rules.
    pub(crate) fn has_nonstandard(&self) -> bool {
        !self.changes.is_empty()
    }

    /// Estimate heap storage used by long patterns and replacement records.
    pub(crate) fn extra_memory(&self) -> u64 {
        let long: usize = self.long.iter().map(|(k, v)| k.len() + v.len()).sum();
        let changes: usize = self.changes.iter().map(|(k, c)| 96 + k.len() + c.left.len() + c.right.len()).sum();
        (long as u64).saturating_add(changes as u64)
    }

    /// Read UTF-8 Libhyphen patterns, including replacement/start/cut declarations.
    /// Malformed replacement records are logged and skipped, without rewriting data.
    /// At most eight detailed warnings and one summary are emitted per file.
    ///
    /// # Errors
    /// Rejects oversized or unsupported patterns and files with no usable records.
    pub fn from_huhyphn(text: &str) -> Result<Self, String> {
        Self::read_huhyphn(text, |warning| log::warn!("hyphenation patterns: {warning}"))
    }

    /// Read records with bounded diagnostics; an unusable file retains its first error.
    fn read_huhyphn(text: &str, mut warn: impl FnMut(&str)) -> Result<Self, String> {
        let mut out = Self::default();
        let (mut records, mut skipped) = (0usize, 0usize);
        let mut first_error = None;
        for (line_no, line) in text.strip_prefix('\u{feff}').unwrap_or(text).lines().enumerate() {
            let line = line.trim();
            if line.is_empty()
                || line.starts_with(['%', '#'])
                || line == "UTF-8"
                || line.starts_with("COMPOUNDLEFTHYPHENMIN ")
                || line.starts_with("COMPOUNDRIGHTHYPHENMIN ")
                || line.starts_with("LEFTHYPHENMIN ")
                || line.starts_with("RIGHTHYPHENMIN ")
            {
                continue;
            }
            records += 1;
            if records > 250_000 {
                return Err("too many patterns".into());
            }
            if line.len() > 1024 {
                return Err(format!("line {}: pattern too long", line_no + 1));
            }
            let (raw, replacement) = line.split_once('/').map_or((line, None), |(p, r)| (p, Some(r)));
            let change = match replacement.map(|r| parse_change(raw, r)).transpose() {
                Ok(change) => change,
                Err(error) => {
                    let error = format!("line {}: {error}", line_no + 1);
                    skipped += 1;
                    if skipped <= 8 {
                        warn(&error);
                    }
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    continue;
                }
            };
            if !out.insert(raw) {
                return Err(format!("line {}: unsupported pattern", line_no + 1));
            }
            if let Some(change) = change {
                let codes: Vec<u8> = raw.chars().filter(|c| !c.is_ascii_digit()).map(|c| out.alpha.code(c)).collect();
                out.changes.insert(codes.into_boxed_slice(), change);
            }
        }
        if skipped > 8 {
            warn(&format!("{} additional malformed replacement records skipped", skipped - 8));
        }
        if out.is_empty()
            && let Some(error) = first_error
        {
            return Err(error);
        }
        Ok(out)
    }

    /// Breaks including display-only insertions. Replacements that would alter or
    /// delete source letters are conservatively left unbroken by this insertion API.
    pub fn opportunities(&self, word: &[char]) -> Option<Vec<Opportunity>> {
        if word.len() > super::MAX_WORD_CHARS {
            return None;
        }
        if let Some(points) = self.exception(&word.iter().collect::<String>()) {
            return Some(points.iter().map(|&at| Opportunity { at, suffix: "".into() }).collect());
        }
        let mut codes = Vec::with_capacity(word.len() + 2);
        codes.push(1);
        for &c in word {
            let code = self.alpha.code(c);
            if code == 0 {
                return None;
            }
            codes.push(code);
        }
        codes.push(1);
        let mut weights = vec![0u8; codes.len() + 1];
        let mut selected: Vec<Option<(usize, &Change)>> = vec![None; codes.len() + 1];
        // The same left-to-right priority as the ordinary Liang application. Equal
        // weights do not replace a previously selected rule.
        for start in 0..codes.len() {
            for len in 1..=self.max_len.min(codes.len() - start) {
                let key = &codes[start..start + len];
                let values = if len <= MAX_PATTERN_LEN { self.map.get(&pack(key)) } else { self.long.get(key) };
                let Some(values) = values else { continue };
                let change = self.changes.get(key);
                for (local, &weight) in values.iter().enumerate() {
                    let pos = start + local;
                    if let Some(old) = weights.get_mut(pos)
                        && weight > *old
                    {
                        *old = weight;
                        if let Some(slot) = selected.get_mut(pos) {
                            *slot = change.filter(|c| c.locus == local).map(|c| (start, c));
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        for at in 1..word.len() {
            if weights.get(at + 1).is_none_or(|w| w % 2 == 0) {
                continue;
            }
            let suffix = if let Some(Some((start, c))) = selected.get(at + 1) {
                let Some(a) = start.checked_add(c.start).and_then(|x| x.checked_sub(1)) else { continue };
                let Some(b) = a.checked_add(c.cut) else { continue };
                let (Some(old_left), Some(old_right)) = (word.get(a..at), word.get(at..b)) else { continue };
                let left: String = old_left.iter().collect();
                let right: String = old_right.iter().collect();
                let Some(extra) = c.left.strip_prefix(&left).filter(|_| c.right.as_ref() == right) else { continue };
                if extra.chars().count() > 16 {
                    continue;
                }
                extra.into()
            } else {
                "".into()
            };
            out.push(Opportunity { at, suffix });
        }
        Some(out)
    }
}

/// Validate a replacement declaration and locate its winning odd break weight.
fn parse_change(raw: &str, replacement: &str) -> Result<Change, String> {
    let mut args = replacement.split(',');
    let (left, right) = args.next().and_then(|s| s.split_once('=')).ok_or("missing replacement break")?;
    if !left.chars().chain(right.chars()).all(char::is_alphabetic) || left.len() + right.len() > 64 {
        return Err("replacement must contain bounded letters".into());
    }
    let start = args
        .next()
        .ok_or("missing start")?
        .parse::<usize>()
        .map_err(|_| "invalid start")?
        .checked_sub(1)
        .and_then(|x| x.checked_add(usize::from(raw.starts_with('.'))))
        .ok_or("invalid start")?;
    let cut = args.next().ok_or("missing cut")?.parse::<usize>().map_err(|_| "invalid cut")?;
    if args.next().is_some() {
        return Err("unexpected replacement fields".into());
    }
    let end = start.checked_add(cut).ok_or("replacement overflow")?;
    let mut pos = 0usize;
    let mut values = FxMap::default();
    for c in raw.chars() {
        if let Some(w) = c.to_digit(10) {
            values.insert(pos, w as u8);
        } else {
            pos += 1;
        }
    }
    if end > pos || cut == 0 {
        return Err("replacement outside pattern".into());
    }
    let locus = values
        .iter()
        .filter(|(i, w)| start <= **i && **i <= end && **w % 2 == 1)
        .max_by_key(|(i, w)| (**w, std::cmp::Reverse(**i)))
        .map(|(i, _)| *i)
        .ok_or("no odd point in replacement")?;
    Ok(Change { start, cut, locus, left: left.into(), right: right.into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Regression: oversized match inputs are refused before allocating tables.
    #[test]
    fn oversized_match_inputs_are_refused_before_allocating_tables() {
        let p = Patterns::from_huhyphn("UTF-8\na1a\nas5szon2y/sz=,2,1\n").unwrap();
        let word = vec!['a'; super::super::MAX_WORD_CHARS + 1];
        assert!(p.opportunities(&word).is_none());
        assert!(p.points(&word).is_none());
        assert!(p.values(&word).is_none());
    }
    /// Regression: utf8 bom keeps inserted letter rules and line errors.
    #[test]
    fn utf8_bom_keeps_inserted_letter_rules_and_line_errors() {
        let p = Patterns::parse_checked("\u{feff}UTF-8\nas5szon2y/sz=,2,1\n").unwrap();
        assert_eq!(p.opportunities(&"asszony".chars().collect::<Vec<_>>()).unwrap(), vec![Opportunity { at: 2, suffix: "z".into() }]);
        let e = Patterns::parse_checked("\u{feff}UTF-8\nas5szony/sz=,0,1").unwrap_err();
        assert!(e.contains("line 2"), "{e}");
    }
    /// Regression: inserted letters are not ordinary points.
    #[test]
    fn inserted_letters_are_not_ordinary_points() {
        let p = Patterns::from_huhyphn("UTF-8\nas5szon2y/sz=,2,1\n").unwrap();
        let w: Vec<char> = "asszony".chars().collect();
        assert_eq!(p.opportunities(&w).unwrap(), vec![Opportunity { at: 2, suffix: "z".into() }]);
        assert!(p.points(&w).unwrap().is_empty());
    }
    /// Regression: malformed replacements fail without panicking.
    #[test]
    fn malformed_replacements_fail_without_panicking() {
        for rule in [
            "as5szony/sz=,0,1",
            "as5szony/sz=,999999999999999999999,1",
            "as5szony/sz=,3,999999999999999999999",
            "as5szony/sz=,3,1,4",
            "as5szony/sz=1,3,1",
        ] {
            assert!(Patterns::from_huhyphn(rule).is_err(), "{rule}");
        }
    }
    /// Broken upstream records do not disable neighbouring valid rules or change source bytes.
    #[test]
    fn malformed_upstream_records_are_skipped_and_reported() {
        let text = "UTF-8\nas5szon2y/sz=,2,1\n.mas5száz8s3zok1ni/sz=3,1,1,14\ngic5csetek/cs=,3,1 ,1,9\nkön5nyen/ny=,3,1\n";
        let original = text.as_bytes().to_vec();
        let mut warnings = Vec::new();
        let p = Patterns::read_huhyphn(text, |w| warnings.push(w.to_owned())).unwrap();
        assert_eq!(warnings.len(), 2);
        assert!(warnings[0].starts_with("line 3:"));
        assert!(warnings[1].starts_with("line 4:"));
        assert_eq!(p.opportunities(&"asszony".chars().collect::<Vec<_>>()).unwrap(), vec![Opportunity { at: 2, suffix: "z".into() }]);
        assert_eq!(text.as_bytes(), original);
    }

    /// Corrupt files cannot flood the log while the valid record remains available.
    #[test]
    fn malformed_record_warnings_are_bounded() {
        let text = format!("UTF-8\na1a\n{}", "as5szony/sz=,0,1\n".repeat(30));
        let mut warnings = Vec::new();
        let p = Patterns::read_huhyphn(&text, |w| warnings.push(w.to_owned())).unwrap();
        assert_eq!(warnings.len(), 9);
        assert!(warnings[8].contains("22 additional"));
        assert_eq!(p.points(&['a', 'a']).unwrap(), vec![1]);
    }

    /// Regression: a higher even weight suppresses the replacement.
    #[test]
    fn a_higher_even_weight_suppresses_the_replacement() {
        let p = Patterns::from_huhyphn("as5szony/sz=,2,1\nas6s").unwrap();
        assert!(p.opportunities(&"asszony".chars().collect::<Vec<_>>()).unwrap().is_empty());
    }
}
