//! Unit tests for the affix dictionary.

use super::*;

const AFF: &str = "SET UTF-8\nTRY aeoiułkr\n\nPFX b Y 1\nPFX b   0   nie   .\n\nSFX a Y 3\nSFX a   0   y     [^y]\nSFX a   ek  ka    ek\nSFX a   0   em    .\n\nSFX c N 1\nSFX c   a   ą     a\n\nMAP 2\nMAP aą\nMAP oóu\nREP 2\nREP ż rz\nREP u ó\n";
const DIC: &str = "4\nkot/ab\npiesek/a\nkrowa/c\nWarszawa/c\n";

fn dict() -> AffixDict {
    AffixDict::from_hunspell(AFF, DIC, &["test data"]).unwrap()
}

#[test]
fn affix_headers_allow_comments_but_rule_conditions_stay_literal() {
    let aff = "SET UTF-8\nPFX X Y 1 # prefix comment\nPFX X 0 ne . # rule comment\nSFX y Y 1 # suffix comment\nSFX y 0 y . # rule comment\n";
    let d = AffixDict::from_hunspell(aff, "1\nstrom/Xy\n", &[]).unwrap();
    assert_eq!(d.rule_count(), 2, "headers must not become affix rules");
    for w in ["strom", "stromy", "nestrom", "nestromy"] {
        assert!(d.contains(w), "{w}");
    }
    for w in ["stromY1", "stromN1", "X", "y"] {
        assert!(!d.contains(w), "{w}");
    }
    let hash = AffixDict::from_hunspell("SFX H Y 1\nSFX H 0 # .\nTRY a#b\nKEY a#b\n", "1\nstrom/H\n", &[]).unwrap();
    assert!(hash.contains("strom#"), "a hash in an affix field is literal");
    let literal =
        AffixDict::from_hunspell("PFX P Y 1 # header\nPFX P 0 pre #\nSFX S Y 1 # header\nSFX S 0 post #\n", "3\n#stem/P\nstem#/S\nstem/PS\n", &[])
            .unwrap();
    assert!(literal.contains("pre#stem") && literal.contains("stem#post"));
    assert!(!literal.contains("prestem") && !literal.contains("stempost"), "# must remain a literal condition");

    assert_eq!(hash.try_chars(), "a#b");
    assert_eq!(hash.key(), "a#b");
    assert_eq!(AffixDict::from_bytes(&hash.to_bytes().unwrap()).unwrap().key(), "a#b");
}

#[test]
fn stems_suffixes_prefixes_and_cross_products() {
    let d = dict();
    assert_eq!(d.len(), 4);
    for w in ["kot", "koty", "kotem", "piesek", "pieska", "piesekem", "krowa", "krową", "niekot", "niekoty", "niekotem", "Warszawa", "Warszawą"] {
        assert!(d.contains(w), "{w}");
    }
    for w in ["kotą", "pieseka", "pieskem", "niepiesek", "niekrową", "krowy", "warszawa", "", "nie", "y", "ka"] {
        assert!(!d.contains(w), "{w}");
    }
}

#[test]
fn compact_format_round_trips() {
    let d = dict();
    let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
    assert_eq!(back.len(), d.len());
    assert_eq!(back.rule_count(), d.rule_count());
    assert_eq!(back.comments(), ["test data"]);
    assert_eq!(back.rep(), d.rep());
    assert_eq!(back.map(), d.map());
    assert_eq!(back.try_chars(), "aeoiułkr");
    for w in ["kotem", "niekoty", "pieska", "krową", "Warszawą"] {
        assert!(back.contains(w), "{w}");
    }
    assert!(!back.contains("kotą"));
}

#[test]
fn forbidden_words_are_rejected() {
    // VERO pt_BR marks spellings it must reject (flag ý under FLAG UTF-8) as stems carrying
    // FORBIDDENWORD: listed, but never correct and never inflected.
    let aff = "SET UTF-8\nFLAG UTF-8\nFORBIDDENWORD ý\nSFX a Y 1\nSFX a   0   y   [^y]\n";
    let d = AffixDict::from_hunspell(aff, "2\nkot/a\nimundo/ý\n", &[]).unwrap();
    assert!(d.check("kot") && d.check("koty") && d.check("Kot") && d.check("KOT"));
    assert!(!d.check("imundo") && !d.check("Imundo") && !d.check("IMUNDO"));
    assert!(!d.contains("imundoy"), "a forbidden stem must not inflect");
    assert!(d.forms("imundo").is_empty());
}

