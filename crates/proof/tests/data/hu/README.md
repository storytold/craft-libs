# Hungarian reference data

These small regression fixtures were retained from DesignCraft #1484. They are
not production dictionaries and are never used for runtime spelling. The
unchanged TSV files and their recorded oracle versions, selected licences and
SHA-256 hashes are described in sources.json. Magyar Ispell-derived word data:
MPL-2.0; Huhyphn-derived break data: MPL-1.1. Full licence texts are adjacent.

The original contributions record Németh László / László Németh and Godó Ferenc
for Magyar Ispell; Bence Nagy / Nagy Bence and László Németh for Huhyphn.
The 550 spelling verdicts are raw Hunspell results; two decomposed accent cases
are intentionally accepted by the application's NFC normalization instead.
The application-level check also handles hyphen splitting, unlike AffixDict's
raw compound lookup. The 857 native Hyphen cases use the original test's 2/2
minimum source-letter limits. Suggestion ranking is not required to be identical.

Run with CRAFT_HU_TEST_PACKS pointing to the downloaded pack folder:
`cargo test -p craft-proof --test hungarian -- --include-ignored --test-threads=1`.
These pack-dependent tests stay explicitly ignored in ordinary offline CI.
