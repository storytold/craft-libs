//! Rule-based writing checks across English, Portuguese, and Spanish:
//! balanced delimiters and quotation marks, bracket/punctuation spacing,
//! conservative contextual paronyms and homophones, and selected redundancies.
//!
//! All checks operate directly on `&str` and [`ProofLang`] with conservative
//! guards so numbers, URLs, contractions, measurements, emoticons, list
//! markers, and inline code stay quiet.

use crate::{Issue, IssueKind, ProofLang};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LangGroup {
    En,
    Pt,
    Es,
}

fn lang_group(lang: ProofLang) -> Option<LangGroup> {
    if lang.is_english() {
        Some(LangGroup::En)
    } else if lang == ProofLang::Pt || lang.built_in() == ProofLang::Pt {
        Some(LangGroup::Pt)
    } else if matches!(lang, ProofLang::Es | ProofLang::EsMx | ProofLang::Es419 | ProofLang::EsUy) {
        Some(LangGroup::Es)
    } else {
        None
    }
}

struct ContextualPattern {
    needle: &'static str,
    target_sub: &'static str,
    replacement: &'static str,
    message: &'static str,
}

const EN_PATTERNS: &[ContextualPattern] = &[
    ContextualPattern {
        needle: "their is",
        target_sub: "their",
        replacement: "there",
        message: "Use \"there\" as the existential pronoun before \"is\".",
    },
    ContextualPattern {
        needle: "their are",
        target_sub: "their",
        replacement: "there",
        message: "Use \"there\" as the existential pronoun before \"are\".",
    },
    ContextualPattern {
        needle: "their was",
        target_sub: "their",
        replacement: "there",
        message: "Use \"there\" as the existential pronoun before \"was\".",
    },
    ContextualPattern {
        needle: "their were",
        target_sub: "their",
        replacement: "there",
        message: "Use \"there\" as the existential pronoun before \"were\".",
    },
    ContextualPattern {
        needle: "over their",
        target_sub: "their",
        replacement: "there",
        message: "Use \"there\" to indicate place (\"over there\").",
    },
    ContextualPattern { needle: "there own", target_sub: "there", replacement: "their", message: "Use possessive \"their\" before \"own\"." },
    ContextualPattern { needle: "there house", target_sub: "there", replacement: "their", message: "Use possessive \"their\" before \"house\"." },
    ContextualPattern { needle: "there team", target_sub: "there", replacement: "their", message: "Use possessive \"their\" before \"team\"." },
    ContextualPattern {
        needle: "there going to",
        target_sub: "there",
        replacement: "they're",
        message: "Use contraction \"they're\" (\"they are\") before \"going to\".",
    },
    ContextualPattern {
        needle: "your welcome",
        target_sub: "your",
        replacement: "you're",
        message: "Use contraction \"you're\" (\"you are\") in \"you're welcome\".",
    },
    ContextualPattern { needle: "you're name", target_sub: "you're", replacement: "your", message: "Use possessive \"your\" before \"name\"." },
    ContextualPattern { needle: "you're team", target_sub: "you're", replacement: "your", message: "Use possessive \"your\" before \"team\"." },
    ContextualPattern {
        needle: "it's own",
        target_sub: "it's",
        replacement: "its",
        message: "Use possessive \"its\" (without apostrophe) before \"own\".",
    },
    ContextualPattern {
        needle: "its been",
        target_sub: "its",
        replacement: "it's",
        message: "Use contraction \"it's\" (\"it has\") before \"been\".",
    },
    ContextualPattern { needle: "its a", target_sub: "its", replacement: "it's", message: "Use contraction \"it's\" (\"it is\") before \"a\"." },
    ContextualPattern { needle: "its an", target_sub: "its", replacement: "it's", message: "Use contraction \"it's\" (\"it is\") before \"an\"." },
    ContextualPattern { needle: "better then", target_sub: "then", replacement: "than", message: "Use \"than\" for comparisons after \"better\"." },
    ContextualPattern { needle: "more then", target_sub: "then", replacement: "than", message: "Use \"than\" for comparisons after \"more\"." },
    ContextualPattern { needle: "less then", target_sub: "then", replacement: "than", message: "Use \"than\" for comparisons after \"less\"." },
    ContextualPattern { needle: "rather then", target_sub: "then", replacement: "than", message: "Use \"than\" after \"rather\"." },
    ContextualPattern { needle: "other then", target_sub: "then", replacement: "than", message: "Use \"than\" after \"other\"." },
    ContextualPattern { needle: "greater then", target_sub: "then", replacement: "than", message: "Use \"than\" for comparisons after \"greater\"." },
    ContextualPattern { needle: "the affect", target_sub: "affect", replacement: "effect", message: "Use noun \"effect\" after \"the\"." },
    ContextualPattern { needle: "an affect", target_sub: "affect", replacement: "effect", message: "Use noun \"effect\" after \"an\"." },
    ContextualPattern { needle: "side affect", target_sub: "affect", replacement: "effect", message: "Use noun \"effect\" in \"side effect\"." },
    ContextualPattern {
        needle: "cause and affect",
        target_sub: "affect",
        replacement: "effect",
        message: "Use noun \"effect\" in \"cause and effect\".",
    },
];

