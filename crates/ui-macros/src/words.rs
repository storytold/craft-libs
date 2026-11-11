//! `words!`: a table of built-in strings written as one struct and its `WordSet` impl.
//!
//! One row per field: `field: "module.id" = "English";`. Other row kinds start with a keyword: `template` (a
//! `{name}` template), `unit` and `count` (they contain `{n}`), `array` (one id and text per element), `context`
//! (a word translated only in its catalog context) and `plain` (a word with no id, looked up by its English text).

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Attribute, Ident, LitStr, Token, Visibility, bracketed};

enum Kind {
    Text,
    Template,
    Unit,
    Plain,
    Context(LitStr),
    Count(LitStr),
    Array(Vec<(LitStr, LitStr)>),
}

struct Row {
    attrs: Vec<Attribute>,
    field: Ident,
    kind: Kind,
    id: Option<LitStr>,
    english: Option<LitStr>,
}

pub(crate) struct Words {
    attrs: Vec<Attribute>,
    vis: Visibility,
    name: Ident,
    rows: Vec<Row>,
}

impl Parse for Words {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis = input.parse()?;
        input.parse::<Token![struct]>()?;
        let name = input.parse()?;
        let body;
        syn::braced!(body in input);
        let mut rows = Vec::new();
        while !body.is_empty() {
            rows.push(body.parse()?);
        }
        Ok(Words { attrs, vis, name, rows })
    }
}

impl Parse for Row {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let field: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let mut row = Row { attrs, field, kind: Kind::Text, id: None, english: None };
        if input.peek(LitStr) {
            row.id = Some(input.parse()?);
        } else {
            let keyword: Ident = input.parse()?;
            match keyword.to_string().as_str() {
                "template" => {
                    row.kind = Kind::Template;
                    row.id = Some(input.parse()?);
                }
                "unit" => {
                    row.kind = Kind::Unit;
                    row.id = Some(input.parse()?);
                }
                "plain" => row.kind = Kind::Plain,
                "context" => {
                    let context: LitStr = input.parse()?;
                    row.kind = Kind::Context(context);
                }
                "count" => {
                    row.id = Some(input.parse()?);
                    row.kind = Kind::Count(LitStr::new("", Span::call_site()));
                }
                "array" => {
                    let list;
                    bracketed!(list in input);
                    let mut elements = Vec::new();
                    while !list.is_empty() {
                        let id: LitStr = list.parse()?;
                        list.parse::<Token![=]>()?;
                        let english: LitStr = list.parse()?;
                        elements.push((id, english));
                        if !list.is_empty() {
                            list.parse::<Token![,]>()?;
                        }
                    }
                    row.kind = Kind::Array(elements);
                    input.parse::<Token![;]>()?;
                    return Ok(row);
                }
                _ => {
                    return Err(syn::Error::new(
                        keyword.span(),
                        "a row is `\"id\" = \"English\"`, or starts with `template`, `unit`, `count`, `array`, `context` or `plain`",
                    ));
                }
            }
        }
        input.parse::<Token![=]>()?;
        let english: LitStr = input.parse()?;
        if let Kind::Count(other) = &mut row.kind {
            input.parse::<Token![|]>()?;
            *other = input.parse()?;
        }
        row.english = Some(english);
        input.parse::<Token![;]>()?;
        Ok(row)
    }
}

/// `module.camelCase`.
fn valid_id(id: &str) -> bool {
    let camel = |part: &str| part.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && part.chars().all(|c| c.is_ascii_alphanumeric());
    id.split_once('.').is_some_and(|(module, name)| camel(module) && camel(name))
}

/// The names between braces, or the first unmatched brace.
fn placeholders(text: &str) -> Result<Vec<&str>, &'static str> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find(['{', '}']) {
        if rest.as_bytes()[open] == b'}' {
            return Err("an unmatched `}`");
        }
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { return Err("an unmatched `{`") };
        let name = &after[..close];
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("a placeholder is `{name}` in letters, digits and underscores");
        }
        names.push(name);
        rest = &after[close + 1..];
    }
    Ok(names)
}

fn check_id(id: &LitStr) -> syn::Result<()> {
    if valid_id(&id.value()) { Ok(()) } else { Err(syn::Error::new(id.span(), "a word id is `module.camelCase`")) }
}

fn check_text(text: &LitStr, needs_n: bool) -> syn::Result<()> {
    let value = text.value();
    let names = placeholders(&value).map_err(|message| syn::Error::new(text.span(), message))?;
    if needs_n && !names.contains(&"n") {
        return Err(syn::Error::new(text.span(), "a unit or count row contains `{n}`"));
    }
    Ok(())
}

/// What one row contributes: its field type, the words it registers, its English and translated values and
/// its rows.
struct Expanded {
    ty: TokenStream,
    words: Vec<TokenStream>,
    counts: Vec<TokenStream>,
    english: TokenStream,
    localized: TokenStream,
    rows: TokenStream,
}

const ENV: &str = "::craft_ui::runtime::env";

