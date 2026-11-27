use super::*;
use craft_ui::testing::trial;

/// A virama doesn't split a word: the limits count the whole word, and no point falls before a
/// mark or next to a joiner.
#[cfg(feature = "hindi")]
#[trial]
fn indic_word_limits_span_viramas() {
    let hi = Lang::for_language("hi").unwrap();
    // न म स ् क ा र: unrestricted points before म, स and र; only स has two letters on each side.
    assert_eq!(word_breaks_in("नमस्कार", hi).0, vec![3, 6, 18]);
    assert_eq!(hyphen_points_in("नमस्कार", &Limits::default(), hi), vec![6]);
    for w in ["नमस्कार", "हिन्दुस्तानी", "प्रौद्योगिकी", "विश्वविद्यालय", "क्ष\u{200D}त्रिय"]
    {
        for p in word_breaks_in(w, hi).0 {
            let (before, after) = w.split_at(p);
            let (Some(x), Some(y)) = (before.chars().next_back(), after.chars().next()) else { panic!("{w} at {p}") };
            assert!(!is_mark(y) && !is_joiner(x) && !is_joiner(y), "{w} at {p}");
        }
    }
}

/// Slovak's patterns are GPL-2.0-or-later: only their own feature turns them on, never
/// `all-languages`.
#[trial]
fn all_languages_leaves_out_the_gpl_slovak_patterns() {
    let manifest = include_str!("../../Cargo.toml");
    let all = manifest.split("all-languages = [").nth(1).and_then(|s| s.split(']').next()).unwrap_or_default();
    assert!(all.contains("\"spanish\""), "all-languages list not found");
    assert!(!all.contains("slovak"), "{all}");
    assert!(manifest.contains("\nslovak-gpl = [\"hyphen\", \"hypher/slovak\"]"));
}

/// `hyphen_points`' limit checks as first written (a rescan of the letter run per point): the
/// reference for the linear version.
fn hyphen_points_ref(word: &str, lim: &Limits) -> Vec<usize> {
    let chars: Vec<char> = word.chars().collect();
    let (pts, _) = char_breaks(word, Lang::English);
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

#[trial]
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
        "cafe\u{301}-cre\u{300}me",
        "e\u{301}tude",
        "नमस्कार",
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
                    assert_eq!(hyphen_points(w, &lim), byte_offsets(w, hyphen_points_ref(w, &lim)), "{w:?} {lim:?}");
                }
            }
        }
        let c: Vec<char> = w.to_lowercase().chars().collect();
        assert_eq!(heuristic_points(&c), heuristic_ref(&c), "{w:?}");
    }
}
