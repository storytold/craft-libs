//! Synthetic Hebrew Hunspell dictionary used by unit tests to verify Hebrew spelling and suggestions
//! without depending on system dictionary paths.

/// Synthetic Hunspell `.aff` prefix/suffix rules for Hebrew tests.
pub const AFF: &str = "SET UTF-8\nFLAG UTF-8\nFORBIDDENWORD F\nNEEDAFFIX X\nTRY שלוםביתספרעבודהאניאתהךםןףץ\nREP 2\nREP מ ם\nREP נ ן\nPFX H Y 3\nPFX H 0 ה .\nPFX H 0 ב .\nPFX H 0 וב .\nSFX S Y 1\nSFX S 0 ים .\n";

/// Synthetic Hunspell `.dic` word list for Hebrew tests.
pub const DIC: &str =
    "15\nשלום/H\nבית/H\nספר/HS\nעולם/H\nעבודה/H\nילד/HS\nילדה/H\nירושלים\nישראל\nצה\"ל/H\nוכו\'/H\nוגו\'/H\nחייל/H\nמשרת/H\nמילהאסורה/F\n";

/// Installs the synthetic Hebrew dictionary for unit tests.
pub fn install() {
    if let Err(e) = super::install_dictionary(AFF, DIC) {
        panic!("test dictionary: {e}");
    }
}
