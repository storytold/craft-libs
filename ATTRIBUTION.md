# Attribution

Third-party assets bundled in this repository (fonts, icons, images, test data), each with its
author, source and license. Library code and original assets are under the repository license
(MIT OR Apache-2.0, see [NOTICE](NOTICE)).

| Asset | Author | Source | License | Notes |
|---|---|---|---|---|
| `assets/fonts/SourceSerif4-Regular.ttf` | Frank Grießhammer, Adobe | https://github.com/adobe-fonts/source-serif | OFL-1.1 (`assets/fonts/OFL-SourceSerif4.txt`) | Default equation family (`equashion-font`) |
| `assets/fonts/SourceSerif4-It.ttf` | Frank Grießhammer, Adobe | https://github.com/adobe-fonts/source-serif | OFL-1.1 (`assets/fonts/OFL-SourceSerif4.txt`) | Math italic (`equashion-font`) |
| `assets/fonts/SourceSerif4-Bold.ttf` | Frank Grießhammer, Adobe | https://github.com/adobe-fonts/source-serif | OFL-1.1 (`assets/fonts/OFL-SourceSerif4.txt`) | Bold math (`equashion-font`) |
| `assets/fonts/SourceSerif4-BoldIt.ttf` | Frank Grießhammer, Adobe | https://github.com/adobe-fonts/source-serif | OFL-1.1 (`assets/fonts/OFL-SourceSerif4.txt`) | Bold italic math (`equashion-font`) |
| `assets/fonts/Inter-Regular.ttf` | Rasmus Andersson / The Inter Project Authors | https://github.com/rsms/inter | OFL-1.1 (`assets/fonts/OFL-Inter.txt`) | Second bundled family; the layout unit tests use it (`equashion-font`) |
| `assets/fonts/STIXTwoMath-Regular.otf` | The STIX Fonts Project Authors | https://github.com/stipub/stixfonts (copied unmodified from Fedora's `stix-fonts-2.13b171` package) | OFL-1.1 (`assets/fonts/OFL-STIXTwoMath.txt`) | Math symbol fallback; not subset (838 KB) (`equashion-font`) |
| `assets/fonts/OFL-*.txt` | (license texts) | upstream projects | | |

The OMML samples in `corpus/omml/` are copied from the WordCraft and DeckCraft test suites
(MIT OR Apache-2.0, like the code). The typed equations in `corpus/equations.tsv` and the generated
files in `tests/golden/` are original to equaSHion.