const PT_PATTERNS: &[ContextualPattern] = &[
    ContextualPattern {
        needle: "cada vez mas",
        target_sub: "mas",
        replacement: "mais",
        message: "Use o advérbio de intensidade \"mais\" em \"cada vez mais\".",
    },
    ContextualPattern {
        needle: "muito mas",
        target_sub: "mas",
        replacement: "mais",
        message: "Use \"mais\" (intensidade ou quantidade) após \"muito\".",
    },
    ContextualPattern { needle: "bem mas", target_sub: "mas", replacement: "mais", message: "Use \"mais\" (intensidade) após \"bem\"." },
    ContextualPattern {
        needle: "o mas importante",
        target_sub: "mas",
        replacement: "mais",
        message: "Use \"mais\" antes do adjetivo \"importante\".",
    },
    ContextualPattern {
        needle: "mas rápido",
        target_sub: "mas",
        replacement: "mais",
        message: "Use \"mais\" para comparação ou intensidade antes de \"rápido\".",
    },
    ContextualPattern {
        needle: "mas fácil",
        target_sub: "mas",
        replacement: "mais",
        message: "Use \"mais\" para comparação ou intensidade antes de \"fácil\".",
    },
    ContextualPattern { needle: "mas tarde", target_sub: "mas", replacement: "mais", message: "Use \"mais\" na expressão \"mais tarde\"." },
    ContextualPattern { needle: "mas cedo", target_sub: "mas", replacement: "mais", message: "Use \"mais\" na expressão \"mais cedo\"." },
    ContextualPattern {
        needle: "mal humor",
        target_sub: "mal",
        replacement: "mau",
        message: "Use o adjetivo \"mau\" (oposto de \"bom\") antes de \"humor\".",
    },
    ContextualPattern {
        needle: "mal caráter",
        target_sub: "mal",
        replacement: "mau",
        message: "Use o adjetivo \"mau\" (oposto de \"bom\") antes de \"caráter\".",
    },
    ContextualPattern {
        needle: "mau feito",
        target_sub: "mau",
        replacement: "mal",
        message: "Use o advérbio \"mal\" (oposto de \"bem\") antes do particípio \"feito\".",
    },
    ContextualPattern {
        needle: "passou mau",
        target_sub: "mau",
        replacement: "mal",
        message: "Use o advérbio \"mal\" (oposto de \"bem\") com o verbo \"passar\".",
    },
    ContextualPattern {
        needle: "a anos",
        target_sub: "a",
        replacement: "há",
        message: "Use o verbo \"há\" para indicar tempo decorrido (\"há anos\").",
    },
    ContextualPattern {
        needle: "a meses",
        target_sub: "a",
        replacement: "há",
        message: "Use o verbo \"há\" para indicar tempo decorrido (\"há meses\").",
    },
    ContextualPattern {
        needle: "a dias",
        target_sub: "a",
        replacement: "há",
        message: "Use o verbo \"há\" para indicar tempo decorrido (\"há dias\").",
    },
    ContextualPattern {
        needle: "a muito tempo",
        target_sub: "a",
        replacement: "há",
        message: "Use \"há\" para indicar tempo transcorrido (\"há muito tempo\").",
    },
    ContextualPattern {
        needle: "daqui há", target_sub: "há", replacement: "a", message: "Para tempo futuro (\"daqui a\"), use a preposição \"a\"."
    },
    ContextualPattern {
        needle: "há partir de",
        target_sub: "há",
        replacement: "a",
        message: "Na locução \"a partir de\", use a preposição \"a\" sem \"h\".",
    },
    ContextualPattern {
        needle: "aonde fica",
        target_sub: "aonde",
        replacement: "onde",
        message: "Use \"onde\" com verbos estáticos como \"ficar\".",
    },
    ContextualPattern {
        needle: "aonde está",
        target_sub: "aonde",
        replacement: "onde",
        message: "Use \"onde\" com verbos estáticos como \"estar\".",
    },
    ContextualPattern {
        needle: "seção de cinema",
        target_sub: "seção",
        replacement: "sessão",
        message: "Use \"sessão\" para evento ou período de exibição.",
    },
    ContextualPattern {
        needle: "seção plenária", target_sub: "seção", replacement: "sessão", message: "Use \"sessão\" para reunião ou encontro."
    },
    ContextualPattern {
        needle: "sessão de livros",
        target_sub: "sessão",
        replacement: "seção",
        message: "Use \"seção\" para divisão ou departamento.",
    },
    ContextualPattern {
        needle: "sessão do artigo",
        target_sub: "sessão",
        replacement: "seção",
        message: "Use \"seção\" para parte ou subdivisão de um texto.",
    },
    ContextualPattern {
        needle: "sessão de direitos",
        target_sub: "sessão",
        replacement: "cessão",
        message: "Use \"cessão\" no sentido de ceder ou transferir direitos.",
    },
    ContextualPattern {
        needle: "seção de direitos",
        target_sub: "seção",
        replacement: "cessão",
        message: "Use \"cessão\" no sentido de ceder ou transferir direitos.",
    },
    ContextualPattern {
        needle: "o porque",
        target_sub: "porque",
        replacement: "porquê",
        message: "Substantivado após artigo (\"o porquê\"), escreve-se junto e com acento.",
    },
    ContextualPattern {
        needle: "um porque",
        target_sub: "porque",
        replacement: "porquê",
        message: "Substantivado após artigo (\"um porquê\"), escreve-se junto e com acento.",
    },
];

