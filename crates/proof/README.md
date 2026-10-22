# craft-proof

Spelling mechanism shared by the Craft apps: read a Hunspell `.aff`/`.dic` pair installed on the
system and check words against it. Seeded from WordCraft's `wordcraft-proof`; GridCraft,
DeckCraft and PdfCraft are the next consumers (storytold/craft-libs#30). No dependency on any
app or UI framework. Requires Rust 1.95.

```rust,no_run
let search = craft_proof::hunspell::Search::for_app("WordCraft");
if let Some((aff, dic)) = search.find(&["pl_PL", "pl"]) {
    let dict = craft_proof::hunspell::load(&aff, &dic)?;
    assert!(dict.check("kotami"));
}
# Ok::<(), String>(())
```

- **Dictionaries are not bundled.** Apps read what the OS, a package manager or LibreOffice
  installed; without one a language simply has no spelling check.
- **Search order** (`Search`): the app's dictionaries folder setting, `<APP>_DICTIONARIES`, the
  app's folder under the user's application data (`%LOCALAPPDATA%\WordCraft\dictionaries`,
  `~/Library/Application Support/WordCraft/dictionaries`, `$XDG_DATA_HOME/wordcraft/dictionaries`),
  then the usual Hunspell and LibreOffice locations for the platform. Nothing on wasm.
- **Affix engine** (`AffixDict`): the Hunspell subset real dictionaries such as SJP.PL (Polish)
  and VERO (Brazilian Portuguese) use: one-character or `FLAG UTF-8` flags, prefixes and
  suffixes with conditions and cross products, continuation classes, `FORBIDDENWORD`, and the
  `TRY`/`MAP`/`REP` suggestion tables. Compounding and long or numeric flags are rejected with
  an error rather than misread.
- **Compact form**: `AffixDict::to_bytes`/`from_bytes` store a loaded dictionary deflated
  (Polish is under 1 MB), so an app can cache it instead of parsing the `.aff`/`.dic` each start.
  The data starts `WCAFF1`, or `WCAFF2` when the dictionary uses `KEY` or `NOSUGGEST`; a reader
  that predates those rejects `WCAFF2` data as a bad header rather than misreading it, so a cache
  should be rebuilt when that happens.

What stays in each app: which languages it proofs, suggestion ranking and grammar rules per
language, the user dictionary, and how marks are shown.