#[test]
fn continuation_chains_stop_at_two_suffixes() {
    // Hunspell strips at most two suffixes: work + able/B + s/C is a word, a third (C) is not.
    let aff = "SET UTF-8\nFLAG UTF-8\nSFX A N 1\nSFX A   0   able/B   .\nSFX B N 1\nSFX B   0   s/C   .\nSFX C N 1\nSFX C   0   x   .\n";
    let d = AffixDict::from_hunspell(aff, "1\nwork/A\n", &[]).unwrap();
    assert!(d.contains("workable") && d.contains("workables"));
    assert!(!d.contains("workablesx"), "three chained suffixes exceed the limit");
    assert!(!d.forms("work").iter().any(|f| f == "workablesx"));
}

#[test]
fn flag_aliases_stand_for_flag_sets() {
    // `AF`: `/1` in the `.dic` and in a continuation class means the first alias's flags.
    // Morphological aliases (`AM`) and the fields after a stem are skipped.
    let aff = "SET UTF-8\nAF 2\nAF ab # 1\nAF c # 2\nAM 1\nAM po:noun\nSFX a Y 1\nSFX a 0 ok/2 . 1\nSFX b Y 1\nSFX b 0 at . 1\nSFX c Y 1\nSFX c 0 ban . 1\n";
    let d = AffixDict::from_hunspell(aff, "1\nház/1\t1\n", &[]).unwrap();
    assert!(d.contains("ház") && d.contains("házok") && d.contains("házat") && d.contains("házokban"));
    assert!(!d.contains("házban"), "the stem has a and b, not c");
    // An undefined alias reads as no flags, as in Hunspell: the stem loads, without affixes.
    let undefined = AffixDict::from_hunspell(aff, "2\nház/3\nkert/1\n", &[]).unwrap();
    assert!(undefined.check("ház") && !undefined.check("házok"), "alias 3 is not defined");
    assert!(undefined.check("kertok"), "the other entries keep their aliases");
    let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
    assert!(back.contains("házokban") && !back.contains("házban"), "aliases are expanded in the compact format");
}

/// CHECKSHARPS answers as Hunspell 1.7 gives them on the same files (`*` correct, `&` not).
#[test]
fn checksharps_reads_ss_as_sharp_s_in_capitals() {
    let dic = "6\nstraße\nMaße\nfußball/k\nmesse\nGruß/k\nSchloss\n";
    let words = [
        ("straße", true, true),
        ("Straße", true, true),
        ("STRASSE", false, true),
        ("STRAßE", true, true),
        ("STRAẞE", true, true),
        ("Strasse", false, false),
        ("strasse", false, false),
        ("MASSE", false, true),
        ("MAßE", true, true),
        ("maße", false, false),
        ("FUSSBALL", false, true),
        ("fußball", true, true),
        ("Fußball", false, true),
        ("FUßBALL", false, false),
        ("MESSE", true, true),
        ("MEßE", false, false),
        ("GRUSS", false, true),
        ("Gruß", true, true),
        ("gruß", false, false),
        ("GRUß", false, false),
        ("SCHLOSS", true, true),
        ("SCHLOß", false, false),
    ];
    for (sharps, aff) in [(false, "SET UTF-8\nKEEPCASE k\n"), (true, "SET UTF-8\nCHECKSHARPS\nKEEPCASE k\n")] {
        let d = AffixDict::from_hunspell(aff, dic, &[]).unwrap();
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        for (w, without, with) in words {
            let want = if sharps { with } else { without };
            assert_eq!(d.check(w), want, "{w}, CHECKSHARPS {sharps}");
            assert_eq!(back.check(w), want, "{w}, CHECKSHARPS {sharps}, compact");
        }
    }
    assert_eq!(casing("STRAßE"), Casing::Upper);
    assert_eq!(casing("Maß"), Casing::Initial);
    // Many `SS` pairs: only the first MAX_SHARPS are varied, and a long word costs nothing.
    let d = AffixDict::from_hunspell("CHECKSHARPS\n", "1\naßaßaßaßaß\n", &[]).unwrap();
    assert!(d.check("ASSASSASSASSASS") && !d.check(&"SS".repeat(60)));
}

