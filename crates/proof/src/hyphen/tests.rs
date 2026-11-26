use super::*;

/// An exception word of a pack keeps its breaks even with a letter the patterns never mention.
#[test]
fn pack_exception_words_need_no_pattern_letters() {
    let chars = |w: &str| w.chars().collect::<Vec<_>>();
    let p = Patterns::parse("1на 1ка ё-ка");
    let w = chars("ёка");
    assert!(pack_patterns_fit(ProofLang::Ru, &p, &w));
    assert_eq!(core_breaks_patterns(&p, &w), (vec![1], Source::Dictionary));
    assert!(!pack_patterns_fit(ProofLang::Ru, &p, &chars("ёна")));
}

/// CodeRabbit on craft-libs#484: a Turkish word with a suffix after an apostrophe is
/// hyphenated by the Turkish rules, part by part, never next to the apostrophe.
#[test]
fn turkish_apostrophe_suffixes_follow_turkish_rules() {
    let tr = |w: &str| hyphenate_word_in(w, &Limits::default(), ProofLang::Tr);
    assert_eq!(tr("Ankara'dan"), "An-ka-ra'dan");
    assert_eq!(tr("Türkiye'nin"), "Tür-ki-ye'nin");
    assert_eq!(tr("İstanbul'daki"), "İs-tan-bul'da-ki");
    assert_eq!(word_breaks_in("Ankara'dan", ProofLang::Tr).1, Source::Rules);
    assert_eq!(tr("Ankara"), "An-ka-ra", "no apostrophe: as before");
}

fn h(w: &str) -> String {
    hyphenate_word(w, &Limits::default())
}

#[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
#[test]
fn case_apostrophes_and_punctuation() {
    assert_eq!(h("Typography"), "Ty-pog-ra-phy");
    assert_eq!(h("TYPOGRAPHY"), "TYPOGRAPHY");
    assert_eq!(h("typography,"), "ty-pog-ra-phy,");
    assert_eq!(h("“typography”"), "“ty-pog-ra-phy”");
    assert_eq!(h("typography's"), "ty-pog-ra-phy's");
    assert_eq!(h("typography’s"), "ty-pog-ra-phy’s");
    assert_eq!(h("'typography'"), "'ty-pog-ra-phy'");
    // Compounds: each part separately, never next to the hyphen.
    assert_eq!(h("typography-composition"), "ty-pog-ra-phy-com-po-si-tion");
}

/// `hyphen_points`' limit checks as first written (a rescan of the letter run per point): the
/// reference for the linear version.
fn hyphen_points_ref(word: &str, lim: &Limits) -> Vec<usize> {
    let chars: Vec<char> = word.chars().collect();
    let (pts, _) = word_breaks(word);
    let min_before = lim.after_first.max(1);
    let min_after = lim.before_last.max(1);
    let run_of = |p: usize| {
        let mut a = p;
        while a > 0 && is_word_char(chars[a - 1]) {
            a -= 1;
        }
        let mut b = p;
        while b < chars.len() && is_word_char(chars[b]) {
            b += 1;
        }
        while a < b && is_apostrophe(chars[a]) {
            a += 1;
        }
        while b > a && is_apostrophe(chars[b - 1]) {
            b -= 1;
        }
        (a, b)
    };
    pts.into_iter()
        .filter(|&p| {
            let (a, b) = run_of(p);
            let run = &chars[a..b];
            let letters = run.iter().filter(|c| c.is_alphabetic()).count();
            if letters < lim.min_word.max(2) {
                return false;
            }
            if letters > 1 && run.iter().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase()) {
                return false;
            }
            if !lim.capitalized && run[0].is_uppercase() {
                return false;
            }
            let before = chars[a..p].iter().filter(|c| c.is_alphabetic()).count();
            let after = chars[p..b].iter().filter(|c| c.is_alphabetic()).count();
            before >= min_before && after >= min_after
        })
        .collect()
}

