# craft-html

The bottom layer of reading and writing HTML: a tolerant tokenizer, character references,
escaping and the character set a page declares. It replaces the tokenizer and entity table each
Craft app carried. There is no tree builder, no CSS and no rendering: what the tags mean stays
with the caller (a document importer, a mail reader, a clipboard reader).

```rust
use craft_html::{Lexer, Options, Token, escape, unescape};

let opts = Options { drop: &["script", "title"], raw_text: &["textarea"], ..Options::default() };
let mut lexer = Lexer::with("<title>t</title><style>p{}</style>caf&eacute <b class='x'>&lt;3</b>", opts);
let mut text = String::new();
while let Some(token) = lexer.next() {
    match token {
        Token::Text(t) => text.push_str(&unescape(t)),
        // A style sheet's text, for a caller that reads it.
        Token::Start(tag) if tag.name == "style" => assert_eq!(lexer.raw_content("style"), "p{}"),
        Token::Start(tag) => assert_eq!(tag.attr("class").as_deref(), Some("x")),
        Token::End(name) => assert_eq!(name, "b"),
    }
}
assert_eq!(text, "café <3");
assert_eq!(escape(&text), "café &lt;3");
```

## The tokenizer

`Lexer` is an iterator of `Token::Text`, `Token::Start(Tag)` and `Token::End(name)`, borrowed
from the input. Text and attribute values come as written; `unescape` and `Tag::attr` decode
them.

- A `<` that starts no tag is text. A tag, comment or raw element that never ends runs to the
  end of the input.
- Comments, doctypes, processing instructions and conditional comments never come out.
- `Options::drop`: elements skipped whole, content and end tag included.
- `Options::raw_text`: elements whose content is one text token, not markup.
- `Lexer::raw_content(name)`: after a start tag, take the element's raw content yourself.
- `Limits`: attributes kept per tag (64) and the longest tag name (64 bytes; a longer one has an
  empty name).

## Character references

`unescape` decodes named, decimal and hex references. The named set is HTML 4's (Latin-1, Greek,
symbols, arrows, mathematics) and a few more. The 94 legacy names decode without their `;`
(`&copy 2024`), longest match first, as in browsers; `unescape_attr` leaves such a name alone
when a letter, a digit or `=` follows, so a query string keeps its `&copy=1`. A numeric reference
to 0x80-0x9F means the Windows-1252 character; 0, surrogates, and numbers past U+10FFFF or too
long to be numbers become U+FFFD. Names are looked at for 32 bytes at most, so the work is
linear.

`escape` writes `& < > " '` as references; `escape_dq` leaves `'`, for content and attribute
values in double quotes.

## HTML documents (feature `flow`)

`craft_html::doc` reads a page or a pasted fragment into the [craft-flow](../flow/README.md)
model and a craft-wordmodel `Document`, and writes one as clean HTML. The reader is a tolerant
tag-soup builder on this crate's tokenizer (a stack of open elements with HTML's implicit closes),
with a CSS subset from `style` attributes and `<style>` rules with simple selectors in cascade
order, tables with spans, lists, pictures (`data:` URIs, others through the caller's
`ImageLoader`), ruby and MathML equations ([craft-math](../math/README.md)). `import_page` opens a
web page as Word does (Times New Roman 12 pt, HTML's default spacing); `import` reads a fragment in
the document's own formatting. `decode` / `decode_as` turn a page's bytes into text (BOM, declared
charset, else detection). The writer gives semantic HTML with inline CSS, pictures as `data:`
URIs, equations as MathML and ruby as `<ruby>`.

The feature brings in craft-flow, craft-wordmodel, craft-math, craft-mime and craft-codepage; a
reader that only needs tokens (mail, clipboard tables) leaves it off.

## Not here

- The full HTML5 named reference table (about 2,200 names): only the names above.
- Decoding bytes to text. `declared_charset` gives the label a page names; a decoder for it is
  the caller's (`craft-mime::charset` for the WHATWG labels).
- A DOM, and CSS beyond the subset the document reader applies.

## Provenance

The tokenizer is MailCraft's (`crates/html/src/tokenizer.rs`), with its policy (which elements to
drop, the caps) turned into options. The character references are WordCraft's
(`crates/formats/src/html/entity.rs`: the name table and the legacy names without `;`), with
MailCraft's Latin-1 and Greek tables and its handling of numbers too long to be numbers.
Written from the HTML Living Standard.