const ES_PATTERNS: &[ContextualPattern] = &[
    ContextualPattern {
        needle: "haber si",
        target_sub: "haber",
        replacement: "a ver",
        message: "Use la locución \"a ver\" (mirar o comprobar) antes de \"si\".",
    },
    ContextualPattern {
        needle: "haber qué",
        target_sub: "haber",
        replacement: "a ver",
        message: "Use \"a ver qué\" para expresar expectativa o comprobación.",
    },
    ContextualPattern { needle: "debe a ver", target_sub: "a ver", replacement: "haber", message: "Use el verbo \"haber\" tras \"debe\"." },
    ContextualPattern { needle: "puede a ver", target_sub: "a ver", replacement: "haber", message: "Use el verbo \"haber\" tras \"puede\"." },
    ContextualPattern {
        needle: "por hay",
        target_sub: "hay",
        replacement: "ahí",
        message: "Use el adverbio de lugar \"ahí\" en la expresión \"por ahí\".",
    },
    ContextualPattern { needle: "de hay", target_sub: "hay", replacement: "ahí", message: "Use el adverbio \"ahí\" en \"de ahí\"." },
    ContextualPattern {
        needle: "ahí que",
        target_sub: "ahí",
        replacement: "hay",
        message: "Use la forma verbal \"hay\" en la perífrasis \"hay que\".",
    },
    ContextualPattern { needle: "ahí muchos", target_sub: "ahí", replacement: "hay", message: "Use el verbo \"hay\" (haber) antes de \"muchos\"." },
    ContextualPattern { needle: "ahí muchas", target_sub: "ahí", replacement: "hay", message: "Use el verbo \"hay\" (haber) antes de \"muchas\"." },
    ContextualPattern {
        needle: "¡hay!",
        target_sub: "hay",
        replacement: "ay",
        message: "Use la interjección \"ay\" para expresar sorpresa o dolor.",
    },
    ContextualPattern {
        needle: "ha echo",
        target_sub: "echo",
        replacement: "hecho",
        message: "Use el participio \"hecho\" (del verbo \"hacer\") con el auxiliar \"ha\".",
    },
    ContextualPattern {
        needle: "he echo",
        target_sub: "echo",
        replacement: "hecho",
        message: "Use el participio \"hecho\" (del verbo \"hacer\") con el auxiliar \"he\".",
    },
    ContextualPattern {
        needle: "han echo",
        target_sub: "echo",
        replacement: "hecho",
        message: "Use el participio \"hecho\" (del verbo \"hacer\") con el auxiliar \"han\".",
    },
    ContextualPattern {
        needle: "hemos echo",
        target_sub: "echo",
        replacement: "hecho",
        message: "Use el participio \"hecho\" (del verbo \"hacer\") con \"hemos\".",
    },
    ContextualPattern { needle: "fue echo", target_sub: "echo", replacement: "hecho", message: "Use el participio \"hecho\" tras \"fue\"." },
    ContextualPattern { needle: "está echo", target_sub: "echo", replacement: "hecho", message: "Use el participio \"hecho\" tras \"está\"." },
    ContextualPattern {
        needle: "hecho de menos",
        target_sub: "hecho",
        replacement: "echo",
        message: "La expresión \"echar de menos\" usa \"echo\" (sin \"h\").",
    },
    ContextualPattern { needle: "una ves", target_sub: "ves", replacement: "vez", message: "Use el sustantivo \"vez\" en \"una vez\"." },
    ContextualPattern { needle: "otra ves", target_sub: "ves", replacement: "vez", message: "Use el sustantivo \"vez\" en \"otra vez\"." },
    ContextualPattern { needle: "cada ves", target_sub: "ves", replacement: "vez", message: "Use el sustantivo \"vez\" en \"cada vez\"." },
    ContextualPattern { needle: "tal ves", target_sub: "ves", replacement: "vez", message: "Use el sustantivo \"vez\" en \"tal vez\"." },
    ContextualPattern { needle: "en ves de", target_sub: "ves", replacement: "vez", message: "Use el sustantivo \"vez\" en \"en vez de\"." },
    ContextualPattern { needle: "si lo vez", target_sub: "vez", replacement: "ves", message: "Use el verbo \"ves\" (de \"ver\") tras \"si lo\"." },
    ContextualPattern { needle: "como vez", target_sub: "vez", replacement: "ves", message: "Use el verbo \"ves\" (de \"ver\") en \"como ves\"." },
    ContextualPattern {
        needle: "ojalá halla",
        target_sub: "halla",
        replacement: "haya",
        message: "Use \"haya\" (subjuntivo de \"haber\") después de \"ojalá\".",
    },
    ContextualPattern {
        needle: "espero que halla",
        target_sub: "halla",
        replacement: "haya",
        message: "Use \"haya\" (de \"haber\") en \"espero que haya\".",
    },
    ContextualPattern {
        needle: "aunque halla",
        target_sub: "halla",
        replacement: "haya",
        message: "Use \"haya\" (de \"haber\") después de \"aunque\".",
    },
    ContextualPattern {
        needle: "se haya en",
        target_sub: "haya",
        replacement: "halla",
        message: "Use \"halla\" (del verbo \"hallar\", encontrarse) en \"se halla en\".",
    },
    ContextualPattern {
        needle: "se haya ubicado",
        target_sub: "haya",
        replacement: "halla",
        message: "Use \"halla\" (del verbo \"hallar\") en \"se halla ubicado\".",
    },
    ContextualPattern {
        needle: "si no que",
        target_sub: "si no",
        replacement: "sino",
        message: "Use la conjunción adversativa \"sino\" en \"sino que\".",
    },
    ContextualPattern {
        needle: "sino vienes",
        target_sub: "sino",
        replacement: "si no",
        message: "Use \"si no\" (condicional + negación) antes de \"vienes\".",
    },
    ContextualPattern {
        needle: "sino llueve",
        target_sub: "sino",
        replacement: "si no",
        message: "Use \"si no\" (condicional + negación) antes de \"llueve\".",
    },
    ContextualPattern {
        needle: "sino puedes",
        target_sub: "sino",
        replacement: "si no",
        message: "Use \"si no\" (condicional + negación) antes de \"puedes\".",
    },
    ContextualPattern {
        needle: "¿porque",
        target_sub: "porque",
        replacement: "por qué",
        message: "En preguntas directas, use \"por qué\" separado y con tilde.",
    },
    ContextualPattern {
        needle: "no sé porque",
        target_sub: "porque",
        replacement: "por qué",
        message: "En interrogativas indirectas (\"no sé por qué\"), use \"por qué\".",
    },
    ContextualPattern {
        needle: "dime porque",
        target_sub: "porque",
        replacement: "por qué",
        message: "En interrogativas indirectas (\"dime por qué\"), use \"por qué\".",
    },
];

