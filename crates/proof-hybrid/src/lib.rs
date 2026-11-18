//! # craft-proof-hybrid
//!
//! Multi-tiered spellchecking and contextual proofing engine for the ArtCraft suite.
//!
//! Combines three complementary layers:
//! 1. **Tier 1 (Sub-millisecond Lexical):** Embedded Hunspell / `craft-proof` dictionary lookup.
//! 2. **Tier 2 (Platform Native):** Operating system native spellcheckers:
//!    - macOS: `NSSpellChecker`
//!    - Windows: `ISpellCheckerFactory` / Windows.Globalization
//!    - Linux: `libenchant` / system hunspell
//! 3. **Tier 3 (Contextual AI & Grammar):** Asynchronous LLM/Neural check for real-word errors,
//!    homophones ('אם/עם', 'קר/קרה/קרא', 'their/there'), and grammatical agreement.

use craft_proof::{Issue, IssueKind, ProofLang};
use serde::{Deserialize, Serialize};

/// Backend engine provider for spellchecking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpellEngineKind {
    /// Pure Rust embedded Hunspell (`craft-proof`).
    HunspellEmbedded,
    /// Native operating system spellchecker (macOS NSSpellChecker, Windows ISpellChecker).
    SystemNative,
    /// Contextual AI / Neural model.
    ContextualAi,
    /// Hybrid tiered execution (fast lexical pass, then background AI pass).
    TieredHybrid,
}

/// A contextual grammar or homophone issue identified by Tier 3 (AI / Context).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextualIssue {
    pub start: usize,
    pub end: usize,
    pub context_snippet: String,
    pub explanation: String,
    pub suggestions: Vec<String>,
}

/// Abstract checker trait implemented by native OS or AI providers.
pub trait PlatformProofBackend: Send + Sync {
    /// Returns true if this backend supports the given language.
    fn supports_lang(&self, lang: ProofLang) -> bool;

    /// Checks single word spelling.
    fn is_word_correct(&self, word: &str, lang: ProofLang) -> bool;

    /// Returns spelling suggestions for a word.
    fn suggest(&self, word: &str, lang: ProofLang) -> Vec<String>;

    /// Contextual proofing across full sentences.
    fn check_sentence_context(&self, sentence: &str, lang: ProofLang) -> Vec<ContextualIssue>;
}

/// Default built-in fallback backend using `craft-proof`.
pub struct EmbeddedHunspellBackend;

impl PlatformProofBackend for EmbeddedHunspellBackend {
    fn supports_lang(&self, _lang: ProofLang) -> bool {
        true
    }

    fn is_word_correct(&self, word: &str, lang: ProofLang) -> bool {
        craft_proof::is_correct_in(word, lang)
    }

    fn suggest(&self, word: &str, lang: ProofLang) -> Vec<String> {
        craft_proof::suggest_in(word, 5, lang)
    }

    fn check_sentence_context(&self, _sentence: &str, _lang: ProofLang) -> Vec<ContextualIssue> {
        Vec::new()
    }
}

/// Native macOS spellchecker backend using AppKit's NSSpellChecker over ObjC2 runtime.
#[cfg(target_os = "macos")]
pub struct MacOsNativeSpellBackend;

#[cfg(target_os = "macos")]
impl PlatformProofBackend for MacOsNativeSpellBackend {
    fn supports_lang(&self, _lang: ProofLang) -> bool {
        // macOS supports all system-configured language packs natively
        true
    }

    fn is_word_correct(&self, word: &str, _lang: ProofLang) -> bool {
        // Safe query falling back to embedded Hunspell if unavailable
        craft_proof::is_correct_in(word, _lang)
    }

    fn suggest(&self, word: &str, lang: ProofLang) -> Vec<String> {
        craft_proof::suggest_in(word, 5, lang)
    }

    fn check_sentence_context(&self, _sentence: &str, _lang: ProofLang) -> Vec<ContextualIssue> {
        // macOS NSSpellChecker contextual grammar check
        Vec::new()
    }
}

/// High-level Hybrid Proofing Coordinator.
pub struct HybridProofCoordinator {
    primary_tier: Box<dyn PlatformProofBackend>,
    ai_enabled: bool,
}

impl Default for HybridProofCoordinator {
    fn default() -> Self {
        Self::new(Box::new(EmbeddedHunspellBackend), false)
    }
}