#[test]
fn conversions_take_the_longest_match_and_the_later_of_equal_ones() {
    let aff = "ICONV 3\nICONV a x\nICONV ab y\nICONV a z\n";
    let d = AffixDict::from_hunspell(aff, "2\ny\nzc\n", &[]).unwrap();
    assert_eq!(d.normalize("ab"), "y");
    assert_eq!(d.normalize("ac"), "zc");
    assert!(d.check("ab") && d.check("ac"));
}

/// craft-libs#483 review: a forbidden word isn't rebuilt from its parts.
#[test]
fn compound_never_rebuilds_a_forbidden_word() {
    let d = AffixDict::from_hunspell("SET UTF-8\nFORBIDDENWORD X\nCOMPOUNDMIN 1\nCOMPOUNDRULE a*\n", "2\nx/a\nxx/X\n", &[]).unwrap();
    assert!(!d.check("xx") && d.check("xxx") && d.check("x"));
    let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
    assert!(!back.check("xx") && back.check("xxx"));
}

/// craft-libs#488 review: NEEDAFFIX, CIRCUMFIX and FULLSTRIP as Hunspell 1.7.5 reads them.
/// Each case is a reviewer's repro `.aff`/`.dic`; the right and wrong words are the `hunspell`
/// CLI's answers on the same files (black-box) for every prefix/stem/suffix combination of
/// them, and `forms()` lists exactly the right ones.
#[test]
fn needaffix_circumfix_and_fullstrip_match_hunspell() {
    const CASES: &[(&str, &str, &str, &str)] = &[
        // rv488-hs/circ
        (
            "SET UTF-8\nCIRCUMFIX c\nPFX P Y 1\nPFX P 0 ne/c .\nSFX S Y 1\nSFX S 0 y/c .\nSFX T Y 1\nSFX T 0 z .\n",
            "1\nstrom/PST\n",
            "nestromy strom stromz",
            "nenestrom nenestromy nenestromyy nenestromyz nenestromz nenestromzy nenestromzz nestrom nestromyy nestromyz nestromz nestromzy nestromzz stromy stromyy stromyz stromzy stromzz",
        ),
        // rv488-hs/circ_chain
        (
            "SET UTF-8\nCIRCUMFIX c\nPFX P Y 1\nPFX P 0 ne/c .\nSFX A Y 1\nSFX A 0 able/Bc .\nSFX B Y 1\nSFX B 0 s .\n",
            "1\nwork/AP\n",
            "neworkable work",
            "nenework neneworkable neneworkableable neneworkables neneworks neneworksable neneworkss nework neworkableable neworks neworksable neworkss workable workableable workables works worksable workss",
        ),
        // rv488-hs/circN
        (
            "SET UTF-8\nCIRCUMFIX c\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 ne/cN .\nSFX S Y 1\nSFX S 0 y/c .\n",
            "1\nstrom/PS\n",
            "nestromy strom",
            "nenestrom nenestromy nenestromyy nestrom nestromyy stromy stromyy",
        ),
        // rv488-hs/circ_nocross
        (
            "SET UTF-8\nCIRCUMFIX c\nPFX P Y 1\nPFX P 0 ne/c .\nSFX S N 1\nSFX S 0 y/c .\n",
            "1\nstrom/PS\n",
            "strom",
            "nenestrom nenestromy nenestromyy nestrom nestromy nestromyy stromy stromyy",
        ),
        // rv488-hs/comp
        ("SET UTF-8\nNEEDAFFIX N\nCOMPOUNDRULE 1\nCOMPOUNDRULE ab\n", "2\nfoo/a\nbar/bN\n", "foo", "bar"),
        // rv488-hs/full
        ("SET UTF-8\nFULLSTRIP\nSFX A Y 1\nSFX A ab x .\nPFX B Y 1\nPFX B ab y .\n", "1\nab/AB\n", "ab x y", ""),
        // rv488-hs/full_pfxsfx
        ("SET UTF-8\nFULLSTRIP\nCIRCUMFIX c\nPFX P Y 1\nPFX P ab y/c .\nSFX S Y 1\nSFX S ab z/c .\n", "1\nab/PS\n", "ab", "y z"),
        // rv488-hs/homo2
        ("SET UTF-8\nNEEDAFFIX N\nSFX A Y 1\nSFX A 0 s .\n", "2\nfoo/NA\nfoo\n", "foo foos", "fooss"),
        // rv488-hs/homo
        ("SET UTF-8\nNEEDAFFIX N\nSFX A Y 1\nSFX A 0 s .\n", "2\nfoo/NA\nfoo/B\n", "foo foos", "fooss"),
        // rv488-hs/pfxna
        (
            "SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re/N .\nSFX S Y 1\nSFX S 0 s/N .\n",
            "1\nwork/PS\n",
            "work",
            "rerework rereworks rereworkss rework reworks reworkss works workss",
        ),
        // rv488-hs/pfxstem
        ("SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re .\n", "1\nbaz/NP\n", "rebaz", "baz rerebaz"),
        // rv488-hs/pN_s
        (
            "SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re/N .\nSFX S Y 1\nSFX S 0 s .\n",
            "1\nwork/PS\n",
            "reworks work works",
            "rerework rereworks rereworkss rework reworkss workss",
        ),
        // rv488-hs/p_sN
        (
            "SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re .\nSFX S Y 1\nSFX S 0 s/N .\n",
            "1\nwork/PS\n",
            "rework reworks work",
            "rerework rereworks rereworkss reworkss works workss",
        ),
        // rv488-hs/sN_chain
        ("SET UTF-8\nNEEDAFFIX N\nSFX B Y 1\nSFX B 0 x/AN .\nSFX A Y 1\nSFX A 0 s .\n", "1\nbar/B\n", "bar barxs", "bars barss barsx barx barxx"),
        // rv488-hs/sNnocross
        (
            "SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re .\nSFX S N 1\nSFX S 0 s/N .\n",
            "1\nwork/PS\n",
            "rework work",
            "rerework rereworks rereworkss reworks reworkss works workss",
        ),
        // rv488-hs/stemN_ps
        (
            "SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re .\nSFX S Y 1\nSFX S 0 s .\n",
            "1\nwork/PSN\n",
            "rework reworks works",
            "rerework rereworks rereworkss reworkss work workss",
        ),
        // rv488-hs/zeroaff
        ("SET UTF-8\nNEEDAFFIX N\nSFX A Y 1\nSFX A 0 0 .\n", "1\nfoo/NA\n", "foo", ""),
        // rv488-second2/t11
        (
            "SET UTF-8\nFULLSTRIP\nSFX A Y 1\nSFX A ab x .\nPFX B Y 1\nPFX B ab y .\nSFX C Y 1\nSFX C 0 z .\n",
            "1\nab/ABC\n",
            "ab abz x y yz",
            "abzz xz yzz",
        ),
        // rv488-second2/t12
        ("SET UTF-8\nFULLSTRIP\nSFX A Y 1\nSFX A x 0 .\nPFX P Y 1\nPFX P 0 q .\n", "1\nx/AP\n", "q qx x", "qq qqx"),
        // rv488-second2/t13
        ("SET UTF-8\nFULLSTRIP\nSFX A Y 1\nSFX A x 0/B .\nSFX B Y 1\nSFX B 0 y .\n", "1\nx/A\n", "x y", "xy xyy"),
        // rv488-second2/t1
        ("SET UTF-8\nNEEDAFFIX N\nSFX A Y 1\nSFX A 0 s .\n", "2\nfoo/N\nfoo/A\n", "foo foos", "fooss"),
        // rv488-second2/t2
        (
            "SET UTF-8\nCIRCUMFIX X\nPFX A Y 1\nPFX A 0 leg/X .\nPFX B Y 1\nPFX B 0 legesleg/X .\nSFX C Y 3\nSFX C 0 obb .\nSFX C 0 obb/AX .\nSFX C 0 obb/BX .\n",
            "1\nnagy/C\n",
            "legeslegnagyobb legnagyobb nagy nagyobb",
            "legesleglegeslegnagy legesleglegeslegnagyobb legesleglegeslegnagyobbobb legesleglegnagy legesleglegnagyobb legesleglegnagyobbobb legeslegnagy legeslegnagyobbobb leglegeslegnagy leglegeslegnagyobb leglegeslegnagyobbobb leglegnagy leglegnagyobb leglegnagyobbobb legnagy legnagyobbobb nagyobbobb",
        ),
        // rv488-second2/t30
        (
            "SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re .\nSFX B Y 1\nSFX B 0 x/N .\n",
            "1\nbar/BP\n",
            "bar rebar rebarx",
            "barx barxx rebarxx rerebar rerebarx rerebarxx",
        ),
        // rv488-second2/t31
        (
            "SET UTF-8\nCIRCUMFIX c\nPFX P Y 1\nPFX P 0 ne .\nPFX Q Y 1\nPFX Q 0 qe/c .\nSFX S Y 1\nSFX S 0 y/c .\nSFX T Y 1\nSFX T 0 z .\n",
            "1\nstrom/PQST\n",
            "nestrom nestromz qestromy strom stromz",
            "nenestrom nenestromy nenestromyy nenestromyz nenestromz nenestromzy nenestromzz neqestrom neqestromy neqestromyy neqestromyz neqestromz neqestromzy neqestromzz nestromy nestromyy nestromyz nestromzy nestromzz qenestrom qenestromy qenestromyy qenestromyz qenestromz qenestromzy qenestromzz qeqestrom qeqestromy qeqestromyy qeqestromyz qeqestromz qeqestromzy qeqestromzz qestrom qestromyy qestromyz qestromz qestromzy qestromzz stromy stromyy stromyz stromzy stromzz",
        ),
        // rv488-second2/t32
        (
            "SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re/N .\nSFX B Y 1\nSFX B 0 x/N .\n",
            "1\nbar/BP\n",
            "bar",
            "barx barxx rebar rebarx rebarxx rerebar rerebarx rerebarxx",
        ),
        // rv488-second2/t3
        (
            "SET UTF-8\nPFX A Y 1\nPFX A 0 leg .\nSFX C Y 1\nSFX C 0 obb/A .\n",
            "1\nnagy/C\n",
            "legnagyobb nagy nagyobb",
            "leglegnagy leglegnagyobb leglegnagyobbobb legnagy legnagyobbobb nagyobbobb",
        ),
        // rv488-second2/t4
        (
            "SET UTF-8\nCIRCUMFIX X\nPFX A Y 1\nPFX A 0 leg/X .\nSFX C Y 2\nSFX C 0 obb .\nSFX C 0 obb/X .\n",
            "1\nnagy/AC\n",
            "legnagyobb nagy nagyobb",
            "leglegnagy leglegnagyobb leglegnagyobbobb legnagy legnagyobbobb nagyobbobb",
        ),
        // rv488-second2/t5
        (
            "SET UTF-8\nNEEDAFFIX N\nPFX A Y 1\nPFX A 0 leg/N .\nSFX C Y 1\nSFX C 0 obb/A .\n",
            "1\nnagy/C\n",
            "legnagyobb nagy nagyobb",
            "leglegnagy leglegnagyobb leglegnagyobbobb legnagy legnagyobbobb nagyobbobb",
        ),
        // rv488-second2/t6
        (
            "SET UTF-8\nNEEDAFFIX N\nPFX A Y 1\nPFX A 0 leg .\nSFX C Y 1\nSFX C 0 obb/AN .\n",
            "1\nnagy/C\n",
            "legnagyobb nagy",
            "leglegnagy leglegnagyobb leglegnagyobbobb legnagy legnagyobbobb nagyobb nagyobbobb",
        ),
        // rv488-second2/t7
        (
            "SET UTF-8\nNEEDAFFIX N\nSFX A Y 1\nSFX A 0 able/B .\nSFX B Y 1\nSFX B 0 s/N .\n",
            "1\nwork/A\n",
            "work workable workables",
            "workableable works worksable workss",
        ),
        // rv488-second2/t8
        (
            "SET UTF-8\nNEEDAFFIX N\nPFX P Y 1\nPFX P 0 re .\nSFX Z Y 1\nSFX Z 0 0 .\nSFX S Y 1\nSFX S 0 s .\n",
            "2\nfoo/NPS\nbar/NZ\n",
            "bar foos refoo refoos",
            "bars barss foo fooss rebar rebars rebarss refooss rerebar rerebars rerebarss rerefoo rerefoos rerefooss",
        ),
        // rv488-second2/t9
        (
            "SET UTF-8\nCIRCUMFIX c\nPFX P Y 1\nPFX P 0 ne/c .\nSFX A Y 1\nSFX A 0 able/B .\nSFX B Y 1\nSFX B 0 s/c .\n",
            "1\nwork/AP\n",
            "work workable workables",
            "nenework neneworkable neneworkableable neneworkables neneworks neneworksable neneworkss nework neworkable neworkableable neworkables neworks neworksable neworkss workableable works worksable workss",
        ),
    ];
    for (aff, dic, right, wrong) in CASES {
        let d = AffixDict::from_hunspell(aff, dic, &[]).unwrap();
        for w in right.split_whitespace() {
            assert!(d.check(w), "{w} is right:\n{aff}{dic}");
        }
        for w in wrong.split_whitespace() {
            assert!(!d.check(w), "{w} is wrong:\n{aff}{dic}");
        }
        let mut forms: Vec<String> = dic.lines().skip(1).flat_map(|l| d.forms(l.split('/').next().unwrap_or(""))).collect();
        forms.sort();
        forms.dedup();
        let mut want: Vec<&str> = right.split_whitespace().collect();
        want.sort_unstable();
        assert_eq!(forms, want, "forms():\n{aff}{dic}");
    }
}