const EN_REDUNDANCIES: &[(&str, &str)] = &[
    ("due to the fact that", "because"),
    ("at this point in time", "now"),
    ("for the purpose of", "for"),
    ("each and every", "each"),
    ("in order to", "to"),
    ("very unique", "unique"),
];

const PT_REDUNDANCIES: &[(&str, &str)] = &[
    ("devido ao fato de que", "porque"),
    ("com o intuito de", "para"),
    ("entrar para dentro", "entrar"),
    ("sair para fora", "sair"),
    ("elo de ligação", "elo"),
    ("há anos atrás", "há anos"),
    ("subir para cima", "subir"),
    ("descer para baixo", "descer"),
];

const ES_REDUNDANCIES: &[(&str, &str)] =
    &[("subir para arriba", "subir"), ("bajar para abajo", "bajar"), ("entrar para adentro", "entrar"), ("salir para afuera", "salir")];

/// Runs conservative delimiter/quote, spacing, contextual paronym, and
/// redundancy checks for English, Portuguese, and Spanish text.
pub(crate) fn check(text: &str, lang: ProofLang) -> Vec<Issue> {
    let Some(group) = lang_group(lang) else {
        return Vec::new();
    };
    if text.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    check_balanced_delimiters(text, group, lang, &mut out);
    check_balanced_straight_double_quotes(text, group, lang, &mut out);
    check_bracket_spacing(text, group, lang, &mut out);
    check_missing_space_after_comma_semicolon(text, group, lang, &mut out);

    let folded = FoldedText::new(text);
    check_contextual(text, &folded, group, lang, &mut out);
    check_redundancies(text, &folded, group, lang, &mut out);

    out.sort_by_key(|i| (i.start, i.end));
    out
}

