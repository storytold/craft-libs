//! Hunspell dictionaries installed on the system, read at runtime.
//!
//! WordCraft doesn't bundle large spelling dictionaries. For languages with an affix dictionary
//! (Polish, Russian), it reads the standard Hunspell `.aff`/`.dic` pair installed by the operating system,
//! a package manager or LibreOffice, with [`crate::affix::AffixDict::from_hunspell`]. When none is
//! installed the language has no spelling check (no marks, no errors), like Word without the
//! language's proofing tools.
//!
//! Search order: the folder set with [`set_dictionary_dir`], the directory in
//! `WORDCRAFT_DICTIONARIES`, WordCraft's own `dictionaries` folder under the user's application
//! data (on every desktop system), then an installed Language pack ([`crate::packs`]), then the
//! usual system locations. On the web there are only the first and the packs.

mod decode;
mod search;

use std::path::Path;

use crate::affix::AffixDict;

pub use decode::decode;
pub(crate) use decode::dic_entry_end;
#[cfg(test)]
pub(crate) use search::ordered;
pub use search::{
    candidates, candidates_exact, candidates_in, clear_load_error, find, find_in, load_error, note_load, search_dirs, set_dictionary_dir,
    user_candidates, user_dirs,
};

/// Largest `.aff` or `.dic` file read (real ones are a few MB).
pub(crate) const MAX_FILE: u64 = 64 << 20;

/// Read a dictionary file: a regular file of at most [`MAX_FILE`] bytes. The read itself is
/// bounded, so a file that grows after its size is checked (or a filesystem that reports a wrong
/// size) can't make it read more (#3661).
pub(crate) fn read_capped(p: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let err = |e: std::io::Error| format!("{}: {e}", p.display());
    let f = std::fs::File::open(p).map_err(err)?;
    let meta = f.metadata().map_err(err)?;
    if !meta.is_file() {
        return Err(format!("{}: not a regular file", p.display()));
    }
    if meta.len() > MAX_FILE {
        return Err(format!("{}: too large ({} bytes)", p.display(), meta.len()));
    }
    let mut buf = Vec::new();
    f.take(MAX_FILE + 1).read_to_end(&mut buf).map_err(err)?;
    if buf.len() as u64 > MAX_FILE {
        return Err(format!("{}: too large", p.display()));
    }
    Ok(buf)
}

/// Read a Hunspell pair from disk (size-capped), decoded per the `.aff`'s `SET` line.
pub fn load(aff: &Path, dic: &Path) -> Result<AffixDict, String> {
    let (aff_text, dic_text) = decode(&read_capped(aff)?, &read_capped(dic)?)?;
    let source = format!("Hunspell dictionary {}", dic.display());
    AffixDict::from_hunspell(&aff_text, &dic_text, &[source.as_str()])
}

/// [`load`] for an English dictionary: flag-based compounding (`COMPOUNDFLAG`…), which this engine
/// does not implement, is ignored rather than refused ([`AffixDict::from_hunspell_english`]); `OCONV`
/// is applied to suggestions, as for every language.
pub fn load_english(aff: &Path, dic: &Path) -> Result<AffixDict, String> {
    let (aff_text, dic_text) = decode(&read_capped(aff)?, &read_capped(dic)?)?;
    let source = format!("Hunspell dictionary {}", dic.display());
    AffixDict::from_hunspell_english(&aff_text, &dic_text, &[source.as_str()])
}

#[cfg(test)]
mod tests;
