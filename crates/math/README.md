# craft-math

Equations as one tree, and the syntaxes it is read from and written to:

| Module | Syntax | Read | Write |
| --- | --- | --- | --- |
| `math` | the tree: `MNode`, `MRun`, `Math` (an equation with its kept source OMML), modelled on the Office Math object model (ECMA-376 Part 1 §22.1) | | |
| `math_linear` | UnicodeMath, the linear format Word shows (`x=(-b±√(b^2-4ac))/2a`) | `math::parse_linear` | `math::to_linear` |
| `math_latex` | LaTeX math, including MathLive's spellings | `parse_latex` | `to_latex` |
| `latex_text` | LaTeX math as text, keeping the source's spacing (for LaTeX documents) | `latex_to_linear` | `linear_to_latex` |
| `omml` | Office Math Markup (`m:oMath`, DOCX/PPTX/XLSX) | `read_omath`, `read_str` | `write_equation`, `write_str` |
| `mathml` | presentation MathML (HTML, ODF formulas, the clipboard) | `parse`, `read`, `to_tree` | `to_mathml`, `linear_to_mathml` |

Beside them: `math_symbols` (LaTeX names of symbols), `math_tree` (the arguments of a node, a walk
over every run), `symbol_fonts` (Symbol, Wingdings and the other symbol fonts as Unicode) and
`Rgb` (a run's colour).

```rust
use craft_math::{Math, math, math_latex, mathml, omml};

let tree = math_latex::parse_latex(r"\frac{a}{b}+\sqrt{x}");
assert_eq!(math::to_linear(&tree), "a/b+√x");

// OMML out and back.
let eq = Math { nodes: tree.clone(), ..Default::default() };
let xml = omml::write_str("", false, &eq, None);
let read = omml::read_str(&xml).unwrap();
assert_eq!(read.nodes, tree);

// MathML carries the linear format in an annotation, so it reads back exactly.
let ml = mathml::linear_to_mathml("a/b+√x", true);
assert_eq!(mathml::parse(&ml).and_then(|m| mathml::read(&m)), Some(("a/b+√x".to_string(), true)));
```

## Reading OMML

`omml::read_omath` takes an `m:oMath` element of a `craft_xml` tree read with the canonical
prefixes of `omml::NAMESPACES` (`m`, `w` and `mc`, transitional and strict URIs), so a document
reader passes the element it already has. `omml::read_str` and `omml::parse` read a string. The
source XML is kept in `Math::omml`; `write_equation` writes it back unchanged while the
equation's colour is the same and the XML is well formed, so a saved document carries the
equation as Word wrote it.

## Limits

Nesting is bounded by `math::MAX_DEPTH` in every reader and writer; `omml::parse` reads at most
`omml::MAX_ELEMENTS` elements and `mathml::parse` keeps at most `mathml::MAX_NODES`; the LaTeX
reader looks at 20,000 characters. Nothing panics on any input.

## Provenance

The tree, the linear format, LaTeX, the symbol names, the tree walk, the symbol fonts and `Rgb`
are WordCraft's (`crates/doc`), as moved into craft-wordmodel; OMML is WordCraft's DOCX reader
and writer (`crates/docx/src/read/math.rs`, `write/math.rs`) and MathML its formats crate's
(`crates/formats/src/mathml.rs`), moved onto craft-xml. The MathLive command spellings and the
round-trip corpus in `tests/corpus/` come from equaSHion (storytold/craft-libs#29).