fn expand(row: &Row) -> syn::Result<Expanded> {
    let env: TokenStream = ENV.parse().unwrap_or_default();
    let field = &row.field;
    let word = |row: &Row| -> syn::Result<TokenStream> {
        let english = row.english.clone().ok_or_else(|| syn::Error::new(row.field.span(), "a row has its English text"))?;
        Ok(match (&row.kind, &row.id) {
            (Kind::Plain, _) => quote!(#env::Word::plain(#english)),
            (Kind::Context(context), _) => quote!(#env::Word::in_context(#context, #english)),
            (_, Some(id)) => quote!(#env::Word::keyed(#id, #english)),
            _ => return Err(syn::Error::new(row.field.span(), "a row has an id")),
        })
    };
    if let Some(id) = &row.id {
        check_id(id)?;
    }
    match &row.kind {
        Kind::Text | Kind::Plain | Kind::Context(_) => {
            let english = row.english.clone().unwrap_or_else(|| LitStr::new("", Span::call_site()));
            check_text(&english, false)?;
            let word = word(row)?;
            Ok(Expanded {
                ty: quote!(#env::Label),
                words: vec![word.clone()],
                counts: Vec::new(),
                english: quote!(#field: #env::Label::Static(#english)),
                localized: quote!(#field: #env::Label::from_cow(words.get(#word))),
                rows: quote!(.row(#word, self.#field.as_str())),
            })
        }
        Kind::Template | Kind::Unit => {
            let english = row.english.clone().unwrap_or_else(|| LitStr::new("", Span::call_site()));
            check_text(&english, matches!(row.kind, Kind::Unit))?;
            let word = word(row)?;
            Ok(Expanded {
                ty: quote!(#env::WordTemplate),
                words: vec![word.clone()],
                counts: Vec::new(),
                english: quote!(#field: #env::WordTemplate::new(#english)),
                localized: quote!(#field: #env::WordTemplate::new(words.get(#word).into_owned())),
                rows: quote!(.row(#word, self.#field.as_str())),
            })
        }
        Kind::Count(other) => {
            let (id, one) = (
                row.id.clone().unwrap_or_else(|| LitStr::new("", Span::call_site())),
                row.english.clone().unwrap_or_else(|| LitStr::new("", Span::call_site())),
            );
            check_text(&one, true)?;
            check_text(other, true)?;
            let counted = quote!(#env::Counted::new(#id, #one, #other));
            Ok(Expanded {
                ty: quote!(#env::Count),
                words: vec![quote!(#env::Word::keyed(#id, #other))],
                counts: vec![counted.clone()],
                english: quote!(#field: #env::Count::new(#counted, &#env::Words::english())),
                localized: quote!(#field: #env::Count::new(#counted, words)),
                rows: quote!(),
            })
        }
        Kind::Array(elements) => {
            let mut words = Vec::new();
            for (id, english) in elements {
                check_id(id)?;
                check_text(english, false)?;
                words.push(quote!(#env::Word::keyed(#id, #english)));
            }
            let n = elements.len();
            let (english, localized, rows): (Vec<_>, Vec<_>, Vec<_>) = elements
                .iter()
                .enumerate()
                .map(|(at, (id, text))| {
                    let word = quote!(#env::Word::keyed(#id, #text));
                    (
                        quote!(#env::Label::Static(#text)),
                        quote!(#env::Label::from_cow(words.get(#word))),
                        quote!(.row(#word, self.#field[#at].as_str())),
                    )
                })
                .multiunzip3();
            Ok(Expanded {
                ty: quote!([#env::Label; #n]),
                words,
                counts: Vec::new(),
                english: quote!(#field: [#(#english),*]),
                localized: quote!(#field: [#(#localized),*]),
                rows: quote!(#(#rows)*),
            })
        }
    }
}

trait Unzip3<A, B, C>: Iterator<Item = (A, B, C)> + Sized {
    fn multiunzip3(self) -> (Vec<A>, Vec<B>, Vec<C>) {
        let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
        for (x, y, z) in self {
            a.push(x);
            b.push(y);
            c.push(z);
        }
        (a, b, c)
    }
}

impl<I: Iterator<Item = (A, B, C)>, A, B, C> Unzip3<A, B, C> for I {}

pub(crate) fn expand_words(input: TokenStream) -> syn::Result<TokenStream> {
    let Words { attrs, vis, name, rows } = syn::parse2(input)?;
    let env: TokenStream = ENV.parse().unwrap_or_default();
    let mut fields = Vec::new();
    let (mut words, mut counts, mut english, mut localized, mut set_rows) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for row in &rows {
        let expanded = expand(row)?;
        let (field, ty, row_attrs) = (&row.field, &expanded.ty, &row.attrs);
        fields.push(quote!(#(#row_attrs)* #[allow(missing_docs)] pub #field: #ty));
        words.extend(expanded.words);
        counts.extend(expanded.counts);
        english.push(expanded.english);
        localized.push(expanded.localized);
        set_rows.push(expanded.rows);
    }
    Ok(quote! {
        #(#attrs)*
        #[derive(Clone, Debug)]
        #vis struct #name {
            #(#fields,)*
        }

        impl #env::WordSet for #name {
            const WORDS: &'static [#env::Word] = &[#(#words),*];
            const COUNTS: &'static [#env::Counted] = &[#(#counts),*];

            fn english() -> Self {
                Self { #(#english),* }
            }

            fn localized_in(self, words: &#env::Words) -> Self {
                Self { #(#localized),* }
            }

            fn rows(&self) -> #env::WordRows {
                #env::WordRows::new() #(#set_rows)*
            }
        }

        impl ::core::default::Default for #name {
            fn default() -> Self {
                <Self as #env::WordSet>::english()
            }
        }
    })
}
