# equaSHion in craft-libs

equaSHion is the equation editor that WordCraft and DeckCraft share. It was extracted from WordCraft
(see [PROVENANCE.md](PROVENANCE.md)) and lives in the `equashion-*` crates here.

| Crate | What |
|---|---|
| `equashion-math` | math tree, OMML / MathML / LaTeX / linear text, templates, caret and selection edit model |
| `equashion-font` | font database (bundled OFL fonts), metrics, glyph outlines, shaping |
| `equashion-layout` | TeX-style layout into a backend-neutral scene, caret geometry |
| `equashion-render` | scenes to RGBA pixels (vello_cpu) and SVG |
| `equashion-egui` | the editor widget (canvas, palette, linear field), with no host types |

Layout in the repo (kept from the standalone repo so the crates' relative paths still work):

- `assets/fonts/`: the bundled fonts and their OFL texts, included with `include_bytes!`.
- `corpus/`: typed equations, LaTeX and OMML samples the test suites read.
- `tests/golden/`: the rendered goldens `equashion-render` compares against.

[DIVERGENCE.md](DIVERGENCE.md) lists where behaviour differs from the WordCraft code it came from.
The crates keep the `equashion-*` names (not `craft-*`) because three apps already import them by
that name.