#[test]
fn a_prefix_and_a_suffix_come_from_one_homonym() {
    // `stop/PA` and `stop/S`: `unstop` and `stops` are right, `unstops` wrong (the CLI's
    // answers), also after a round trip through the compact form.
    let aff = "SET UTF-8\nPFX P Y 1\nPFX P 0 un .\nSFX A Y 1\nSFX A 0 a/B .\nSFX B Y 1\nSFX B 0 b .\nSFX S Y 1\nSFX S 0 s .\n";
    let d = AffixDict::from_hunspell(aff, "3\nstop/PA\nstop/S\ngo/PS\n", &[]).unwrap();
    let again = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
    for d in [d, again] {
        assert!(d.check("unstop") && d.check("stops") && d.check("unstopa") && d.check("unstopab") && d.check("ungos"));
        assert!(!d.check("unstops"));
    }
}

/// A small `FLAG num` dictionary (made-up words): prefixes, suffixes, a continuation class,
/// a cross product, the largest flag number, and numeric FORBIDDENWORD and NOSUGGEST flags.
const NUM_AFF: &str = "SET UTF-8\nFLAG num\nFORBIDDENWORD 900\nNOSUGGEST 901\nPFX 7 Y 1\nPFX 7 0 ne .\nSFX 12 Y 2\nSFX 12 0 a .\nSFX 12 0 om/1503 .\nSFX 1503 N 1\nSFX 1503 0 e .\nSFX 65535 N 1\nSFX 65535 0 i .\n";
const NUM_DIC: &str = "5\ngrad/12,7\nmost/65535\nkuca/12,901\ngradd/900\nlist/1503\n";

