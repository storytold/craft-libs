//! Sample text: a sentence in the language the catalogue is filtered by, or in a family's own
//! script when it has no Latin, so a sample never comes out as boxes.

/// The English pangram, the sample for Latin.
pub const PANGRAM: &str = "The quick brown fox jumps over the lazy dog";
pub const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz";
pub const NUMERALS: &str = "0123456789 & ? ! @ # % ( ) [ ] { } $ € £ ¥";

/// Sentences by language code (gflanguages' code before the script).
const BY_LANGUAGE: &[(&str, &str)] = &[
    ("ar", "صف خلق خود كمثل الشمس إذ بزغت يحظى الضجيع بها نجلاء معطار"),
    ("bn", "সমস্ত মানুষ স্বাধীনভাবে সমান মর্যাদা এবং অধিকার নিয়ে জন্মগ্রহণ করে"),
    ("cs", "Příliš žluťoučký kůň úpěl ďábelské ódy"),
    ("de", "Victor jagt zwölf Boxkämpfer quer über den großen Sylter Deich"),
    ("el", "Ξεσκεπάζω την ψυχοφθόρα βδελυγμία"),
    ("en", PANGRAM),
    ("es", "El veloz murciélago hindú comía feliz cardillo y kiwi"),
    ("fa", "همه افراد بشر آزاد به دنیا می‌آیند"),
    ("fr", "Portez ce vieux whisky au juge blond qui fume"),
    ("he", "דג סקרן שט בים מאוכזב ולפתע מצא חברה"),
    ("hi", "सभी मनुष्यों को गौरव और अधिकारों के मामले में जन्मजात स्वतन्त्रता प्राप्त है"),
    ("hu", "Árvíztűrő tükörfúrógép"),
    ("it", "Quel vituperabile xenofobo zelante assaggia il whisky ed esclama: alleluja!"),
    ("ja", "いろはにほへと ちりぬるを 色は匂へど 散りぬるを"),
    ("ko", "다람쥐 헌 쳇바퀴에 타고파"),
    ("pl", "Pchnąć w tę łódź jeża lub ośm skrzyń fig"),
    ("pt", "Luís argüia à Júlia que «brações, fé, chá, óxido, pôr, zângão» eram palavras do português"),
    ("ru", "Съешь же ещё этих мягких французских булок, да выпей чаю"),
    ("ta", "மனிதப் பிறவியினர் சகலரும் சுதந்திரமாகவே பிறக்கின்றனர்"),
    ("th", "เป็นมนุษย์สุดประเสริฐเลิศคุณค่า"),
    ("tr", "Pijamalı hasta yağız şoföre çabucak güvendi"),
    ("uk", "Чуєш їх, доцю, га? Кумедна ж ти, прощайся без ґольфів!"),
    ("ur", "تمام انسان آزاد اور حقوق و عزت کے اعتبار سے برابر پیدا ہوئے ہیں"),
    ("vi", "Tôi có thể ăn thủy tinh mà không hại gì"),
];

const SIMPLIFIED: &str = "天地玄黄，宇宙洪荒。日月盈昃，辰宿列张。";
const TRADITIONAL: &str = "天地玄黃，宇宙洪荒。日月盈昃，辰宿列張。";

/// Sentences by ISO 15924 script.
const BY_SCRIPT: &[(&str, &str)] = &[
    ("Arab", "ar"),
    ("Beng", "bn"),
    ("Cyrl", "ru"),
    ("Deva", "hi"),
    ("Grek", "el"),
    ("Hang", "ko"),
    ("Hebr", "he"),
    ("Jpan", "ja"),
    ("Kana", "ja"),
    ("Hira", "ja"),
    ("Kore", "ko"),
    ("Latn", "en"),
    ("Taml", "ta"),
    ("Thai", "th"),
];

fn language(code: &str) -> Option<&'static str> {
    BY_LANGUAGE.iter().find(|(c, _)| *c == code).map(|(_, s)| *s)
}

/// A sentence in the language with gflanguages id `id` (`ja_Jpan`, `zh_Hant`), or in its script.
pub fn for_language(id: &str) -> Option<&'static str> {
    let mut parts = id.split('_');
    let code = parts.next().unwrap_or(id);
    let script = parts.next().unwrap_or("");
    if code == "zh" {
        return Some(if script == "Hant" { TRADITIONAL } else { SIMPLIFIED });
    }
    language(code).or_else(|| for_script(script))
}

/// A sentence in the ISO 15924 script `script`.
pub fn for_script(script: &str) -> Option<&'static str> {
    match script {
        "Hans" | "Hani" => Some(SIMPLIFIED),
        "Hant" => Some(TRADITIONAL),
        _ => BY_SCRIPT.iter().find(|(s, _)| *s == script).and_then(|(_, code)| language(code)),
    }
}

/// The sample for a family with `scripts`: the pangram when it has Latin, else a sentence in the
/// first of its scripts there is one for.
pub fn for_scripts<'a>(scripts: impl IntoIterator<Item = &'a str>) -> &'static str {
    let mut first = None;
    for script in scripts {
        if script == "Latn" {
            return PANGRAM;
        }
        first = first.or_else(|| for_script(script));
    }
    first.unwrap_or(PANGRAM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_follow_language_then_script() {
        assert_eq!(for_language("ja_Jpan"), language("ja"));
        assert_eq!(for_language("zh_Hant"), Some(TRADITIONAL));
        assert_eq!(for_language("xx_Arab"), language("ar"));
        assert_eq!(for_language("xx_Zzzz"), None);
        assert_eq!(for_scripts(["Arab", "Latn"]), PANGRAM);
        assert_eq!(for_scripts(["Zzzz", "Hebr"]), language("he").unwrap_or_default());
    }
}
