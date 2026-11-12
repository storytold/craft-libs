# craft-latex

LaTeX documents, read and written, to and from the [craft-flow](../flow/README.md) model and a
craft-wordmodel `Document`.

- **Write** (`export`): a standalone LaTeX 2ε `article` that compiles with pdfLaTeX, XeLaTeX or
  LuaLaTeX using only packages every distribution ships (`fontenc`, `ulem`, `xcolor`, `multirow`,
  `amsmath`, `hyperref`): unnumbered sectioning commands, `itemize`/`enumerate`, `tabular` with the
  column widths, equations `$…$` / `\[…\]`, character formatting, links and bookmarks. Pictures
  become their alternative text. Nothing the document says is written as a TeX command.
- **Read** (`import`, `parse` for the flow): the everyday subset documents are written in: the
  preamble's `\title`/`\author`, sectioning, paragraphs, lists, `quote`, `center`/`flush…`,
  verbatim-like environments, `tabular` with `\multicolumn`/`\multirow`, inline and display math
  (kept as equations), text formatting commands and declarations, colours, links, labels, accents,
  special characters and ligatures. It is not a TeX engine: macros are not expanded. Unknown
  commands are dropped with their braced arguments kept as text; layout commands are dropped
  with their arguments. Bytes that name no encoding are decoded by `craft_mime::detect`.

```rust
let doc = craft_latex::import(br"\section*{Intro} Some \textbf{bold} text and $x^2$.");
assert!(doc.plain_text(Default::default()).starts_with("Intro\nSome bold text and"));
let tex = craft_latex::export(&doc);
assert!(tex.contains(r"\section*{Intro}") && tex.contains(r"\textbf{bold}") && tex.contains("$x^2$"));
```

Input is hostile: nesting, column specs, labels and the source size (`MAX_SOURCE`) are bounded,
and the work is linear.

## Math

`latex_to_linear` and `linear_to_latex` convert the math of a document to and from the linear
format equations are kept in, as text, keeping the source's spacing. craft-math converts the same
two syntaxes through its equation tree. These are two converters for one pair of syntaxes, and
which one documents should use is an open question; this crate keeps WordCraft's choice.

Moved from WordCraft (`crates/formats/src/latex`, `07f97737`) with its tests and the LaTeX parts
of its cross-format tests.