/// Lowercased text paired with a map back to valid UTF-8 byte boundaries in the
/// original string so arbitrary Unicode input never breaks byte slicing.
struct FoldedText {
    lower: String,
    orig_bounds: Vec<usize>,
}

impl FoldedText {
    fn new(text: &str) -> Self {
        let mut lower = String::with_capacity(text.len());
        let mut orig_bounds = Vec::with_capacity(text.len() + 1);
        for (orig_start, ch) in text.char_indices() {
            let mut first = true;
            for lc in ch.to_lowercase() {
                let mut buf = [0u8; 4];
                let s = lc.encode_utf8(&mut buf);
                lower.push_str(s);
                for _ in 0..s.len() {
                    if first {
                        orig_bounds.push(orig_start);
                        first = false;
                    } else {
                        orig_bounds.push(usize::MAX);
                    }
                }
            }
        }
        orig_bounds.push(text.len());
        Self { lower, orig_bounds }
    }

    fn map_range(&self, lower_start: usize, lower_end: usize) -> Option<(usize, usize)> {
        let orig_start = *self.orig_bounds.get(lower_start)?;
        let orig_end = *self.orig_bounds.get(lower_end)?;
        (orig_start != usize::MAX && orig_end != usize::MAX && orig_start <= orig_end).then_some((orig_start, orig_end))
    }
}

fn check_balanced_delimiters(text: &str, group: LangGroup, lang: ProofLang, out: &mut Vec<Issue>) {
    let mut stack: Vec<(char, usize)> = Vec::new();
    let mut in_backticks = false;

    for (idx, ch) in text.char_indices() {
        if ch == '`' {
            in_backticks = !in_backticks;
            continue;
        }
        if in_backticks || inside_url(text, idx) {
            continue;
        }
        if matches!(ch, '(' | ')') && is_emoticon_paren(text, idx) {
            continue;
        }
        if matches!(ch, '(' | '[' | '“' | '«') {
            stack.push((ch, idx));
            continue;
        }
        let expected_open = match ch {
            ')' => Some('('),
            ']' => Some('['),
            '”' => Some('“'),
            '»' => Some('«'),
            _ => None,
        };
        let Some(open) = expected_open else {
            continue;
        };
        if ch == ')' && is_list_marker_close(text, idx) {
            continue;
        }
        if let Some(pos) = stack.iter().rposition(|(c, _)| *c == open) {
            for (unclosed, u_idx) in stack.drain((pos + 1)..) {
                out.push(unclosed_issue(unclosed, u_idx, group, lang));
            }
            stack.pop();
        } else {
            out.push(unmatched_close_issue(ch, idx, group, lang));
        }
    }

    for (unclosed, idx) in stack {
        out.push(unclosed_issue(unclosed, idx, group, lang));
    }
}

fn is_emoticon_paren(text: &str, idx: usize) -> bool {
    let Some(before) = text.get(..idx) else {
        return false;
    };
    for prefix in [":-", ";-", ":", ";", "="] {
        if let Some(rest) = before.strip_suffix(prefix) {
            return rest.chars().next_back().is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '['));
        }
    }
    false
}

fn is_list_marker_close(text: &str, close_idx: usize) -> bool {
    let Some(before) = text.get(..close_idx) else {
        return false;
    };
    let line_start = before.rfind('\n').map_or(0, |n| n + 1);
    let prefix = before.get(line_start..).unwrap_or("").trim_start();
    let len = prefix.len();
    (1..=3).contains(&len) && (prefix.chars().all(|c| c.is_ascii_digit()) || len == 1 && prefix.chars().all(|c| c.is_ascii_alphabetic()))
}

fn unclosed_issue(ch: char, idx: usize, group: LangGroup, lang: ProofLang) -> Issue {
    let message = match group {
        LangGroup::Pt => format!("Delimitador \"{ch}\" aberto sem fechamento correspondente."),
        LangGroup::Es => format!("Se abrió \"{ch}\" sin un delimitador de cierre correspondiente."),
        LangGroup::En => format!("Opening \"{ch}\" does not have a matching closing delimiter."),
    };
    Issue { start: idx, end: idx.saturating_add(ch.len_utf8()), kind: IssueKind::Grammar, message, suggestions: Vec::new(), lang }
}

fn unmatched_close_issue(ch: char, idx: usize, group: LangGroup, lang: ProofLang) -> Issue {
    let message = match group {
        LangGroup::Pt => format!("Delimitador \"{ch}\" sem abertura correspondente."),
        LangGroup::Es => format!("El delimitador \"{ch}\" aparece sin apertura correspondiente."),
        LangGroup::En => format!("Closing \"{ch}\" does not have a matching opening delimiter."),
    };
    Issue { start: idx, end: idx.saturating_add(ch.len_utf8()), kind: IssueKind::Grammar, message, suggestions: Vec::new(), lang }
}

