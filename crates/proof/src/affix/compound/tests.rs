//! Compound tests pinned to the `hunspell` CLI.

use std::collections::HashSet;

use crate::affix::AffixDict;

/// A synthetic dictionary and the `hunspell` 1.7.5 CLI's answers on it (black-box: `hunspell
/// -a`), for every word of one to three `pieces` and the `extra` words. `listed` are the
/// right words when `right`, else the wrong ones; every other word is the opposite.
struct Pinned {
    name: &'static str,
    aff: &'static str,
    dic: &'static str,
    pieces: &'static str,
    extra: &'static str,
    right: bool,
    listed: &'static str,
}

fn load(aff: &str, dic: &str) -> AffixDict {
    let entries: Vec<&str> = dic.split_whitespace().collect();
    let dic = format!("{}\n{}\n", entries.len(), entries.join("\n"));
    AffixDict::from_hunspell(&format!("SET UTF-8\n{aff}\n"), &dic, &[]).unwrap()
}

fn candidates(p: &Pinned) -> Vec<String> {
    let pieces: Vec<&str> = p.pieces.split_whitespace().collect();
    let mut out: Vec<String> = pieces.iter().map(|a| (*a).to_string()).collect();
    for a in &pieces {
        for b in &pieces {
            out.push(format!("{a}{b}"));
        }
    }
    for a in &pieces {
        for b in &pieces {
            for c in &pieces {
                out.push(format!("{a}{b}{c}"));
            }
        }
    }
    out.extend(p.extra.split_whitespace().map(str::to_string));
    out
}

