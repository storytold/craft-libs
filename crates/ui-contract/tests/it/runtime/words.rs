//! The words of the workspace's crates: one id carries one text, wherever the `words!` tables sit.

use craft_ui::testing::trial;
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// The `"id" = "text"` rows of a `words!` table, in token order. A `context` row's first string is a context, not
/// an id, a `plain` row has no id, and a row with an empty id is looked up by its English text.
fn rows(tokens: TokenStream, found: &mut Vec<(String, String)>) {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    for (at, tree) in trees.iter().enumerate() {
        let TokenTree::Literal(id) = tree else { continue };
        let (Some(TokenTree::Punct(equals)), Some(TokenTree::Literal(text))) = (trees.get(at + 1), trees.get(at + 2)) else { continue };
        let after_context = at > 0 && matches!(&trees[at - 1], TokenTree::Ident(word) if word == "context");
        if equals.as_char() != '=' || after_context || id.to_string() == "\"\"" {
            continue;
        }
        if let (Ok(id), Ok(text)) = (syn::parse_str::<syn::LitStr>(&id.to_string()), syn::parse_str::<syn::LitStr>(&text.to_string())) {
            found.push((id.value(), text.value()));
        }
    }
}

fn tables(tokens: TokenStream, found: &mut Vec<(String, String)>) {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    for (at, tree) in trees.iter().enumerate() {
        let TokenTree::Group(group) = tree else { continue };
        let is_words = at >= 2
            && matches!(&trees[at - 1], TokenTree::Punct(bang) if bang.as_char() == '!')
            && matches!(&trees[at - 2], TokenTree::Ident(name) if name == "words");
        if is_words && group.delimiter() == Delimiter::Brace {
            collect_rows(group.stream(), found);
        } else {
            tables(group.stream(), found);
        }
    }
}

/// Rows at every depth of a table: an array row nests its `"id" = "text"` pairs in brackets.
fn collect_rows(tokens: TokenStream, found: &mut Vec<(String, String)>) {
    rows(tokens.clone(), found);
    for tree in tokens {
        if let TokenTree::Group(group) = tree {
            collect_rows(group.stream(), found);
        }
    }
}

fn sources(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

#[trial]
fn no_word_id_carries_two_texts() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for entry in fs::read_dir(&crates).expect("the crates directory").flatten() {
        sources(&entry.path().join("src"), &mut files);
    }
    let mut texts: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut total = 0;
    for file in &files {
        let Ok(source) = fs::read_to_string(file) else { continue };
        if !source.contains("words!") {
            continue;
        }
        let Ok(stream) = source.parse::<TokenStream>() else { continue };
        let mut found = Vec::new();
        tables(stream, &mut found);
        total += found.len();
        for (id, text) in found {
            texts.entry(id).or_default().insert(text, file.display().to_string());
        }
    }
    assert!(total > 500, "the scan reads the words tables; found {total} rows");
    let clashes: Vec<String> = texts
        .iter()
        .filter(|(_, by_text)| by_text.len() > 1)
        .map(|(id, by_text)| format!("`{id}`: {}", by_text.iter().map(|(text, file)| format!("{text:?} in {file}")).collect::<Vec<_>>().join(", ")))
        .collect();
    assert!(clashes.is_empty(), "ids with two texts:\n{}", clashes.join("\n"));
}