fn check_balanced_straight_double_quotes(text: &str, group: LangGroup, lang: ProofLang, out: &mut Vec<Issue>) {
    let mut in_backticks = false;
    let mut positions = Vec::new();
    for (idx, ch) in text.char_indices() {
        if ch == '`' {
            in_backticks = !in_backticks;
            continue;
        }
        if in_backticks || ch != '"' || inside_url(text, idx) {
            continue;
        }
        let prev = text.get(..idx).and_then(|s| s.chars().next_back());
        let next = text.get(idx + 1..).and_then(|s| s.chars().next());
        let is_inch_mark = prev.is_some_and(|c| c.is_ascii_digit()) && !next.is_some_and(char::is_alphabetic);
        if !is_inch_mark {
            positions.push(idx);
        }
    }

    if positions.len() % 2 == 1
        && let Some(&last_idx) = positions.last()
    {
        let message = match group {
            LangGroup::Pt => "Há uma aspa dupla sem par correspondente neste parágrafo.".to_owned(),
            LangGroup::Es => "Falta una comilla doble de cierre o apertura en este párrafo.".to_owned(),
            LangGroup::En => "Unmatched straight double quotation mark in this paragraph.".to_owned(),
        };
        out.push(Issue { start: last_idx, end: last_idx.saturating_add(1), kind: IssueKind::Grammar, message, suggestions: Vec::new(), lang });
    }
}

fn check_bracket_spacing(text: &str, group: LangGroup, lang: ProofLang, out: &mut Vec<Issue>) {
    let mut in_backticks = false;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for i in 0..chars.len() {
        let (idx, ch) = chars[i];
        if ch == '`' {
            in_backticks = !in_backticks;
            continue;
        }
        if in_backticks || inside_url(text, idx) {
            continue;
        }
        if matches!(ch, '(' | '[')
            && let Some(&(sp_idx, ' ')) = chars.get(i + 1)
            && let Some(&(_, after_sp)) = chars.get(i + 2)
            && !after_sp.is_whitespace()
        {
            let message = match group {
                LangGroup::Pt => format!("Remova o espaço extra logo após \"{ch}\"."),
                LangGroup::Es => format!("Elimine el espacio extra después de \"{ch}\"."),
                LangGroup::En => format!("Remove the extra space immediately after \"{ch}\"."),
            };
            out.push(Issue { start: idx, end: sp_idx.saturating_add(1), kind: IssueKind::Grammar, message, suggestions: vec![ch.to_string()], lang });
        }

        if matches!(ch, ')' | ']')
            && i >= 2
            && let (Some(&(sp_idx, ' ')), Some(&(_, before_sp))) = (chars.get(i - 1), chars.get(i - 2))
            && !before_sp.is_whitespace()
            && !matches!(before_sp, '(' | '[')
        {
            let message = match group {
                LangGroup::Pt => format!("Remova o espaço extra antes de \"{ch}\"."),
                LangGroup::Es => format!("Elimine el espacio extra antes de \"{ch}\"."),
                LangGroup::En => format!("Remove the extra space immediately before \"{ch}\"."),
            };
            out.push(Issue { start: sp_idx, end: idx.saturating_add(1), kind: IssueKind::Grammar, message, suggestions: vec![ch.to_string()], lang });
        }
    }
}

fn check_missing_space_after_comma_semicolon(text: &str, group: LangGroup, lang: ProofLang, out: &mut Vec<Issue>) {
    let mut in_backticks = false;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for i in 1..chars.len().saturating_sub(1) {
        let (idx, ch) = chars[i];
        if ch == '`' {
            in_backticks = !in_backticks;
            continue;
        }
        if in_backticks || !matches!(ch, ',' | ';') {
            continue;
        }
        let prev = chars[i - 1].1;
        let next = chars[i + 1].1;
        if (prev.is_alphabetic() || matches!(prev, ')' | ']' | '”' | '»')) && next.is_alphabetic() && !inside_url(text, idx) {
            let message = match group {
                LangGroup::Pt => format!("Insira um espaço após \"{ch}\"."),
                LangGroup::Es => format!("Agregue un espacio después de \"{ch}\"."),
                LangGroup::En => format!("Add a space after \"{ch}\"."),
            };
            out.push(Issue { start: idx, end: idx.saturating_add(1), kind: IssueKind::Grammar, message, suggestions: vec![format!("{ch} ")], lang });
        }
    }
}

fn inside_url(text: &str, idx: usize) -> bool {
    let Some(before) = text.get(..idx) else {
        return false;
    };
    let token_start = before.rfind(char::is_whitespace).map_or(0, |p| p + 1);
    let after = text.get(idx..).unwrap_or("");
    let token_end = after.find(char::is_whitespace).map_or(text.len(), |p| idx + p);
    let token = text.get(token_start..token_end).unwrap_or("");
    token.contains("://") || token.starts_with("www.")
}