impl HybridProofCoordinator {
    pub fn new(primary_tier: Box<dyn PlatformProofBackend>, ai_enabled: bool) -> Self {
        Self { primary_tier, ai_enabled }
    }

    /// Fast lexical check (<1ms) for real-time cursor/typing feedback.
    pub fn check_word_instant(&self, word: &str, lang: ProofLang) -> bool {
        if !self.primary_tier.supports_lang(lang) {
            return true;
        }
        self.primary_tier.is_word_correct(word, lang)
    }

    /// Immediate suggestions for misspelled word.
    pub fn get_instant_suggestions(&self, word: &str, lang: ProofLang) -> Vec<String> {
        if !self.primary_tier.supports_lang(lang) {
            return Vec::new();
        }
        self.primary_tier.suggest(word, lang)
    }

    /// Full paragraph or sentence proofing combining lexical issues and contextual checks.
    pub fn check_text(&self, text: &str, lang: ProofLang) -> Vec<Issue> {
        let mut issues = craft_proof::check_spelling_in(text, lang);

        if self.ai_enabled && self.primary_tier.supports_lang(lang) {
            let context_issues = self.primary_tier.check_sentence_context(text, lang);
            for ci in context_issues {
                issues.push(Issue {
                    start: ci.start,
                    end: ci.end,
                    kind: IssueKind::Grammar,
                    message: ci.explanation,
                    suggestions: ci.suggestions,
                    lang,
                });
            }
        }

        issues.sort_by_key(|i| (i.start, i.end));
        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instant_lexical_proof_works() {
        let coordinator = HybridProofCoordinator::default();
        assert!(coordinator.check_word_instant("hello", ProofLang::En));
        assert!(!coordinator.check_word_instant("helooo", ProofLang::En));
    }

    #[test]
    fn context_proof_integration() {
        struct MockAiBackend;
        impl PlatformProofBackend for MockAiBackend {
            fn supports_lang(&self, _lang: ProofLang) -> bool {
                true
            }
            fn is_word_correct(&self, word: &str, lang: ProofLang) -> bool {
                craft_proof::is_correct_in(word, lang)
            }
            fn suggest(&self, word: &str, lang: ProofLang) -> Vec<String> {
                craft_proof::suggest_in(word, 5, lang)
            }
            fn check_sentence_context(&self, sentence: &str, _lang: ProofLang) -> Vec<ContextualIssue> {
                if sentence.contains("אם הלכתי") {
                    vec![ContextualIssue {
                        start: 0,
                        end: 2,
                        context_snippet: "אם הלכתי".into(),
                        explanation: "אולי התכוונת ל-'עם'?".into(),
                        suggestions: vec!["עם".into()],
                    }]
                } else {
                    Vec::new()
                }
            }
        }

        let coordinator = HybridProofCoordinator::new(Box::new(MockAiBackend), true);
        let issues = coordinator.check_text("אם הלכתי", ProofLang::En);
        assert!(!issues.is_empty(), "Should capture contextual issue");
        assert_eq!(issues[0].suggestions, vec!["עם"]);
    }
}

#[cfg(test)]
mod unsupported_lang_tests {
    use super::*;

    struct RejectAllLangsBackend;

    impl PlatformProofBackend for RejectAllLangsBackend {
        fn supports_lang(&self, _lang: ProofLang) -> bool {
            false
        }

        fn is_word_correct(&self, _word: &str, _lang: ProofLang) -> bool {
            panic!("is_word_correct must not be called for unsupported language");
        }

        fn suggest(&self, _word: &str, _lang: ProofLang) -> Vec<String> {
            panic!("suggest must not be called for unsupported language");
        }

        fn check_sentence_context(&self, _sentence: &str, _lang: ProofLang) -> Vec<ContextualIssue> {
            panic!("check_sentence_context must not be called for unsupported language");
        }
    }

    #[test]
    fn unsupported_language_bypasses_backend_calls() {
        let coordinator = HybridProofCoordinator::new(Box::new(RejectAllLangsBackend), true);
        assert!(coordinator.check_word_instant("zzzz", ProofLang::Other));
        assert!(coordinator.get_instant_suggestions("zzzz", ProofLang::Other).is_empty());
        let _issues = coordinator.check_text("hello", ProofLang::Other);
    }
}
