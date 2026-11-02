//! English spelling from SCOWL (Spell Checker Oriented Word Lists, Kevin Atkinson): the Hunspell
//! dictionaries of craft-i18n's `language-data/en-us`, `en-gb` (British `-ise`) and `en-ca`,
//! embedded unmodified at build time and read by the affix engine ([`crate::affix`]) on first use.
//!
//! Each English has its own: American English reads en-US, the British-spelling Englishes
//! (en-GB, en-AU, en-NZ, en-IE…) read en-GB and Canadian English reads en-CA. Its suggestions
//! come from its own dictionary, and a word is right when its own dictionary knows it: the other
//! dialects' dictionaries are never asked (en-US would take `colourblind` from en-GB's). SCOWL
//! picks one spelling per dialect (en-GB `-ise` has no `organize`, en-US no `judgement`), and Word
//! accepts more, so British English also takes the `-ize` spellings of its `-ise` words that
//! en-US or en-CA spells so, and `en-variants.txt` widens each dialect by its pairs
//! (`crate::english`). An English dictionary in the dictionaries folder comes first; a SCOWL
//! dictionary that isn't embedded, or fails to load, leaves that English to the next source (`crate::english`).
//!
//! SCOWL is under its own permissive licence and the affix files under Ispell's BSD licence;
//! [`notice`] is the upstream text, for an About window.

use std::sync::OnceLock;

use crate::ProofLang;
use crate::affix::AffixDict;
use crate::data::{EN_CA_AFF, EN_CA_SPELLING, EN_GB_AFF, EN_GB_SPELLING, EN_US_AFF, EN_US_SPELLING, SCOWL_NOTICE};

/// Per English (American, British, Canadian): the loaded dictionary, or why it isn't there.
static DICTS: [OnceLock<Result<AffixDict, String>>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];

/// The slot, `.aff` and `.dic` text of `lang`'s dictionary.
fn source(lang: ProofLang) -> (usize, &'static str, &'static str) {
    match lang {
        ProofLang::EnGb => (1, EN_GB_AFF, EN_GB_SPELLING),
        ProofLang::EnCa => (2, EN_CA_AFF, EN_CA_SPELLING),
        _ => (0, EN_US_AFF, EN_US_SPELLING),
    }
}

/// `lang`'s SCOWL dictionary (an English), when it is embedded and loads.
pub(crate) fn dict(lang: ProofLang) -> Option<&'static AffixDict> {
    let (i, aff, dic) = source(lang);
    if aff.is_empty() || dic.is_empty() {
        return None;
    }
    let loaded = DICTS.get(i)?.get_or_init(|| {
        let d = AffixDict::from_hunspell(aff, dic, &["SCOWL (wordlist.aspell.net), embedded from craft-i18n"])
            .and_then(|d| if d.is_empty() { Err("no words".to_string()) } else { Ok(d) });
        if let Err(e) = &d {
            log::error!("embedded SCOWL dictionary for {lang:?}: {e}; English spelling falls back to the next source");
        }
        d
    });
    loaded.as_ref().ok()
}

/// Does `lang` read the British `-ise` dictionary (en-GB, en-AU, en-NZ…)?
pub(crate) fn is_british(lang: ProofLang) -> bool {
    source(lang).0 == 1
}

/// Why `lang`'s embedded SCOWL dictionary failed to load, once something tried to load it.
pub(crate) fn load_error(lang: ProofLang) -> Option<String> {
    let (i, ..) = source(lang);
    DICTS.get(i)?.get()?.as_ref().err().map(|e| format!("embedded SCOWL dictionary: {e}"))
}

/// The SCOWL copyright and licence notices (the upstream README, with the Ispell BSD licence of
/// the affix files and Benjamin Titze's notice for the Australian data), when SCOWL is embedded.
pub fn notice() -> Option<&'static str> {
    (!SCOWL_NOTICE.is_empty() && !EN_US_SPELLING.is_empty()).then_some(SCOWL_NOTICE)
}