fn check_contextual(text: &str, folded: &FoldedText, group: LangGroup, lang: ProofLang, out: &mut Vec<Issue>) {
    let patterns = match group {
        LangGroup::En => EN_PATTERNS,
        LangGroup::Pt => PT_PATTERNS,
        LangGroup::Es => ES_PATTERNS,
    };
    for pattern in patterns {
        let mut cursor = 0usize;
        while let Some(rel) = folded.lower.get(cursor..).and_then(|s| s.find(pattern.needle)) {
            let start = cursor + rel;
            let end = start + pattern.needle.len();
            if has_word_boundaries(&folded.lower, start, end)
                && let Some(sub_rel) = pattern.needle.find(pattern.target_sub)
            {
                let sub_start = start + sub_rel;
                let sub_end = sub_start + pattern.target_sub.len();
                if let Some((orig_start, orig_end)) = folded.map_range(sub_start, sub_end)
                    && let Some(orig) = text.get(orig_start..orig_end)
                {
                    let suggestion = match_case(orig, pattern.replacement);
                    out.push(Issue {
                        start: orig_start,
                        end: orig_end,
                        kind: IssueKind::Grammar,
                        message: pattern.message.to_owned(),
                        suggestions: vec![suggestion],
                        lang,
                    });
                }
            }
            cursor = end;
        }
    }
    if group == LangGroup::Pt {
        check_pt_end_of_question(text, folded, lang, out);
    }
}

fn check_pt_end_of_question(text: &str, folded: &FoldedText, lang: ProofLang, out: &mut Vec<Issue>) {
    for (needle, target_sub) in [("porque?", "porque"), ("por que?", "por que")] {
        let mut cursor = 0usize;
        while let Some(rel) = folded.lower.get(cursor..).and_then(|s| s.find(needle)) {
            let start = cursor + rel;
            let target_end = start + target_sub.len();
            if left_boundary(&folded.lower, start)
                && let Some((orig_start, orig_end)) = folded.map_range(start, target_end)
                && let Some(orig) = text.get(orig_start..orig_end)
            {
                let suggestion = match_case(orig, "por quê");
                out.push(Issue {
                    start: orig_start,
                    end: orig_end,
                    kind: IssueKind::Grammar,
                    message: "Em fim de frase antes de ponto de interrogação, use \"por quê\".".to_owned(),
                    suggestions: vec![suggestion],
                    lang,
                });
            }
            cursor = start + needle.len();
        }
    }
}

fn check_redundancies(text: &str, folded: &FoldedText, group: LangGroup, lang: ProofLang, out: &mut Vec<Issue>) {
    let table = match group {
        LangGroup::En => EN_REDUNDANCIES,
        LangGroup::Pt => PT_REDUNDANCIES,
        LangGroup::Es => ES_REDUNDANCIES,
    };
    for &(phrase, replacement) in table {
        let mut cursor = 0usize;
        while let Some(rel) = folded.lower.get(cursor..).and_then(|s| s.find(phrase)) {
            let start = cursor + rel;
            let end = start + phrase.len();
            if has_word_boundaries(&folded.lower, start, end)
                && let Some((orig_start, orig_end)) = folded.map_range(start, end)
                && let Some(orig) = text.get(orig_start..orig_end)
            {
                let suggestion = match_case(orig, replacement);
                let message = match group {
                    LangGroup::Pt => format!("Expressão redundante; prefira \"{suggestion}\"."),
                    LangGroup::Es => format!("Expresión redundante; prefiera \"{suggestion}\"."),
                    LangGroup::En => format!("Wordy phrasing; consider \"{suggestion}\"."),
                };
                out.push(Issue { start: orig_start, end: orig_end, kind: IssueKind::Grammar, message, suggestions: vec![suggestion], lang });
            }
            cursor = end;
        }
    }
}

fn left_boundary(text: &str, start: usize) -> bool {
    text.get(..start).and_then(|s| s.chars().next_back()).is_none_or(|c| !c.is_alphanumeric())
}

fn has_word_boundaries(text: &str, start: usize, end: usize) -> bool {
    let left_ok = left_boundary(text, start);
    let right_ok = text.get(end..).and_then(|s| s.chars().next()).is_none_or(|c| !c.is_alphanumeric());
    left_ok && right_ok
}