/// One case per directive (or pair that only means something together), each word's answer
/// pinned to the CLI's, before and after the compact format.
#[test]
fn flag_compounds_answer_as_the_cli_does() {
    let mut failed = Vec::new();
    for p in PINNED {
        let d = load(p.aff, p.dic);
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        let listed: HashSet<&str> = p.listed.split_whitespace().collect();
        for w in candidates(p) {
            let want = listed.contains(w.as_str()) == p.right;
            if d.check(&w) != want || back.check(&w) != want {
                failed.push(format!("{}: {w} should be {}", p.name, if want { "right" } else { "wrong" }));
            }
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

/// `COMPOUNDRULE` as the Swedish (`)k`) and Mongolian (`[a0,a1]`, `%?`, `.`) dictionaries
/// write it, read as the CLI answers on the same rules.
#[test]
fn compound_rules_with_odd_marks_answer_as_the_cli_does() {
    let d = load("COMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE )k", "a/) b/k");
    let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
    for d in [&d, &back] {
        assert!(d.check("ab") && !d.check("abb") && !d.check("aab"));
    }
    let d = load("FLAG long\nCOMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE (aa)*[a0,a1](bb)", "x/aa y/bb");
    assert!(d.check("xy") && d.check("xxy") && !d.check("xyy") && !d.check("yx"));
    for rule in ["(nn)*%?", "(nn)*.(nn)*%?", "(aa)*%?(bb)"] {
        let d = load(&format!("FLAG long\nCOMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE {rule}"), "x/nn y/nn z/aa w/bb");
        let back = AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap();
        for d in [&d, &back] {
            assert!(!d.check("xy") && !d.check("xxy") && !d.check("zw") && d.check("x"), "{rule}");
        }
    }
}

const PINNED: &[Pinned] = &[
    Pinned {
        name: "flag",
        aff: "COMPOUNDFLAG X",
        dic: "foo/X bar/X ba/X baz",
        pieces: "foo bar ba baz",
        extra: "",
        right: true,
        listed: "foo bar ba baz foofoo foobar barfoo barbar foofoofoo foofoobar foobarfoo foobarbar barfoofoo barfoobar \
            barbarfoo barbarbar",
    },
    Pinned { name: "min", aff: "COMPOUNDFLAG X\nCOMPOUNDMIN 1", dic: "f/X bar/X", pieces: "f bar", extra: "", right: false, listed: "" },
    Pinned {
        name: "affixes",
        aff: "COMPOUNDFLAG X\nCOMPOUNDPERMITFLAG Z\nSFX S Y 1\nSFX S 0 s .\nPFX P Y 1\nPFX P 0 un .\nSFX T Y 1\nSFX T 0 t/Z .\nPFX Q Y 1\nPFX Q 0 re/Z .",
        dic: "foo/XSPTQ bar/XSPTQ baz/S",
        pieces: "foo foos unfoo foot refoo bar",
        extra: "barunfoo unfoobar barfoot footbar refoobar barrefoo barfoos foosbar baz",
        right: false,
        listed: "foounfoo foosfoo foosfoos foosunfoo foosfoot foosrefoo foosbar unfoounfoo footunfoo refoounfoo barunfoo \
            foofoounfoo foofoosfoo foofoosfoos foofoosunfoo foofoosfoot foofoosrefoo foofoosbar foounfoounfoo foofootunfoo \
            foorefoounfoo foobarunfoo foosfoofoo foosfoofoos foosfoounfoo foosfoofoot foosfoorefoo foosfoobar foosfoosfoo \
            foosfoosfoos foosfoosunfoo foosfoosfoot foosfoosrefoo foosfoosbar foosunfoofoo foosunfoofoos foosunfoounfoo \
            foosunfoofoot foosunfoorefoo foosunfoobar foosfootfoo foosfootfoos foosfootunfoo foosfootfoot foosfootrefoo \
            foosfootbar foosrefoofoo foosrefoofoos foosrefoounfoo foosrefoofoot foosrefoorefoo foosrefoobar foosbarfoo \
            foosbarfoos foosbarunfoo foosbarfoot foosbarrefoo foosbarbar unfoofoounfoo unfoofoosfoo unfoofoosfoos \
            unfoofoosunfoo unfoofoosfoot unfoofoosrefoo unfoofoosbar unfoounfoounfoo unfoofootunfoo unfoorefoounfoo \
            unfoobarunfoo footfoounfoo footfoosfoo footfoosfoos footfoosunfoo footfoosfoot footfoosrefoo footfoosbar \
            footunfoounfoo footfootunfoo footrefoounfoo footbarunfoo refoofoounfoo refoofoosfoo refoofoosfoos \
            refoofoosunfoo refoofoosfoot refoofoosrefoo refoofoosbar refoounfoounfoo refoofootunfoo refoorefoounfoo \
            refoobarunfoo barfoounfoo barfoosfoo barfoosfoos barfoosunfoo barfoosfoot barfoosrefoo barfoosbar \
            barunfoounfoo barfootunfoo barrefoounfoo barbarunfoo",
    },
    Pinned {
        name: "affix_flags",
        aff: "COMPOUNDFLAG X\nSFX A Y 1\nSFX A 0 s/X .\nPFX B Y 1\nPFX B 0 un/X .\nSFX C Y 1\nSFX C 0 en .",
        dic: "arbeit/AC comp/XC haus/B",
        pieces: "arbeit arbeits comp compen haus unhaus",
        extra: "",
        right: true,
        listed: "arbeit arbeits comp compen haus unhaus comparbeits compcomp compcompen unhausarbeits unhauscomp unhauscompen \
            compcomparbeits compcompcomp compcompcompen compunhausarbeits compunhauscomp compunhauscompen \
            unhauscomparbeits unhauscompcomp unhauscompcompen unhausunhausarbeits unhausunhauscomp unhausunhauscompen",
    },
    Pinned {
        name: "positions",
        aff: "COMPOUNDFLAG X\nCOMPOUNDBEGIN B\nCOMPOUNDMIDDLE M\nCOMPOUNDEND E",
        dic: "beg/B mid/M end/E any/X all/BME",
        pieces: "beg mid end any all",
        extra: "",
        right: true,
        listed: "beg mid end any all begend begany begall anyend anyany anyall allend allany allall begmidend begmidany \
            begmidall beganyend beganyany beganyall begallend begallany begallall anymidend anymidany anymidall anyanyend \
            anyanyany anyanyall anyallend anyallany anyallall allmidend allmidany allmidall allanyend allanyany allanyall \
            allallend allallany allallall",
    },
    Pinned {
        name: "first_last",
        aff: "COMPOUNDFIRST B\nCOMPOUNDLAST E",
        dic: "beg/B end/E",
        pieces: "beg end",
        extra: "",
        right: true,
        listed: "beg end",
    },
    Pinned {
        name: "onlyincompound",
        aff: "COMPOUNDFLAG X\nONLYINCOMPOUND O\nSFX A Y 1\nSFX A 0 er/B .\nSFX B Y 1\nSFX B 0 s .\nSFX F Y 1\nSFX F 0 s/O .",
        dic: "foo/XAF bar/XAF zap/XO qux/A",
        pieces: "foo foos fooers bar zap",
        extra: "zap foos fooer fooers quxer quxers",
        right: true,
        listed: "foo fooers bar foofoo foofooers foobar foozap barfoo barfooers barbar barzap zapfoo zapfooers zapbar zapzap \
            foofoofoo foofoofooers foofoobar foofoozap foobarfoo foobarfooers foobarbar foobarzap foozapfoo foozapfooers \
            foozapbar foozapzap barfoofoo barfoofooers barfoobar barfoozap barbarfoo barbarfooers barbarbar barbarzap \
            barzapfoo barzapfooers barzapbar barzapzap zapfoofoo zapfoofooers zapfoobar zapfoozap zapbarfoo zapbarfooers \
            zapbarbar zapbarzap zapzapfoo zapzapfooers zapzapbar zapzapzap fooer quxer quxers",
    },
    Pinned {
        name: "onlyincompound_permit",
        aff: "COMPOUNDFLAG X\nONLYINCOMPOUND O\nCOMPOUNDPERMITFLAG P\nSFX F Y 1\nSFX F 0 s/OP .\nSFX G Y 1\nSFX G 0 s/O .\nPFX H Y 1\nPFX H 0 un/O .\nPFX I Y 1\nPFX I 0 re/OP .",
        dic: "foo/XFHI bar/XFHI baz/XG",
        pieces: "foo foos unfoo refoo bazs",
        extra: "foos unfoo refoo bazs baz",
        right: true,
        listed: "foo foofoo foorefoo foosfoo foosrefoo unfoofoo unfoorefoo refoofoo refoorefoo foofoofoo foofoorefoo foofoosfoo \
            foofoosrefoo foounfoofoo foounfoorefoo foorefoofoo foorefoorefoo foosfoofoo foosfoorefoo foosfoosfoo \
            foosfoosrefoo foosunfoofoo foosunfoorefoo foosrefoofoo foosrefoorefoo unfoofoofoo unfoofoorefoo unfoofoosfoo \
            unfoofoosrefoo unfoounfoofoo unfoounfoorefoo unfoorefoofoo unfoorefoorefoo refoofoofoo refoofoorefoo \
            refoofoosfoo refoofoosrefoo refoounfoofoo refoounfoorefoo refoorefoofoo refoorefoorefoo baz",
    },
    Pinned {
        name: "forbid_needaffix",
        aff: "COMPOUNDFLAG X\nCOMPOUNDFORBIDFLAG Q\nCOMPOUNDPERMITFLAG P\nNEEDAFFIX N\nSFX S Y 1\nSFX S 0 s .\nSFX T Y 1\nSFX T 0 t/Q .\nSFX U Y 1\nSFX U 0 u/QP .\nSFX V Y 1\nSFX V 0 v/P .",
        dic: "foo/XSTUV bar/XSTUV qoo/XQS nee/XNSV noo/NSV",
        pieces: "foo foot foou qoo nee nees neev",
        extra: "nee noo noos",
        right: true,
        listed: "foo foot foou qoo nees neev foofoo foofoot foofoou fooqoo foonees fooneev neevfoo neevfoot neevfoou neevqoo \
            neevnees neevneev foofoofoo foofoofoot foofoofoou foofooqoo foofoonees foofooneev fooneevfoo fooneevfoot \
            fooneevfoou fooneevqoo fooneevnees fooneevneev neevfoofoo neevfoofoot neevfoofoou neevfooqoo neevfoonees \
            neevfooneev neevneevfoo neevneevfoot neevneevfoou neevneevqoo neevneevnees neevneevneev noos",
    },
    Pinned {
        name: "dup",
        aff: "COMPOUNDFLAG X\nCHECKCOMPOUNDDUP\nSFX S Y 1\nSFX S 0 s .",
        dic: "foo/XS bar/XS foos/X",
        pieces: "foo foos bar",
        extra: "",
        right: false,
        listed: "foofoo barbar foofoofoo foobarbar foosfoofoo foosbarbar barfoofoo barbarbar",
    },
    Pinned {
        name: "case",
        aff: "COMPOUNDFLAG X\nCHECKCOMPOUNDCASE",
        dic: "foo/X Bar/X baz/X QUX/X",
        pieces: "foo Bar baz QUX",
        extra: "",
        right: true,
        listed: "foo Bar baz QUX foofoo foobaz Barfoo Barbaz bazfoo bazbaz foofoofoo foofoobaz foobazfoo foobazbaz Barfoofoo \
            Barfoobaz Barbazfoo Barbazbaz bazfoofoo bazfoobaz bazbazfoo bazbazbaz",
    },
    Pinned {
        name: "triple",
        aff: "COMPOUNDFLAG X\nCHECKCOMPOUNDTRIPLE",
        dic: "foo/X bar/X schiff/X fahrt/X ooze/X baa/X",
        pieces: "foo schiff fahrt ooze baa",
        extra: "",
        right: false,
        listed: "fooooze schifffoo schifffahrt foofooooze fooschifffoo fooschifffahrt foooozefoo foooozeschiff foooozefahrt \
            foooozeooze foooozebaa schifffoofoo schifffooschiff schifffoofahrt schifffooooze schifffoobaa schiffschifffoo \
            schiffschifffahrt schifffahrtfoo schifffahrtschiff schifffahrtfahrt schifffahrtooze schifffahrtbaa \
            fahrtfooooze fahrtschifffoo fahrtschifffahrt oozefooooze oozeschifffoo oozeschifffahrt baafooooze baaschifffoo \
            baaschifffahrt",
    },
    Pinned {
        name: "simplified_triple",
        aff: "COMPOUNDFLAG X\nCHECKCOMPOUNDTRIPLE\nSIMPLIFIEDTRIPLE",
        dic: "foo/X bar/X schiff/X fahrt/X ooze/X baa/X",
        pieces: "foo schiff fahrt ooze baa",
        extra: "schiffahrt",
        right: false,
        listed: "fooooze schifffoo schifffahrt foofooooze fooschifffoo fooschifffahrt foooozefoo foooozeschiff foooozefahrt \
            foooozeooze foooozebaa schifffoofoo schifffooschiff schifffoofahrt schifffooooze schifffoobaa schiffschifffoo \
            schiffschifffahrt schifffahrtfoo schifffahrtschiff schifffahrtfahrt schifffahrtooze schifffahrtbaa \
            fahrtfooooze fahrtschifffoo fahrtschifffahrt oozefooooze oozeschifffoo oozeschifffahrt baafooooze baaschifffoo \
            baaschifffahrt",
    },
    Pinned {
        name: "simplified_only",
        aff: "COMPOUNDFLAG X\nSIMPLIFIEDTRIPLE",
        dic: "foo/X schiff/X fahrt/X ooze/X",
        pieces: "foo schiff fahrt ooze",
        extra: "schiffahrt fooze",
        right: false,
        listed: "fooze",
    },
    Pinned {
        name: "rep",
        aff: "COMPOUNDFLAG X\nCHECKCOMPOUNDREP\nREP 4\nREP oo o\nREP rb r\nREP ^ba x\nREP az$ x\nSFX S Y 1\nSFX S 0 s .",
        dic: "foo/XS bar/XS baz/X fobar/S barb/X baraz/S fobaz bx",
        pieces: "foo foos bar bars baz barb",
        extra: "",
        right: true,
        listed: "foo foos bar bars baz barb foofoo foofoos foobarb barfoo barfoos barbar barbars barbarb bazfoo bazfoos bazbar \
            bazbars bazbaz bazbarb barbfoo barbfoos barbbar barbbars barbbaz barbbarb foofoofoo foofoofoos foofoobarb \
            foobarbfoo foobarbfoos foobarbbar foobarbbars foobarbbaz foobarbbarb barfoofoo barfoofoos barfoobarb barbarfoo \
            barbarfoos barbarbar barbarbars barbarbarb barbarbfoo barbarbfoos barbarbbar barbarbbars barbarbbaz \
            barbarbbarb bazfoofoo bazfoofoos bazfoobarb bazbarfoo bazbarfoos bazbarbar bazbarbars bazbarbarb bazbazfoo \
            bazbazfoos bazbazbar bazbazbars bazbazbaz bazbazbarb bazbarbfoo bazbarbfoos bazbarbbar bazbarbbars bazbarbbaz \
            bazbarbbarb barbfoofoo barbfoofoos barbfoobarb barbbarfoo barbbarfoos barbbarbar barbbarbars barbbarbarb \
            barbbazfoo barbbazfoos barbbazbar barbbazbars barbbazbaz barbbazbarb barbbarbfoo barbbarbfoos barbbarbbar \
            barbbarbbars barbbarbbaz barbbarbbarb",
    },
    Pinned {
        name: "rep_spans",
        aff: "COMPOUNDFLAG X\nCHECKCOMPOUNDREP\nREP 1\nREP oo o",
        dic: "foo/X bar/X baz/X qux/X fobarbaz quxfobar",
        pieces: "foo bar baz qux",
        extra: "",
        right: false,
        listed: "foobarbaz quxfoobar",
    },
    Pinned {
        name: "three_parts",
        aff: "COMPOUNDFLAG X",
        dic: "foo/X bar/X baz/X qux/X foobar fooba barbazqux",
        pieces: "foo bar baz qux",
        extra: "fooxbarbaz",
        right: false,
        listed: "foofoobar foobarfoo foobarbar foobarbaz foobarqux barfoobar bazfoobar bazbarqux quxfoobar fooxbarbaz",
    },
    Pinned {
        name: "three_parts_nosplitsugs",
        aff: "COMPOUNDFLAG X\nNOSPLITSUGS",
        dic: "foo/X bar/X baz/X qux/X foobar fooba",
        pieces: "foo bar baz qux",
        extra: "",
        right: false,
        listed: "",
    },
    Pinned {
        name: "pattern",
        aff: "COMPOUNDFLAG X\nCOMPOUNDPERMITFLAG P\nCHECKCOMPOUNDPATTERN 3\nCHECKCOMPOUNDPATTERN oo ba # comment\nCHECKCOMPOUNDPATTERN /F /G\nCHECKCOMPOUNDPATTERN er /H\nSFX S Y 1\nSFX S 0 s/P .\nSFX T Y 1\nSFX T 0 t/PF .\nSFX E Y 1\nSFX E 0 er/P .",
        dic: "foo/XST bar/XSG baz/XS qux/XF zed/XH hey/XSE heyer/XS",
        pieces: "foo foot bar baz qux zed heyer hey",
        extra: "",
        right: false,
        listed: "foobar foobaz footbar quxbar heyerzed foofoobar foofoobaz foofootbar foobarfoo foobarfoot foobarbar foobarbaz \
            foobarqux foobarzed foobarheyer foobarhey foobazfoo foobazfoot foobazbar foobazbaz foobazqux foobazzed \
            foobazheyer foobazhey fooquxbar fooheyerzed footfoobar footfoobaz footfootbar footbarfoo footbarfoot \
            footbarbar footbarbaz footbarqux footbarzed footbarheyer footbarhey footquxbar footheyerzed barfoobar \
            barfoobaz barfootbar barquxbar barheyerzed bazfoobar bazfoobaz bazfootbar bazquxbar bazheyerzed quxfoobar \
            quxfoobaz quxfootbar quxbarfoo quxbarfoot quxbarbar quxbarbaz quxbarqux quxbarzed quxbarheyer quxbarhey \
            quxquxbar quxheyerzed zedfoobar zedfoobaz zedfootbar zedquxbar zedheyerzed heyerfoobar heyerfoobaz \
            heyerfootbar heyerquxbar heyerzedfoo heyerzedfoot heyerzedbar heyerzedbaz heyerzedqux heyerzedzed \
            heyerzedheyer heyerzedhey heyerheyerzed heyfoobar heyfoobaz heyfootbar heyquxbar heyheyerzed",
    },
    Pinned {
        name: "pattern_unmodified_forceucase",
        aff: "COMPOUNDFLAG X\nFORCEUCASE U\nCHECKCOMPOUNDPATTERN 1\nCHECKCOMPOUNDPATTERN 0/A b\nCOMPOUNDPERMITFLAG P\nSFX S Y 1\nSFX S 0 s .\nSFX R Y 1\nSFX R 0 r/P .",
        dic: "foo/XSR straat/XUS bar/X zoo/XAR big/X",
        pieces: "foo straat bar zoo zoor big",
        extra: "Foostraat FOOSTRAAT Foostraats",
        right: false,
        listed: "foostraat straatstraat barstraat zoostraat zoobar zoobig zoorstraat bigstraat foofoostraat foostraatstraat \
            foobarstraat foozoostraat foozoobar foozoobig foozoorstraat foobigstraat straatfoostraat straatstraatstraat \
            straatbarstraat straatzoostraat straatzoobar straatzoobig straatzoorstraat straatbigstraat barfoostraat \
            barstraatstraat barbarstraat barzoostraat barzoobar barzoobig barzoorstraat barbigstraat zoofoostraat \
            zoostraatstraat zoobarfoo zoobarstraat zoobarbar zoobarzoo zoobarzoor zoobarbig zoozoostraat zoozoobar \
            zoozoobig zoozoorstraat zoobigfoo zoobigstraat zoobigbar zoobigzoo zoobigzoor zoobigbig zoorfoostraat \
            zoorstraatstraat zoorbarstraat zoorzoostraat zoorzoobar zoorzoobig zoorzoorstraat zoorbigstraat bigfoostraat \
            bigstraatstraat bigbarstraat bigzoostraat bigzoobar bigzoobig bigzoorstraat bigbigstraat",
    },
    Pinned {
        name: "wordmax_root",
        aff: "COMPOUNDFLAG X\nCOMPOUNDWORDMAX 2\nCOMPOUNDROOT R",
        dic: "foo/X bar/X foobaz/XR",
        pieces: "foo bar foobaz",
        extra: "",
        right: true,
        listed: "foo bar foobaz foofoo foobar barfoo barbar",
    },
    Pinned {
        name: "wordmax",
        aff: "COMPOUNDFLAG X\nCOMPOUNDWORDMAX 3\nCOMPOUNDMIN 1",
        dic: "fo/X ba/X",
        pieces: "fo ba",
        extra: "fobafoba fobafobafo",
        right: false,
        listed: "fobafoba fobafobafo",
    },
    Pinned {
        name: "syllable",
        aff: "COMPOUNDFLAG X\nCOMPOUNDWORDMAX 2\nCOMPOUNDSYLLABLE 6 aeiou\nCOMPOUNDMIN 1",
        dic: "ba/X be/X baaa/X",
        pieces: "ba be baaa",
        extra: "babababa",
        right: false,
        listed: "",
    },
    Pinned {
        name: "syllable_hu",
        aff: "LANG hu_HU\nCOMPOUNDFLAG X\nCOMPOUNDWORDMAX 2 y\nCOMPOUNDSYLLABLE 6 aeiou\nCOMPOUNDMIN 1",
        dic: "ba/X be/X baaa/X",
        pieces: "ba be baaa",
        extra: "babababa",
        right: false,
        listed: "babaaabaaa bebaaabaaa baaababaaa baaabebaaa baaabaaaba baaabaaabe baaabaaabaaa",
    },
    Pinned {
        name: "near_slips",
        aff: "COMPOUNDFLAG X\nCOMPOUNDMIN 4\nTRY x\nREP 1\nREP ^ab yz\nMAP 1\nMAP eë",
        dic: "abcd/X efgh/X ijkl/X mnop/X abcdefghikjl efghabcdjkli ijklabcdefgg xnopabcdefgh yzcdmnopijkl ëfghmnopabcd Ijklmnopabcd",
        pieces: "abcd efgh ijkl mnop",
        extra: "",
        right: false,
        listed: "abcdefghijkl abcdmnopijkl efghabcdijkl efghmnopabcd ijklabcdefgh ijklmnopabcd mnopabcdefgh",
    },
    Pinned {
        name: "onlyincompound_homonyms",
        aff: "COMPOUNDFLAG X\nONLYINCOMPOUND O\nSFX S Y 1\nSFX S 0 s .\nSFX T Y 1\nSFX T 0 t .",
        dic: "foo/XOS foo/T bar/X baz/XO baz/XT",
        pieces: "foo foos foot bar baz bazt bazs",
        extra: "",
        right: true,
        listed: "foo foot bar baz bazt foofoo foofoos foobar foobaz foobazt barfoo barfoos barbar barbaz barbazt bazfoo bazfoos \
            bazbar bazbaz bazbazt foofoofoo foofoofoos foofoobar foofoobaz foofoobazt foobarfoo foobarfoos foobarbar \
            foobarbaz foobarbazt foobazfoo foobazfoos foobazbar foobazbaz foobazbazt barfoofoo barfoofoos barfoobar \
            barfoobaz barfoobazt barbarfoo barbarfoos barbarbar barbarbaz barbarbazt barbazfoo barbazfoos barbazbar \
            barbazbaz barbazbazt bazfoofoo bazfoofoos bazfoobar bazfoobaz bazfoobazt bazbarfoo bazbarfoos bazbarbar \
            bazbarbaz bazbarbazt bazbazfoo bazbazfoos bazbazbar bazbazbaz bazbazbazt",
    },
    Pinned {
        name: "onlyincompound_homonyms_alone",
        aff: "ONLYINCOMPOUND O\nNEEDAFFIX N\nSFX S Y 1\nSFX S 0 s .\nPFX U Y 1\nPFX U 0 un .",
        dic: "bar/OS bar/U qux/O qux/S zed/OS",
        pieces: "",
        extra: "bar bars unbar qux quxs unqux zed zeds",
        right: true,
        listed: "bar unbar qux quxs",
    },
    Pinned {
        name: "simplified_triple_checked",
        aff: "COMPOUNDFLAG X\nCHECKCOMPOUNDTRIPLE\nSIMPLIFIEDTRIPLE",
        dic: "boo/X oob/X ob/X xaa/X aay/X ay/X schiff/X fahrt/X ffahrt/X buss/X ssen/X fjell/X llama/X",
        pieces: "",
        extra: "booob boob boooob boobob xaaay xaay xaaaay schiffahrt schifffahrt schiffffahrt booobboo boobboo busssen bussen fjelllama fjellama",
        right: true,
        listed: "schiffahrt",
    },
    Pinned {
        name: "simplified_triple_min",
        aff: "COMPOUNDFLAG X\nCOMPOUNDMIN 3\nCHECKCOMPOUNDTRIPLE\nSIMPLIFIEDTRIPLE",
        dic: "foo/X ooa/X oo/X bb/X bbb/X abb/X bba/X",
        pieces: "foo ooa oo abb bba bbb",
        extra: "fooa abba abbba abbbba",
        right: true,
        listed: "foo ooa oo abb bba bbb foofoo fooabb foobba foobbb ooafoo ooaooa ooaabb ooabba ooabbb abbfoo abbooa abbabb \
            bbafoo bbaooa bbaabb bbabba bbabbb bbbfoo bbbooa bbbabb foofoofoo foofooabb foofoobba foofoobbb fooabbfoo \
            fooabbooa fooabbabb foobbafoo foobbaooa foobbaabb foobbabba foobbabbb foobbbfoo foobbbooa foobbbabb ooafoofoo \
            ooafooabb ooafoobba ooafoobbb ooaooafoo ooaooaooa ooaooaabb ooaooabba ooaooabbb ooaabbfoo ooaabbooa ooaabbabb \
            ooabbafoo ooabbaooa ooabbaabb ooabbabba ooabbabbb ooabbbfoo ooabbbooa ooabbbabb abbfoofoo abbfooabb abbfoobba \
            abbfoobbb abbooafoo abbooaooa abbooaabb abbooabba abbooabbb abbabbfoo abbabbooa abbabbabb bbafoofoo bbafooabb \
            bbafoobba bbafoobbb bbaooafoo bbaooaooa bbaooaabb bbaooabba bbaooabbb bbaabbfoo bbaabbooa bbaabbabb bbabbafoo \
            bbabbaooa bbabbaabb bbabbabba bbabbabbb bbabbbfoo bbabbbooa bbabbbabb bbbfoofoo bbbfooabb bbbfoobba bbbfoobbb \
            bbbooafoo bbbooaooa bbbooaabb bbbooabba bbbooabbb bbbabbfoo bbbabbooa bbbabbabb",
    },
    Pinned {
        name: "keepcase_parts",
        aff: "COMPOUNDFLAG X\nKEEPCASE K",
        dic: "foo/X baz/XK bar/X",
        pieces: "foo baz bar Foo Baz BAZ FOO",
        extra: "",
        right: true,
        listed: "foo baz bar Foo FOO foofoo foobaz foobar bazfoo bazbaz bazbar barfoo barbaz barbar Foofoo Foobaz Foobar FOOBAZ \
            FOOFOO foofoofoo foofoobaz foofoobar foobazfoo foobazbaz foobazbar foobarfoo foobarbaz foobarbar bazfoofoo \
            bazfoobaz bazfoobar bazbazfoo bazbazbaz bazbazbar bazbarfoo bazbarbaz bazbarbar barfoofoo barfoobaz barfoobar \
            barbazfoo barbazbaz barbazbar barbarfoo barbarbaz barbarbar Foofoofoo Foofoobaz Foofoobar Foobazfoo Foobazbaz \
            Foobazbar Foobarfoo Foobarbaz Foobarbar FOOBAZBAZ FOOBAZFOO FOOFOOBAZ FOOFOOFOO",
    },
    Pinned {
        name: "capitalized_stem_parts",
        aff: "COMPOUNDFLAG X",
        dic: "foo/X Qux/X bar/X",
        pieces: "foo Qux qux bar Foo QUX",
        extra: "",
        right: true,
        listed: "foo Qux bar Foo QUX foofoo fooQux foobar Quxfoo QuxQux Quxbar barfoo barQux barbar Foofoo FooQux Foobar \
            foofoofoo foofooQux foofoobar fooQuxfoo fooQuxQux fooQuxbar foobarfoo foobarQux foobarbar Quxfoofoo QuxfooQux \
            Quxfoobar QuxQuxfoo QuxQuxQux QuxQuxbar Quxbarfoo QuxbarQux Quxbarbar barfoofoo barfooQux barfoobar barQuxfoo \
            barQuxQux barQuxbar barbarfoo barbarQux barbarbar Foofoofoo FoofooQux Foofoobar FooQuxfoo FooQuxQux FooQuxbar \
            Foobarfoo FoobarQux Foobarbar",
    },
    Pinned {
        name: "forceucase_mixed",
        aff: "COMPOUNDFLAG X\nFORCEUCASE U\nKEEPCASE K",
        dic: "foo/X bar/XU baz/XK Qux/X",
        pieces: "foo bar baz Foo Bar Qux",
        extra: "FOOBAR FooBar fooBar FOOBAZ Foobaz QuxBar Quxbar QUXBAR",
        right: true,
        listed: "foo bar baz Foo Bar Qux foofoo foobaz fooQux barfoo barbaz barQux bazfoo bazbaz bazQux Foofoo Foobar Foobaz \
            FooQux Barfoo Barbar Barbaz BarQux Quxfoo Quxbar Quxbaz QuxQux foofoofoo foofoobaz foofooQux foobarfoo \
            foobarbaz foobarQux foobazfoo foobazbaz foobazQux fooQuxfoo fooQuxbar fooQuxbaz fooQuxQux barfoofoo barfoobaz \
            barfooQux barbarfoo barbarbaz barbarQux barbazfoo barbazbaz barbazQux barQuxfoo barQuxbar barQuxbaz barQuxQux \
            bazfoofoo bazfoobaz bazfooQux bazbarfoo bazbarbaz bazbarQux bazbazfoo bazbazbaz bazbazQux bazQuxfoo bazQuxbar \
            bazQuxbaz bazQuxQux Foofoofoo Foofoobar Foofoobaz FoofooQux Foobarfoo Foobarbar Foobarbaz FoobarQux Foobazfoo \
            Foobazbar Foobazbaz FoobazQux FooQuxfoo FooQuxbar FooQuxbaz FooQuxQux Barfoofoo Barfoobar Barfoobaz BarfooQux \
            Barbarfoo Barbarbar Barbarbaz BarbarQux Barbazfoo Barbazbar Barbazbaz BarbazQux BarQuxfoo BarQuxbar BarQuxbaz \
            BarQuxQux Quxfoofoo Quxfoobar Quxfoobaz QuxfooQux Quxbarfoo Quxbarbar Quxbarbaz QuxbarQux Quxbazfoo Quxbazbar \
            Quxbazbaz QuxbazQux QuxQuxfoo QuxQuxbar QuxQuxbaz QuxQuxQux FOOBAR FOOBAZ QUXBAR",
    },
    Pinned {
        name: "forceucase_suffix",
        aff: "COMPOUNDFLAG X\nFORCEUCASE U\nSFX S Y 1\nSFX S 0 s/U .",
        dic: "foo/X baz/XS",
        pieces: "foo baz bazs",
        extra: "Foobazs",
        right: false,
        listed: "bazsfoo bazsbaz bazsbazs foobazsfoo foobazsbaz foobazsbazs bazbazsfoo bazbazsbaz bazbazsbazs bazsfoofoo \
            bazsfoobaz bazsfoobazs bazsbazfoo bazsbazbaz bazsbazbazs bazsbazsfoo bazsbazsbaz bazsbazsbazs",
    },
    Pinned {
        name: "pattern_unsuffixed",
        aff: "COMPOUNDFLAG X\nCOMPOUNDPERMITFLAG P\nSFX S Y 1\nSFX S 0 s/P .\nPFX U Y 1\nPFX U 0 un/P .\nCHECKCOMPOUNDPATTERN 2\nCHECKCOMPOUNDPATTERN 0 b\nCHECKCOMPOUNDPATTERN 0 un",
        dic: "foo/XSU bar/XSU oof/X",
        pieces: "foo foos unfoo bar bars unbar oof",
        extra: "",
        right: true,
        listed: "foo foos unfoo bar bars unbar oof foofoo foofoos foooof foosfoo foosfoos foosunfoo foosbar foosbars foosunbar \
            foosoof unfoofoo unfoofoos unfoooof barfoo barfoos baroof barsfoo barsfoos barsunfoo barsbar barsbars \
            barsunbar barsoof unbarfoo unbarfoos unbaroof ooffoo ooffoos oofoof foofoofoo foofoofoos foofoooof foofoosfoo \
            foofoosfoos foofoosunfoo foofoosbar foofoosbars foofoosunbar foofoosoof fooooffoo fooooffoos foooofoof \
            foosfoofoo foosfoofoos foosfoooof foosfoosfoo foosfoosfoos foosfoosunfoo foosfoosbar foosfoosbars \
            foosfoosunbar foosfoosoof foosunfoofoo foosunfoofoos foosunfoooof foosbarfoo foosbarfoos foosbaroof \
            foosbarsfoo foosbarsfoos foosbarsunfoo foosbarsbar foosbarsbars foosbarsunbar foosbarsoof foosunbarfoo \
            foosunbarfoos foosunbaroof foosooffoo foosooffoos foosoofoof unfoofoofoo unfoofoofoos unfoofoooof unfoofoosfoo \
            unfoofoosfoos unfoofoosunfoo unfoofoosbar unfoofoosbars unfoofoosunbar unfoofoosoof unfooooffoo unfooooffoos \
            unfoooofoof barfoofoo barfoofoos barfoooof barfoosfoo barfoosfoos barfoosunfoo barfoosbar barfoosbars \
            barfoosunbar barfoosoof barooffoo barooffoos baroofoof barsfoofoo barsfoofoos barsfoooof barsfoosfoo \
            barsfoosfoos barsfoosunfoo barsfoosbar barsfoosbars barsfoosunbar barsfoosoof barsunfoofoo barsunfoofoos \
            barsunfoooof barsbarfoo barsbarfoos barsbaroof barsbarsfoo barsbarsfoos barsbarsunfoo barsbarsbar barsbarsbars \
            barsbarsunbar barsbarsoof barsunbarfoo barsunbarfoos barsunbaroof barsooffoo barsooffoos barsoofoof \
            unbarfoofoo unbarfoofoos unbarfoooof unbarfoosfoo unbarfoosfoos unbarfoosunfoo unbarfoosbar unbarfoosbars \
            unbarfoosunbar unbarfoosoof unbarooffoo unbarooffoos unbaroofoof ooffoofoo ooffoofoos ooffoooof ooffoosfoo \
            ooffoosfoos ooffoosunfoo ooffoosbar ooffoosbars ooffoosunbar ooffoosoof oofooffoo oofooffoos oofoofoof",
    },
    Pinned {
        name: "forbidflag_prefix",
        aff: "COMPOUNDFLAG X\nCOMPOUNDPERMITFLAG P\nCOMPOUNDFORBIDFLAG Q\nPFX C Y 1\nPFX C 0 de/PQ .",
        dic: "foo/XC bar/X",
        pieces: "foo defoo bar",
        extra: "",
        right: true,
        listed: "foo defoo bar foofoo foobar barfoo barbar foofoofoo foofoobar foobarfoo foobarbar barfoofoo barfoobar \
            barbarfoo barbarbar",
    },
    Pinned {
        name: "forbidden",
        aff: "COMPOUNDFLAG X\nFORBIDDENWORD F\nSFX S Y 1\nSFX S 0 en .",
        dic: "foo/X brok/XS broken/F bar/XF foobar/F",
        pieces: "foo brok broken bar",
        extra: "foobar broken",
        right: true,
        listed: "foo brok foofoo foobrok foobroken brokfoo brokbrok brokbroken foofoofoo foofoobrok foofoobroken foobrokfoo \
            foobrokbrok foobrokbroken brokfoofoo brokfoobrok brokfoobroken brokbrokfoo brokbrokbrok brokbrokbroken",
    },
    Pinned {
        name: "mixed_keep",
        aff: "COMPOUNDFLAG X\nKEEPCASE K",
        dic: "foo/X baz/XK Qux/X bar/X",
        pieces: "foo baz Qux Baz Foo BAZ QUX",
        extra: "BazQux BazfooQux FooQux FooBaz fooBaz FOOQux BAZQux",
        right: true,
        listed: "foo baz Qux Foo QUX foofoo foobaz fooQux bazfoo bazbaz bazQux Quxfoo Quxbaz QuxQux Foofoo Foobaz FooQux QUXBAZ \
            foofoofoo foofoobaz foofooQux foobazfoo foobazbaz foobazQux fooQuxfoo fooQuxbaz fooQuxQux bazfoofoo bazfoobaz \
            bazfooQux bazbazfoo bazbazbaz bazbazQux bazQuxfoo bazQuxbaz bazQuxQux Quxfoofoo Quxfoobaz QuxfooQux Quxbazfoo \
            Quxbazbaz QuxbazQux QuxQuxfoo QuxQuxbaz QuxQuxQux Foofoofoo Foofoobaz FoofooQux Foobazfoo Foobazbaz FoobazQux \
            FooQuxfoo FooQuxbaz FooQuxQux QUXBAZBAZ",
    },
    Pinned {
        name: "mixed_case_chk",
        aff: "COMPOUNDFLAG X\nCHECKCOMPOUNDCASE",
        dic: "foo/X Qux/X bar/X",
        pieces: "foo Qux Foo bar Bar",
        extra: "",
        right: true,
        listed: "foo Qux Foo bar Bar foofoo foobar Quxfoo Quxbar Foofoo Foobar barfoo barbar Barfoo Barbar foofoofoo foofoobar \
            foobarfoo foobarbar Quxfoofoo Quxfoobar Quxbarfoo Quxbarbar Foofoofoo Foofoobar Foobarfoo Foobarbar barfoofoo \
            barfoobar barbarfoo barbarbar Barfoofoo Barfoobar Barbarfoo Barbarbar",
    },
    Pinned {
        name: "mixed_forbid",
        aff: "COMPOUNDFLAG X\nFORBIDDENWORD F",
        dic: "foo/X Qux/X fooQux/F bar/X",
        pieces: "foo Qux Foo bar",
        extra: "FooQux fooQux FOOQUX",
        right: false,
        listed: "fooQux fooFoo QuxFoo FooQux FooFoo barFoo foofooFoo fooQuxFoo fooFoofoo fooFooQux fooFooFoo fooFoobar \
            foobarFoo QuxfooFoo QuxQuxFoo QuxFoofoo QuxFooQux QuxFooFoo QuxFoobar QuxbarFoo FoofooFoo FooQuxFoo FooFoofoo \
            FooFooQux FooFooFoo FooFoobar FoobarFoo barfooFoo barQuxFoo barFoofoo barFooQux barFooFoo barFoobar barbarFoo \
            FOOQUX",
    },
    Pinned {
        name: "mixed_onlyc",
        aff: "COMPOUNDFLAG X\nONLYINCOMPOUND O",
        dic: "foo/XO Qux/X bar/X",
        pieces: "foo Qux Foo bar Bar",
        extra: "",
        right: true,
        listed: "Qux bar Bar foofoo fooQux foobar Quxfoo QuxQux Quxbar Foofoo FooQux Foobar barfoo barQux barbar Barfoo BarQux \
            Barbar foofoofoo foofooQux foofoobar fooQuxfoo fooQuxQux fooQuxbar foobarfoo foobarQux foobarbar Quxfoofoo \
            QuxfooQux Quxfoobar QuxQuxfoo QuxQuxQux QuxQuxbar Quxbarfoo QuxbarQux Quxbarbar Foofoofoo FoofooQux Foofoobar \
            FooQuxfoo FooQuxQux FooQuxbar Foobarfoo FoobarQux Foobarbar barfoofoo barfooQux barfoobar barQuxfoo barQuxQux \
            barQuxbar barbarfoo barbarQux barbarbar Barfoofoo BarfooQux Barfoobar BarQuxfoo BarQuxQux BarQuxbar Barbarfoo \
            BarbarQux Barbarbar",
    },
    Pinned {
        name: "mixed_force",
        aff: "COMPOUNDFLAG X\nFORCEUCASE U",
        dic: "foo/X Qux/XU bar/XU",
        pieces: "foo Qux Foo bar Bar",
        extra: "",
        right: true,
        listed: "foo Qux Foo bar Bar foofoo fooQux Quxfoo QuxQux Quxbar Foofoo FooQux Foobar barfoo barQux Barfoo BarQux Barbar \
            foofoofoo foofooQux fooQuxfoo fooQuxQux fooQuxbar foobarfoo foobarQux Quxfoofoo QuxfooQux Quxfoobar QuxQuxfoo \
            QuxQuxQux QuxQuxbar Quxbarfoo QuxbarQux Quxbarbar Foofoofoo FoofooQux Foofoobar FooQuxfoo FooQuxQux FooQuxbar \
            Foobarfoo FoobarQux Foobarbar barfoofoo barfooQux barQuxfoo barQuxQux barQuxbar barbarfoo barbarQux Barfoofoo \
            BarfooQux Barfoobar BarQuxfoo BarQuxQux BarQuxbar Barbarfoo BarbarQux Barbarbar",
    },
    Pinned {
        name: "keep_nosug",
        aff: "COMPOUNDFLAG X\nKEEPCASE K\nNOSUGGEST K",
        dic: "foo/X baz/XK",
        pieces: "foo baz Foo Baz",
        extra: "",
        right: true,
        listed: "foo baz Foo foofoo foobaz bazfoo bazbaz Foofoo Foobaz foofoofoo foofoobaz foobazfoo foobazbaz bazfoofoo \
            bazfoobaz bazbazfoo bazbazbaz Foofoofoo Foofoobaz Foobazfoo Foobazbaz",
    },
    Pinned {
        name: "root_affix",
        aff: "COMPOUNDFLAG Y\nCOMPOUNDWORDMAX 2\nCOMPOUNDROOT R\nCOMPOUNDPERMITFLAG P\nSFX A Y 1\nSFX A 0 s/RP .\nPFX B Y 1\nPFX B 0 un/RP .",
        dic: "foo/YAB bar/YAB baz/YR",
        pieces: "foo foos unfoo bar bars baz",
        extra: "",
        right: true,
        listed: "foo foos unfoo bar bars baz foofoo foofoos foounfoo foobar foobars foosfoo foosfoos foosunfoo foosbar foosbars \
            unfoofoo unfoofoos unfoounfoo unfoobar unfoobars barfoo barfoos barunfoo barbar barbars barsfoo barsfoos \
            barsunfoo barsbar barsbars",
    },
    Pinned {
        name: "syllable_hu_suffix",
        aff: "LANG hu_HU\nCOMPOUNDMIN 1\nCOMPOUNDFLAG Y\nCOMPOUNDWORDMAX 2\nCOMPOUNDSYLLABLE 4 aeiou\nSFX C Y 1\nSFX C 0 ata .\nSFX D Y 1\nSFX D 0 ete/Z .\nSFX E Y 1\nSFX E 0 t/F .\nSFX F Y 1\nSFX F 0 ete .",
        dic: "ba/Y bab/YCDE",
        pieces: "ba bab babata babete babtete",
        extra: "bababababata babababababete bababababtete",
        right: true,
        listed: "ba bab babata babete babtete baba babab bababata bababete bababtete babba babbab babbabata babbabete \
            babbabtete bababa bababab babababata babababtete bababba bababbab bababbabata bababbabtete babbaba babbabab \
            babbababata babbababtete babbabba babbabbab babbabbabata babbabbabtete bababababata bababababtete",
    },
    Pinned {
        name: "near_prefix_shadow",
        aff: "COMPOUNDBEGIN x\nCOMPOUNDMIDDLE y\nCOMPOUNDEND z\nNEEDAFFIX h\nONLYINCOMPOUND o\nSFX S Y 1\nSFX S 0 ad .\nPFX P Y 1\nPFX P 0 fa .\nSFX T Y 1\nSFX T 0 hr .\nPFX V Y 1\nPFX V 0 ve .",
        dic: "fahr/x rad/y verkehr/zo hrrad/P fahrrad/h rke/VT fahrr/S",
        pieces: "fahr rad verkehr",
        extra: "fahrradverkehr fahrradverkehrr fahrradverkehrad",
        right: true,
        listed: "fahr rad verkehr fahrrad fahrverkehr fahrradverkehr",
    },
];