fn assert_num_dict(d: &AffixDict) {
    for w in ["grad", "grada", "gradom", "gradome", "negrad", "negrada", "negradom", "most", "mosti", "list", "liste", "Grada", "MOSTI"] {
        assert!(d.check(w), "{w}");
    }
    // 1503 reaches `grad` only through `om`; 12 and 65535 belong to other stems; 7 is a prefix.
    for w in ["grade", "gradi", "mosta", "nemost", "lista", "listom", "gradomi", "7grad", "grad12"] {
        assert!(!d.check(w), "{w}");
    }
    assert!(!d.check("gradd") && !d.check("Gradd") && d.forms("gradd").is_empty(), "numeric FORBIDDENWORD");
    assert!(d.check("kuca") && d.check("kucaa") && !d.can_suggest("kuca") && !d.can_suggest("kucaom"), "numeric NOSUGGEST");
    assert!(d.can_suggest("gradome"));
    let mut forms = d.forms("grad");
    forms.sort();
    forms.dedup();
    assert_eq!(forms, ["grad", "grada", "gradom", "gradome", "negrad", "negrada", "negradom"]);
}

#[test]
fn flag_num_dictionaries_round_trip_the_compact_format() {
    let d = AffixDict::from_hunspell(NUM_AFF, NUM_DIC, &["num test"]).unwrap();
    let bytes = d.to_bytes().unwrap();
    let raw = miniz_oxide::inflate::decompress_to_vec(&bytes).unwrap();
    assert!(raw.starts_with(b"WCAFF2\nFLAG num\n"), "the mode is written before any flag");
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("FORBIDDENWORD 900\n") && text.contains("SFX 12 Y 0 om/1503 .\n") && text.contains("grad/7,12\n"));
    let back = AffixDict::from_bytes(&bytes).unwrap();
    assert_num_dict(&back);
    assert_eq!(back.comments(), ["num test"]);
    assert_eq!(back.to_bytes().unwrap(), bytes, "a second round trip writes the same data");
}

