//! Searching the index: family names, category, language and script.

use craft_i18n::Folder;

use crate::index::{Category, Index, categories};

/// What the catalogue shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// Part of a family name, ignoring case and accents.
    pub text: String,
    pub category: Option<Category>,
    /// A position in [`Index::languages`].
    pub language: Option<usize>,
    /// An ISO 15924 code.
    pub script: Option<String>,
}

/// Search keys for one index, built once.
pub struct Search {
    folder: Folder,
    families: Vec<String>,
    /// Per language: its name, autonym and id, folded.
    languages: Vec<String>,
    /// Language positions sorted by name.
    languages_by_name: Vec<usize>,
    /// Script positions sorted by name.
    scripts_by_name: Vec<usize>,
}

impl Search {
    pub fn new(index: &Index) -> Search {
        let folder = Folder::new("en");
        let families = index.families().iter().map(|f| folder.key(&f.family)).collect();
        let languages =
            index.languages().iter().map(|l| folder.key(&format!("{}\n{}\n{}", l.name, l.autonym.as_deref().unwrap_or(""), l.id))).collect();
        let mut languages_by_name: Vec<usize> = (0..index.languages().len()).collect();
        languages_by_name.sort_by_cached_key(|&i| index.languages().get(i).map(|l| folder.key(&l.name)));
        let mut scripts_by_name: Vec<usize> = (0..index.scripts().len()).collect();
        scripts_by_name.sort_by_cached_key(|&i| index.scripts().get(i).map(|s| folder.key(&s.name)));
        Search { folder, families, languages, languages_by_name, scripts_by_name }
    }

    /// The families `query` selects, as positions in [`Index::families`], in index order.
    pub fn families(&self, index: &Index, query: &Query) -> Vec<usize> {
        let text = self.folder.key(query.text.trim());
        index
            .families()
            .iter()
            .enumerate()
            .filter(|(i, f)| {
                (text.is_empty() || self.families.get(*i).is_some_and(|key| key.contains(&text)))
                    && query.category.is_none_or(|c| categories(f).any(|fc| fc == c))
                    && query.language.is_none_or(|l| f.languages.contains(&l))
                    && query.script.as_ref().is_none_or(|s| f.scripts.contains(s))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Languages whose English name, autonym or id contains `text`, sorted by English name.
    pub fn languages(&self, text: &str) -> Vec<usize> {
        let text = self.folder.key(text.trim());
        self.languages_by_name.iter().copied().filter(|&i| self.languages.get(i).is_some_and(|key| key.contains(&text))).collect()
    }

    /// Every script, sorted by English name.
    pub fn scripts(&self) -> &[usize] {
        &self.scripts_by_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_combine() {
        let index = crate::index::bundled().unwrap();
        let search = Search::new(&index);
        let names = |q: &Query| search.families(&index, q).into_iter().map(|i| index.families()[i].family.clone()).collect::<Vec<_>>();

        let noto_jp = names(&Query { text: "noto sans jp".into(), ..Query::default() });
        assert_eq!(noto_jp, ["Noto Sans JP"]);

        let japanese = search.languages("日本語");
        let ja = *japanese.first().unwrap();
        assert_eq!(index.languages()[ja].id, "ja_Jpan");
        let serif_ja = names(&Query { category: Some(Category::Serif), language: Some(ja), ..Query::default() });
        assert!(serif_ja.contains(&"Noto Serif JP".to_owned()));
        assert!(!serif_ja.contains(&"Noto Sans JP".to_owned()));

        let arabic = names(&Query { script: Some("Arab".into()), ..Query::default() });
        assert!(arabic.contains(&"Amiri".to_owned()));
        assert!(!arabic.contains(&"Roboto".to_owned()));
    }
}
