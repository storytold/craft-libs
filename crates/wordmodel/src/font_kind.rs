//! The generic family of a font, guessed from its name: what a reader without the font should draw
//! instead. The writers that declare one share it: OOXML's font table (`w:family`, `w:pitch`),
//! ODF's font faces (`style:font-family-generic`, `style:font-pitch`) and RTF's (`\fmodern`).

/// A font's generic family. The names are the values OOXML (`ST_FontFamily`) and ODF give them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenericFamily {
    /// Serif faces (Times New Roman, Georgia, Cambria…).
    Roman,
    /// Sans-serif faces (Arial, Calibri, Liberation Sans…), and any name not known to be another.
    Swiss,
    /// Monospace faces (Courier New, Consolas…).
    Modern,
}

/// Words in the names of serif faces.
const SERIF: [&str; 16] = [
    "serif",
    "times",
    "georgia",
    "cambria",
    "caladea",
    "garamond",
    "palatino",
    "book",
    "roman",
    "libertine",
    "minion",
    "baskerville",
    "bodoni",
    "didot",
    "charter",
    "constantia",
];

impl GenericFamily {
    /// The family of the font named `name`.
    pub fn of(name: &str) -> GenericFamily {
        if is_mono(name) {
            return GenericFamily::Modern;
        }
        let f = name.to_ascii_lowercase();
        if SERIF.iter().any(|s| f.contains(s)) && !f.contains("sans") {
            return GenericFamily::Roman;
        }
        GenericFamily::Swiss
    }

    /// `roman`, `swiss` or `modern`.
    pub fn name(self) -> &'static str {
        match self {
            GenericFamily::Roman => "roman",
            GenericFamily::Swiss => "swiss",
            GenericFamily::Modern => "modern",
        }
    }

    /// Whether its faces are fixed pitch (monospace).
    pub fn fixed_pitch(self) -> bool {
        self == GenericFamily::Modern
    }
}

/// Is this a monospace family name?
pub fn is_mono(font: &str) -> bool {
    let f = font.to_ascii_lowercase();
    f.contains("courier") || f.contains("mono") || f.contains("consolas") || f.contains("menlo") || f.contains("code") || f == "monaco"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_from_names() {
        for (name, want) in [
            ("Arial", GenericFamily::Swiss),
            ("Liberation Sans", GenericFamily::Swiss),
            ("Unknown Face", GenericFamily::Swiss),
            ("Times New Roman", GenericFamily::Roman),
            ("Liberation Serif", GenericFamily::Roman),
            ("Caladea", GenericFamily::Roman),
            ("Noto Sans Serif", GenericFamily::Swiss),
            ("Courier New", GenericFamily::Modern),
            ("Liberation Mono", GenericFamily::Modern),
        ] {
            assert_eq!(GenericFamily::of(name), want, "{name}");
        }
        assert!(GenericFamily::Modern.fixed_pitch() && !GenericFamily::Roman.fixed_pitch());
    }
}