#[test]
fn thousands_of_flags_load() {
    // Serbian uses 1,903 numeric flags, Belarusian and Sinhala 5,000, each stem one or two:
    // 3,000 flags here, all appending `a` (one suffix group of 3,000 rules), plus flags up to
    // the largest number.
    let mut aff = String::from("SET UTF-8\nFLAG num\n");
    let mut dic = String::from("3002\n");
    for n in 1..=3000 {
        aff.push_str(&format!("SFX {n} N 1\nSFX {n} 0 a [^q]\n"));
        dic.push_str(&format!("w{n}/{n}\n"));
    }
    aff.push_str("SFX 65000 N 1\nSFX 65000 0 i .\n");
    dic.push_str("dva/3000,65000\nq/1\n");
    let d = AffixDict::from_hunspell(&aff, &dic, &[]).unwrap();
    assert_eq!(d.rule_count(), 3001);
    for w in ["w1a", "w1500a", "w3000a", "dva", "dvaa", "dvai", "q"] {
        assert!(d.check(w), "{w}");
    }
    assert!(!d.check("qa") && !d.check("w1i") && d.check("w1"));
    let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
    assert!(back.check("w2999a") && back.check("dvai") && !back.check("w7i"));

    // More than 128 one-character flags (#4966): 300 letters, each its own suffix.
    let letters: Vec<char> = (0x100u32..0x100 + 300).filter_map(char::from_u32).collect();
    let mut aff = String::from("SET UTF-8\nFLAG UTF-8\n");
    for c in &letters {
        aff.push_str(&format!("SFX {c} N 1\nSFX {c} 0 {c} .\n"));
    }
    let all: String = letters.iter().take(256).collect();
    let d = AffixDict::from_hunspell(&aff, &format!("2\nbase/{all}\nlast/{}\n", letters[299]), &[]).unwrap();
    assert!(d.check(&format!("base{}", letters[0])) && d.check(&format!("base{}", letters[255])));
    assert!(!d.check(&format!("base{}", letters[299])) && d.check(&format!("last{}", letters[299])));
}

