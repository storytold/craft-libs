# craft-font-store

The suite's font store: the font indexes of every source, exact lookup of a missing font, and verified installs into the one folder every app's font catalog reads ([`craft-font-catalog`](../font-catalog/README.md)'s `font_store_dir`).

The indexes are [aiongg/craft-font-index](https://github.com/aiongg/craft-font-index) (format 1.x), designed by @kntrttm for photocraft#1135 and #2453. Every font file and licence in an index has a size and a sha256, and only families under a licence that allows redistribution (OFL, Apache 2.0, Ubuntu Font Licence) are listed.

## Sources

| Source | ID | Curated | Fonts come from |
|---|---|---|---|
| Google Fonts | `google/fonts` | yes | [google/fonts](https://github.com/google/fonts) at the commit the index records |
| Noto | `notofonts` | yes | [notofonts](https://github.com/notofonts) repositories at the commits and release tags the index records (the full CJK fonts from noto-cjk, and script families, widths and UI variants Google Fonts lacks) |
| An index the user adds | `user:<host>/<id>` | never | wherever its URL templates point (https only) |
| An index the build's host gives | `host:<id>` | never | wherever its URL templates point (https only) |

The built-in sources are the indexes `sources.json` lists. `PINNED_INDEX` names a commit of the index repository and the sha256 of its `sources.json`; each index must have the size and sha256 `sources.json` gives it.

A source the user adds is an index file at an https URL (`.json` or `.json.gz`). It is checked by the same rules as a built-in one, but:

- it is never curated, whatever its file says, so the interface can mark its families "not curated";
- its ID is prefixed with `user:<host>/`, so it can't stand in for a built-in source;
- the sha256 of the file first fetched is kept, and a different file later shows as `FontSource::changed`;
- only an app's own interface can add one (`add_user_source`); an agent gets `Error::NotAllowed`.

A source the build's host gives is for a build that has no font store folder of its own, a web build in a browser tab or a headless engine on a server, whose host page or service has the fonts its users' documents need ([RFC-0034](https://github.com/storytold/craft-libs/issues/2331)). `Index::from_host_bytes(bytes, id)` reads it (JSON or gzipped JSON) by the rules a user's source is read by, with one exception, and:

- `id` is the host's own name for the index: 1 to 64 of `[A-Za-z0-9._-]`, not starting with `.`; anything else is `IndexError::Invalid`;
- its source ID is `host:<id>`, never the ID its file declares, so it can't stand in for a built-in or a `user:` source;
- it is never curated, whatever its file says;
- after the whole index has passed the rules below, a file whose path ends in `.ttc` or `.otc`, ignoring case, is left out with a warning naming the first three by file name and counting the rest, and a family with no other file with it, since `verify_face` refuses every collection (the check is of the name: a single-face font named `.ttc` or `.otc` is left out too, and a collection named `.ttf` is refused when it is fetched); a user's source keeps them;
- an index with an `http` URL template, an unsafe path, a bad sha256 or any other entry the rules refuse is refused whole and adds nothing (a family under a licence that doesn't allow redistribution is not an error: it is left out with a warning, as in a user's source). A list of indexes is all or nothing as RFC-0034 §1.3 says: the build refuses the whole list when one is refused, so the caller reads each with `from_host_bytes` and stops at the first error.

An index may list a single file, so a host that hands over one face at a time needs no catalogue. The build lists the families at once and fetches a face only when type in a document uses it, from the URLs `index.file_urls(family, file)` gives, then checks it with `verify_face` (see Verifying a fetched file). Each host index needs an `id` of its own, since the ID is the source's; two indexes given the same one are one source: `Index::merge` keeps the first and records a `SourceProblem` for the second. `Index::merge` keeps same-named families of different sources apart (a family of the same name in a built-in source and a host's index are two families): look one up by source with `family_from`, or take them all with `families_named` and `Family::face` (or `Index::find_face`, which takes the first source that has the style) on the family you mean, rather than assuming one family per name.

## Network access

| Rule | How |
|---|---|
| Off until the user turns it on | `FontStore::network()` is `Network::Off` until `enable_network()`. The setting is `settings.json` in the store folder, so it holds for every app. |
| Only the user turns it on | `FontStore::for_user` is for the app's own interface; call `enable_network()` from a setting or from the user's Install click. `FontStore::for_agent` (MCP, CLI, scripting) gets `Error::NotAllowed`. An agent can install while the user has the network on. |
| No requests at startup | Nothing in the crate makes a request on its own. Only `fetch_indexes` and the installs do, and they return `Error::NetworkOff` without one while the network is off. |
| One HTTP client | `Client`: ureq with rustls and the webpki roots, HTTPS only, no cookies, `User-Agent: craft-font-store/<version>`. Timeouts: 15 s to connect, 30 s to the response headers, 10 minutes per file. |

A file's URLs come from its source's URL templates, best first. Both built-in sources use `https://cdn.jsdelivr.net/gh/{repo}@{ref}/{path}`, then `https://raw.githubusercontent.com/{repo}/{ref}/{path}`. The store tries the next URL after any failure: a network error, an HTTP error (jsDelivr answers 403 for files over 20 MB, such as the Noto Serif CJK fonts), or bytes that don't verify.

## The index

`Index` holds the families of one or more sources. Families are sorted by name. Two sources can have a family of the same name (Google Fonts and Noto both have `Noto Serif Myanmar`, Noto's with more styles); lookups by name prefer the first source: the built-in ones in `sources.json` order, then the user's and the host's, in the order merged.

| Call | What it does |
|---|---|
| `Index::bundled()` (feature `bundled-index`) | The pinned built-in indexes compiled in: about 620 KB gzipped, no request. |
| `FontStore::offline_indexes()` (feature `bundled-index`) | The bundled indexes and the user's sources as last fetched. |
| `FontStore::cached_indexes(&PINNED_INDEX)` | The indexes the last `fetch_indexes` saved, when its `sources.json` is the pin's and every built-in index matches it, with the user's sources. |
| `FontStore::fetch_indexes(&PINNED_INDEX, &cancel)` | Downloads `sources.json`, every gzipped index it lists and every user source, checks them and saves them. An index that fails is reported in `Index::problems()`; the others load. Needs the network on. |
| `Index::from_bytes(bytes)`, `Index::from_user_bytes(bytes, url)`, `Index::from_host_bytes(bytes, id)` | One index file, JSON or gzipped JSON. |
| `Index::merge(indexes)` | One index of several. |

Reading checks every entry: the version (a major version other than 1 is refused; unknown fields are ignored), https URL templates, safe relative paths (no empty, `.` or `..` segment, no `\`), safe and unique file names, `owner/name` repositories and refs, sha256s, sizes, weights and widths. A format 1.0 index is Google Fonts, with URLs from its `base_url` and `fallback_base_url`.

| Call | What it gives |
|---|---|
| `index.sources()`, `index.source_of(family)` | Each source's name, homepage, curated flag, commit, and families or files left out with why (a licence, or in a host's index a font collection). |
| `family.source`, `family.is_curated()` | The family's source ID, and whether that source is curated. |
| `index.file_urls(family, file)`, `index.license_urls(family)` | The URLs to try, best first. |

## Exact matches only

| Call | Matches |
|---|---|
| `index.family(name)` | The whole family name, ignoring case, from the first source that has it. `families_named(name)` gives every source's; `family_from(source, name)` one source's. |
| `index.find_face(family, style)` | A style name (`Bold Italic`, `SemiBold`), a CSS weight (`700`) or a PostScript style (`BoldItalic`, `CondensedBold`). A variable file has every weight its `wght` axis reaches. A file of normal width is preferred. |
| `index.find_postscript(name)` | The PostScript name, case included. A name that families of different names share is `Unavailable::Ambiguous`; look it up in the document's family with `Family::face_by_postscript`. |

A family without the style asked for is `Unavailable::NoStyle`: the store never offers a nearer weight or a similar family. Substitution stays a separate choice in each app.

## Installing

```rust
use craft_font_catalog::{Catalog, FontEnv};
use craft_font_store::{Cancel, FontStore, Index};

let store = FontStore::for_user(&FontEnv::from_process()).unwrap().with_catalog(Catalog::global().clone());
let index = store.offline_indexes();
let face = index.find_face("Lato", "Bold Italic")?;
store.enable_network()?; // the user clicked Install
let cancel = Cancel::new(); // cancel.cancel() from the UI thread stops the download
store.install_face(&index, face, &cancel, &mut |p| println!("{} of {} bytes", p.done, p.total))?;
```

`install_family` installs every file of a family; `install_face` one file and the licence. Each file is:

1. downloaded (at most its index size), with progress and cancellation;
2. checked: its size, its sha256, then that every face in it parses and has outlines (`verify_font`);
3. saved under `.staging/` as `<name>.part`, which no font scan picks up.

The licence file (`OFL.txt`, `LICENSE`) is checked against its size and sha256 and goes beside the fonts. For a format 1.0 index, which has no sha256 for it, only its size is checked (1 MiB at most).

When every file has passed, the store takes `.lock`, moves the files into the family's folder and writes `manifest.json` (written atomically through `craft-atomic`). If a move or the manifest fails, the files moved so far are taken back out and replaced files are restored. A failed or cancelled install leaves nothing behind. Files already installed with the same sha256 aren't downloaded again.

## Verifying a fetched file

`verify_font(file, bytes)` checks a file against its entry: its size, its sha256, then that every face in it parses and has outlines. A file with the index's sha256 is the file the index lists, so it can only be the wrong font if the index itself is wrong.

`verify_face(family, file, bytes)` is for a build that takes a host's index (RFC-0034 §3): it checks that the file is what the entry says. The host supplies the index and the sha256 together, so this catches an index that describes the wrong file by its family, axes or PostScript name, and, when the entry lists no PostScript names, by its weight, width or italic flag; with PostScript names listed, the PostScript name is the style's identity and those three are not compared. It is not a security boundary. It passes exactly when the bytes are the entry's size and sha256 and one font with a `name` table and outlines, and:

| Attribute | The file passes when |
|---|---|
| File | it is not a collection: an entry can't name a face of one, so a collection is refused (`VerifyError::Collection`), and a host's index leaves out a `.ttc` or `.otc` file. |
| Family | the entry's family, squashed (case, spaces and punctuation ignored), equals the squashed text of any non-empty name ID 16 or ID 1 record (a family with no letters or digits must match exactly) (`VerifyError::Family`). |
| Axes | it is variable exactly when the entry lists axes, each range at least the entry's (`VerifyError::Axis`, `VerifyError::Style`). |
| PostScript | when the entry lists any names, the font's ID 6 is one of them, exactly (`VerifyError::PostScript`); the PostScript name is then the style's identity (generators, `craft-font-index` among them, write the font's own ID 6 as the `postscript` name), and weight, width and italic are not compared. |
| Weight (no PostScript names) | skipped with a `wght` axis; otherwise the entry's weight is the `OS/2` weight class clamped to 1 to 1000, or 100 when the class is 250, or 200 when it is 275; with no `OS/2` table, 700 when `head`'s macStyle is bold, else 400. |
| Width (no PostScript names) | skipped with a `wdth` axis; otherwise within 0.5 of the percentage of the width class (50, 62.5, 75, 87.5, 100, 112.5, 125, 150, 200 for 1 to 9; a class outside 1 to 9 is clamped, and 100 is also accepted for it); with no `OS/2` table, 100. |
| Italic (no PostScript names) | skipped with an `ital` axis; otherwise the entry equals either reading of the raw fields: `fsSelection` bit 0 or bit 9 (oblique), or `fsSelection` bit 0 or `head`'s macStyle italic bit; with no `OS/2` table, the macStyle italic bit. |

These are our own readings of the raw `OS/2` and `head` fields, not `skrifa`'s `font.attributes()`, because generators read those fields differently. The work is bounded: one sha256 pass, one table directory and one face; for each of the name IDs 16, 1 and 6 at most 65,535 record headers are scanned, at most 64 non-empty records kept and at most 1,025 characters of each decoded. A record longer than 1,024 characters is not a match (it is skipped, never cut short and compared).

A file that names itself another family is refused even when its PostScript name is listed, because a build registers a face by the file's own names: accepting it would add the other family's face, never the entry's. A few google/fonts files do this.

`Size`, `Family`, `Style` and `Axis` errors say what they expected and what they found. A file that fails a check is refused and nothing in it is used; the rest of its index is unaffected. The store's installs check with `verify_font` only: the store folder is native only and holds the fonts of built-in sources and those the user added, whom the user chose to trust. `FontStore::install_family` and `install_face` refuse a family of a host's index (`host:`) with `Error::HostFamily`, before anything is read or downloaded.

## The store folder

```text
settings.json        the network setting and the sources the user added
manifest.json        what is installed
indexes/             the last indexes fetched: sources.json, <source>.json.gz, user-<hash>
<family folder>/     the font files and the licence
language-packs/      installed Language packs, one folder per locale
language-packs.json  which Language packs are installed
.staging/            downloads being installed
.lock                held while the store changes
```

A Google Fonts family's folder is google/fonts' own folder name (`lato/`); another source's is `<source>/<family>` (`notofonts/noto-sans-cjk-jp/`, `user-fonts-example-org-myfonts-bfeb06e2/some-family/`).

`manifest.json` lists each installed family: its name, source, folder, licence and licence file, and per file its name, path in its repository, size, sha256, the index commit it came from, and its PostScript names. An entry is one family of one source, so the same name from two sources is two entries. A manifest that can't be read is moved aside to `manifest.json.bad` and counts as empty.

| Call | What it does |
|---|---|
| `installed()`, `installed_family(source, name)`, `is_installed(family, file)` | Read the manifest. |
| `remove_family(source, name)` | Moves the family folder out at once, updates the manifest, then deletes the files and the licence. |
| `verify()` | Reads every installed file and reports the missing, damaged and unparseable ones. |
| `user_sources()`, `add_user_source(url)`, `remove_user_source(url)` | The sources the user added. Removing one leaves its installed families installed. |

## Telling the catalog

The catalog scans the store folder. With `with_catalog(catalog)`, every install and removal ends with `catalog.rescan()`, which reads only new and changed files, so the app sees the fonts at once. Other running apps notice through `Catalog::rescan_if_changed` (when their window is activated) or `Catalog::watch`.

## Language packs

The store's second item kind, `language-pack`: a language's spelling dictionary, hyphenation patterns and licence texts, published by [storytold/craft-i18n](https://github.com/storytold/craft-i18n) as `<locale>.tar.zst` release assets listed in an `index.json`, and read at runtime by `craft-proof` (`craft_proof::packs`). It reuses the store's folder, consent, HTTP client, staging and sha256 checks; there is no second downloader.

```rust
const PACKS: PackIndexPin = PackIndexPin { url: "https://github.com/storytold/craft-i18n/releases/download/language-packs-1/index.json", sha256: "<hex>" };
let index = store.fetch_language_pack_index(&PACKS, &Cancel::new())?;   // or cached_language_pack_index(&PACKS)
let nl = index.pack("nl").unwrap();
store.install_language_pack(&index, nl, &Cancel::new(), &mut |p| ())?;  // network on, like fonts
craft_proof::packs::set_packs_dir(Some(store.language_packs_dir()));      // then packs::reload() after each change
store.remove_language_pack("nl")?;
let installed = store.installed_language_packs();
```

| Call | What it does |
|---|---|
| `fetch_language_pack_index(pin, cancel)` | Downloads the index, checks it against the pin's sha256, saves it as `indexes/language-packs.json`. Needs the network on. |
| `cached_language_pack_index(pin)` | The saved index, when it is the pinned one. |
| `install_language_pack(index, pack, cancel, progress)` | Downloads the archive (at most 128 MiB), checks size and sha256, unpacks it into `.staging/` and moves it into `language-packs/<locale>/` with one rename (an older pack is replaced the same way). Records it in `language-packs.json`. |
| `remove_language_pack(locale)` | Moves the folder out at once, updates the record, deletes the files. No network needed. |
| `installed_language_packs()` | The recorded packs whose folder is there. |
| `language_packs_dir()` | `<store>/language-packs`, for `craft_proof::packs::set_packs_dir`. |

Removal restores the original folder if the installed-record update fails. If restoration also
fails, the error names a preserved backup under `language-packs/.removing/`. That recovery
folder is outside the ordinary download staging sweep; cleanup does not delete the backup.
Committed removals record their backup IDs in the same atomic record update. On lock acquisition,
the cleanup sweep attempts at most four previously recorded backups, completing finite rounds through
the receipts within a live store so failed entries are revisited even as new receipts arrive.
This bounds the number of
deletion attempts, not the work to delete one backup. Recreating the store resets the cursor;
backups and receipts remain available for later retries, and the next lock need not finish all
pending cleanup. Failed rollback backups have no receipt and remain available for recovery.
Receipts stay until cleanup succeeds and a later record write prunes them; their IDs cannot be
reused for a new removal while they remain recorded.
An installed record rebuilt after corruption has no cleanup receipts; unreceipted backups are
retained for manual recovery.
Invalid receipt names are discarded without losing readable pack metadata or other valid receipts.
Cleanup retains open store, Language pack, and recovery-directory handles; it rejects symlinked
recovery ancestors. Unix cleanup resolves receipt names relative to the open recovery directory;
Windows retains ancestor handles that prevent their replacement during cleanup.

**Unpacking is hostile-input safe:** pure-Rust zstd (`ruzstd`, MIT; every frame is read, and nothing but tar padding may follow the archive) and `tar` (MIT OR Apache-2.0); regular files and the `LICENCES` folder only (no links or devices). Only the contract's names are allowed: `pack.toml`, `spelling.aff`, `spelling.dic`, `hyphenation.pat` at the root and `LICENCES/<name>` (`[A-Za-z0-9._-]`, no leading or trailing dot, no Windows device names such as `CON` or `nul.txt`). Each file at most 64 MiB, 256 MiB and 256 entries per pack. `pack.toml` is required and must name the entry's locale; the files the index promises (`spelling`, `hyphenation`) must be there. Index entries need a lowercase BCP 47 locale (listed once, not a Windows device name), a sha256 and an `https://` URL; names, licences, versions, classes and the release lose control and bidi/format characters before they are stored or listed; an optional `source` must be an `https://` URL. At most 4096 zstd frames per archive. An unreadable `language-packs.json` is copied to `language-packs.json.bad` and rebuilt from the pack folders (each `pack.toml`) atomically in place, as is a missing one when pack folders are there, so no installed pack drops out of the list. Only an install or removal (under the store lock) rewrites it; listing reads the folders meanwhile and writes nothing.

**Developer override (testing only — bypasses the pin):** `CRAFT_LANGUAGE_PACK_INDEX=<url or path>` replaces the pinned index URL with an `http://` URL on a loopback host (`127.0.0.1`, `localhost`, `[::1]`), an `https://` URL or a local file path (`file://` URLs are not taken), and **skips the index's sha256 pin**. Archives are still checked against the sha256 and size in that index, and may then also be loopback `http://` URLs or local paths. Off unless the variable is set; it works in release builds too (for end-to-end tests of a real build), a warning is logged whenever it is used, and `language_pack_index_overridden() -> Option<String>` returns the value so apps can show that the pin is bypassed. Never set it for users.

## Web

The index (`from_host_bytes` included), the lookups, `verify_font`, `verify_face` and `IndexPin` build for `wasm32-unknown-unknown`. The store folder and the HTTP client are native only; the web keeps fonts elsewhere.

## Tests

`cargo test -p craft-font-store --all-features`. The store tests run a small HTTP server on 127.0.0.1 that stands in for jsDelivr, raw GitHub and a user's host; no test uses the network. One ignored test fetches the pinned indexes and installs a Noto family over the network: `cargo test -p craft-font-store --all-features -- --ignored`.