/// `heuristic_points` as first written (the right part rebuilt per point): the reference.
fn heuristic_ref(c: &[char]) -> Vec<usize> {
    let n = c.len();
    if n < 4 || !c.iter().all(|c| c.is_alphabetic()) {
        return vec![];
    }
    let v: Vec<bool> = c.iter().map(|&x| is_vowel(x)).collect();
    let mut out = vec![];
    for i in 1..n {
        let ok = if v[i - 1] && !v[i] {
            (i + 1 < n && v[i + 1]) || (i + 2 < n && onset_pair(c[i], c[i + 1]) && v[i + 2])
        } else if !v[i - 1] && !v[i] {
            i >= 2 && v[i - 2] && i + 1 < n && v[i + 1] && !onset_pair(c[i - 1], c[i]) && c[i - 1] != c[i] || (c[i - 1] == c[i] && i >= 2 && v[i - 2])
        } else {
            false
        };
        let right: String = c[i..].iter().collect();
        let core = right.strip_suffix("ed").or_else(|| right.strip_suffix("es")).or_else(|| right.strip_suffix('e')).unwrap_or(&right);
        if ok && core.chars().any(is_vowel) && core.len() > 1 && v[..i].iter().any(|x| *x) {
            out.push(i);
        }
    }
    out
}

#[test]
fn linear_limits_and_heuristic_match_the_reference() {
    let long = "hyphenation".repeat(40);
    let words = [
        "",
        "x",
        "''",
        "a'b'c",
        "cat",
        "happen",
        "hyphenation",
        "Typography's",
        "TYPOGRAPHY",
        "Typography",
        "typography’s",
        "'typography'",
        "“typography”,",
        "typography-composition",
        "don't",
        "rock'n'roll",
        "naïveté",
        "crème-brûlée",
        "Ünïcödé",
        "ÉTUDE",
        "strengths",
        "ee",
        "bed",
        "hopped",
        "rhythmed",
        "aeiouy",
        "xxyyzz",
        "bookkeeper",
        "ʼtwas",
        "mid-1990s",
        "twenty-one",
        "école",
        "Ålesund",
        "smørrebrød",
        "hopes",
        "hoped",
        "taed",
        "teeed",
        &long,
    ];
    for w in words {
        for min_word in [2, 5, 7] {
            for (after_first, before_last) in [(1, 1), (2, 2), (3, 2), (2, 4)] {
                for capitalized in [true, false] {
                    let lim = Limits { min_word, after_first, before_last, capitalized };
                    assert_eq!(hyphen_points(w, &lim), hyphen_points_ref(w, &lim), "{w:?} {lim:?}");
                }
            }
        }
        let c: Vec<char> = w.to_lowercase().chars().collect();
        assert_eq!(heuristic_points(&c), heuristic_ref(&c), "{w:?}");
    }
}

fn pl(w: &str) -> String {
    hyphenate_word_in(w, &Limits::default(), ProofLang::Pl)
}

#[cfg_attr(not(craft_proof_embed = "pl"), ignore = "needs the pl Language data (CRAFT_I18N_DIR; not in a CRAFT_PROOF_EMBED=en build)")]
#[test]
fn polish_heuristic_for_foreign_letters() {
    // Letters outside the Polish alphabet: the Polish syllable heuristic, digraphs kept whole.
    assert_eq!(word_breaks_in("Pöppelmann", ProofLang::Pl).1, Source::Heuristic);
    assert_eq!(pl("Pöppelmann"), "Pöp-pel-mann");
    assert_eq!(pl("Müllerowi"), "Mül-le-ro-wi");
    let c: Vec<char> = "kawiarnié".chars().collect();
    assert_eq!(heuristic_points_pl(&c), vec![2, 6], "ka-wiar-nié: `wi` and `ni` soften, `rn` splits after the sonorant");
    let c: Vec<char> = "szczérze".chars().collect();
    assert_eq!(heuristic_points_pl(&c), vec![5], "szczé-rze: `rz` stays whole");
}