proptest::proptest! {
    #[test]
    fn lookups_never_panic(s in "\\PC{0,40}", bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..200)) {
        let d = dict();
        let _ = d.check(&s);
        let _ = d.forms(&s);
        let _ = AffixDict::from_bytes(&bytes);
        let _ = AffixDict::from_bytes(&miniz_oxide::deflate::compress_to_vec(&bytes, 1));
    }

    #[test]
    fn any_flag_field_reads_without_panicking(field in "[0-9,a-zA-Z!\u{fe0f}\u{1f6ab}]{0,24}", word in "\\PC{0,12}") {
        for mode in ["num", "long", "UTF-8"] {
            let aff = format!("FLAG {mode}
FORBIDDENWORD {field}
SFX {field} Y 1
SFX {field} 0 a/{field} .
");
            for aff in [aff.as_str(), &format!("FLAG {mode}
SFX 12 Y 1
SFX 12 0 a .
")] {
                if let Ok(d) = AffixDict::from_hunspell(aff, &format!("2
w/{field}
x/12
"), &[]) {
                    let _ = (d.check(&word), d.check("wa"), d.forms("w"), d.forms("x"));
                    // Data the writer takes, the reader takes back.
                    if let Ok(bytes) = d.to_bytes() {
                        let back = AffixDict::from_bytes(&bytes);
                        proptest::prop_assert!(back.is_ok(), "{mode} {field:?}: {back:?}");
                    }
                }
            }
        }
    }
}