fn match_case(original: &str, replacement: &str) -> String {
    if original.chars().next().is_some_and(char::is_uppercase) {
        let mut chars = replacement.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => String::new(),
        }
    } else {
        replacement.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Issue, check_grammar_in, check_text};

    fn spans<'a>(text: &'a str, issues: &[Issue]) -> Vec<(&'a str, Vec<String>)> {
        issues.iter().map(|i| (&text[i.start..i.end], i.suggestions.clone())).collect()
    }

    #[test]
    fn delimiter_quote_and_spacing_checks_across_en_pt_es() {
        let en = check_grammar_in("Review the (unfinished note.", ProofLang::En);
        assert!(en.iter().any(|i| i.kind == IssueKind::Grammar && i.message.contains("matching closing")), "{en:?}");

        let pt = check_grammar_in("Ele disse “olá sem fechar.", ProofLang::Pt);
        assert!(pt.iter().any(|i| i.message.contains("fechamento")), "{pt:?}");

        let es = check_grammar_in("Cita «incompleta en español.", ProofLang::Es);
        assert!(es.iter().any(|i| i.message.contains("cierre")), "{es:?}");

        let q = check_grammar_in(r#"She wrote "open quote only."#, ProofLang::En);
        assert!(q.iter().any(|i| i.message.contains("quotation mark")), "{q:?}");

        let spaced = check_grammar_in("Check ( this item ).", ProofLang::En);
        assert_eq!(spans("Check ( this item ).", &spaced), [("( ", vec!["(".into()]), (" )", vec![")".into()])]);

        let comma = check_grammar_in("Olá,mundo;teste.", ProofLang::Pt);
        assert_eq!(spans("Olá,mundo;teste.", &comma), [(",", vec![", ".into()]), (";", vec!["; ".into()])]);
    }

    #[test]
    fn suppresses_false_positives_on_inches_emoticons_urls_numbers_lists_and_code() {
        for (sample, lang) in [
            ("Don't forget O'Brien's 27\" monitor (verified [ok]) :).", ProofLang::En),
            ("1) First item\na) Second item\nThe number is 1,000 and 3.14.", ProofLang::En),
            ("See https://example.com/a,b?x=(1 and `(unclosed` inline code.", ProofLang::En),
            ("Números como 10,5 ou 2,340 estão corretos «com aspas» ;).", ProofLang::Pt),
            ("El valor 1,500 cumple «con estilo» (sin errores) :-).", ProofLang::Es),
        ] {
            let found = check(sample, lang);
            assert!(found.is_empty(), "expected no false positives for {sample:?}, got {found:?}");
        }
    }

    #[test]
    fn contextual_paronyms_across_en_pt_es() {
        let en_text = "Their is a better then average team on it's own, and your welcome.";
        let en = check_grammar_in(en_text, ProofLang::En);
        assert_eq!(
            spans(en_text, &en),
            [("Their", vec!["There".into()]), ("then", vec!["than".into()]), ("it's", vec!["its".into()]), ("your", vec!["you're".into()]),]
        );

        let pt_text = "Ficou cada vez mas claro a anos onde estava, mas não sei o porque. Ele foi porque?";
        let pt = check_grammar_in(pt_text, ProofLang::Pt);
        assert_eq!(
            spans(pt_text, &pt),
            [("mas", vec!["mais".into()]), ("a", vec!["há".into()]), ("porque", vec!["porquê".into()]), ("porque", vec!["por quê".into()]),]
        );

        let es_text = "Haber si ha echo otra ves lo que dijo, sino puedes esperar y dime porque.";
        let es = check_grammar_in(es_text, ProofLang::Es);
        assert_eq!(
            spans(es_text, &es),
            [
                ("Haber", vec!["A ver".into()]),
                ("echo", vec!["hecho".into()]),
                ("ves", vec!["vez".into()]),
                ("sino", vec!["si no".into()]),
                ("porque", vec!["por qué".into()]),
            ]
        );
    }

    #[test]
    fn redundancies_en_and_pt() {
        let en_text = "In order to finish due to the fact that we need it.";
        let en = check_grammar_in(en_text, ProofLang::En);
        assert_eq!(spans(en_text, &en), [("In order to", vec!["To".into()]), ("due to the fact that", vec!["because".into()])]);

        let pt_text = "Vamos subir para cima para fortalecer o elo de ligação.";
        let pt = check_grammar_in(pt_text, ProofLang::Pt);
        assert_eq!(spans(pt_text, &pt), [("subir para cima", vec!["subir".into()]), ("elo de ligação", vec!["elo".into()])]);
    }

    #[test]
    fn preserves_dialect_and_avoids_overlap_with_builtin_rules() {
        let gb = check_text("Their is colour.", &[(0..16, ProofLang::EnGb)]);
        assert!(
            gb.iter().any(|i| i.kind == IssueKind::Grammar && i.lang == ProofLang::EnGb && &gb_slice("Their is colour.", i) == "Their"),
            "{gb:?}"
        );

        // Built-in space-before-punctuation (" ,") overlaps with ",b", so only one mark is emitted on that span.
        let pt = check_grammar_in("Olá ,mundo.", ProofLang::Pt);
        let comma_marks: Vec<_> = pt.iter().filter(|i| i.start <= 5 && i.end >= 4).collect();
        assert_eq!(comma_marks.len(), 1, "{pt:?}");
    }

    fn gb_slice(s: &str, i: &Issue) -> String {
        s[i.start..i.end].to_string()
    }
}
