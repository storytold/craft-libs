# craft-font-store

The suite's font store: the font indexes of every source, exact lookup of a missing font, verified installs into the one folder every app's font catalog reads ([`craft-font-catalog`](../font-catalog/README.md)'s `font_store_dir`), and, per family at the user's request, copies in the OS user font folder so that every other program can use it.

The indexes are [aiongg/craft-font-index](https://github.com/aiongg/craft-font-index) (format 1.x), designed by @kntrttm for photocraft#1135 and #2453. Every font file and licence in an index has a size and a sha256, and only families under a licence that allows redistribution (OFL, Apache 2.0, Ubuntu Font Licence) are listed.

## Sources

| Source | ID | Curated | Fonts come from |
|---|---|---|---|
| Google Fonts | `google/fonts` | yes | [google/fonts](https://github.com/google/fonts) at the commit the index records |
| Noto | `notofonts` | yes | [notofonts](https://github.com/notofonts) repositories at the commits and release tags the index records (the full CJK fonts from noto-cjk, and script families, widths and UI variants Google Fonts lacks) |
| An index the user adds | `user:<host>/<id>` | never | wherever its URL templates point (https only) |

The built-in sources are the indexes `sources.json` lists. `PINNED_INDEX` names a commit of the index repository and the sha256 of its `sources.json`; each index must have the size and sha256 `sources.json` gives it.

A source the user adds is an index file at an https URL (`.json` or `.json.gz`). It is checked by the same rules as a built-in one, but:

- it is never curated, whatever its file says, so the interface can mark its families "not curated";
- its ID is prefixed with `user:<host>/`, so it can't stand in for a built-in source;
- the sha256 of the file first fetched is kept, and a different file later shows as `FontSource::changed`;
- only an app's own interface can add one (`add_user_source`); an agent gets `Error::NotAllowed`.

## Network access

| Rule | How |
|---|---|
| Off until the user turns it on | `FontStore::network()` is `Network::Off` until `enable_network()`. The setting is `settings.json` in the store folder, so it holds for every app. |
| Only the user turns it on | `FontStore::for_user` is for the app's own interface; call `enable_network()` from a setting or from the user's Install click. `FontStore::for_agent` (MCP, CLI, scripting) gets `Error::NotAllowed`. An agent can install while the user has the network on. |
| No requests at startup | Nothing in the crate makes a request on its own. Only `fetch_indexes` and the installs do, and they return `Error::NetworkOff` without one while the network is off. |
| Optional preview service | Off unless set: no default address. `CRAFT_FONT_PREVIEW_URL`, else the `preview_service` field of `settings.json` (`FontStore::set_preview_service`, user only, `Error::NotAllowed` for an agent). `FontStore::preview_service()` returns the one in effect and `preview_service_source()` says whether it is `Env`, `Saved` or `None`; the variable's name is `PREVIEW_SERVICE_ENV`. https only; http is accepted only for the addresses `127.0.0.1` and `::1`, never for a host name, which a resolver can point off the machine. A trailing slash is dropped. Example: `CRAFT_FONT_PREVIEW_URL=https://fonts.example.com`. |
| Previews setting | `previews` in `settings.json`, on by default: `FontStore::previews_enabled()`, `set_previews_enabled(bool)` (user only, `Error::NotAllowed` for an agent). The app fetches previews from a set service only while it is on; installs ignore it. |
| Mirror first | With a service set, an install requests `<service>/v1/files/<sha256>` for each font file and `<service>/v1/licences/<sha256>` for a licence (when the index gives its sha256) before the index's URLs. The size and sha256 checks are the same, and a response that fails them moves on to the next URL. |
| One HTTP client | `Client`: ureq with rustls and the webpki roots, HTTPS only (plain http only to `127.0.0.1` or `::1`, following no redirects, for a preview service run locally), no cookies, `User-Agent: craft-font-store/<version>`. Timeouts: 15 s to connect, 30 s to the response headers, 10 minutes per file. |

A file's URLs come from its source's URL templates, best first. Both built-in sources use `https://cdn.jsdelivr.net/gh/{repo}@{ref}/{path}`, then `https://raw.githubusercontent.com/{repo}/{ref}/{path}`. The store tries the next URL after any failure: a network error, an HTTP error (jsDelivr answers 403 for files over 20 MB, such as the Noto Serif CJK fonts), or bytes that don't verify.

## The index

`Index` holds the families of one or more sources. Families are sorted by name. Two sources can have a family of the same name (Google Fonts and Noto both have `Noto Serif Myanmar`, Noto's with more styles); lookups by name prefer the first source: the built-in ones in `sources.json` order, then the user's.

| Call | What it does |
|---|---|
| `Index::bundled()` (feature `bundled-index`) | The pinned built-in indexes compiled in: about 620 KB gzipped, no request. |
| `FontStore::offline_indexes()` (feature `bundled-index`) | The bundled indexes and the user's sources as last fetched. |
| `FontStore::cached_indexes(&PINNED_INDEX)` | The indexes the last `fetch_indexes` saved, when its `sources.json` is the pin's and every built-in index matches it, with the user's sources. |
| `FontStore::fetch_indexes(&PINNED_INDEX, &cancel)` | Downloads `sources.json`, every gzipped index it lists and every user source, checks them and saves them. An index that fails is reported in `Index::problems()`; the others load. Needs the network on. |
| `Index::from_bytes(bytes)`, `Index::from_user_bytes(bytes, url)` | One index file, JSON or gzipped JSON. |
| `Index::merge(indexes)` | One index of several. |

Reading checks every entry: the version (a major version other than 1 is refused; unknown fields are ignored), https URL templates, safe relative paths (no empty, `.` or `..` segment, no `\`), safe and unique file names, `owner/name` repositories and refs, sha256s, sizes, weights and widths. A format 1.0 index is Google Fonts, with URLs from its `base_url` and `fallback_base_url`.

| Call | What it gives |
|---|---|
| `index.sources()`, `index.source_of(family)` | Each source's name, homepage, curated flag, commit, and families left out with why. |
| `family.source`, `family.is_curated()` | The family's source ID, and whether that source is curated. |
| `index.file_urls(family, file)`, `index.license_urls(family)` | The URLs to try, best first. |
| `family.tags`, `family.copyright`, `family.reserved_names`, `family.notice_file`, `family.upstream` | Format 1.2: design traits, the copyright notice, the Reserved Font Names (a client that modifies a font renames it when there are any), an Apache family's `NOTICE` file and where the family is developed. Absent in a 1.1 index: empty or `None`. |
| `file.instances`, `file.list_instance` | Format 1.2: a variable font's named instances, and the position of the one a font list shows (`None`: the default instance). |
| `index.samples()`, `index.sample(script)` | Format 1.2: the default preview text of each script. When sources have one for the same script, the first source's wins. |
| `index.sample_text(family, file)` | The file's preview text: its own `sample_text`, else the sample of its `sample_script`, else of the family's `sample_script`. |

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

## The store folder

```text
settings.json        the network setting and the sources the user added
manifest.json        what is installed
os-fonts.json        the ledger of copies in the OS user font folder
indexes/             the last indexes fetched: sources.json, <source>.json.gz, user-<hash>
previews/            preview subsets (WOFF2) craft-font-manager caches, at most 50 MB
<family folder>/     the font files and the licence
.staging/            downloads being installed
.lock                held while the store changes
```

A Google Fonts family's folder is google/fonts' own folder name (`lato/`); another source's is `<source>/<family>` (`notofonts/noto-sans-cjk-jp/`, `user-fonts-example-org-myfonts-bfeb06e2/some-family/`).

`manifest.json` lists each installed family: its name, source, folder, licence and licence file, and per file its name, path in its repository, size, sha256, the index commit it came from, and its PostScript names. An entry is one family of one source, so the same name from two sources is two entries. A manifest that can't be read is moved aside to `manifest.json.bad` and counts as empty.

| Call | What it does |
|---|---|
| `installed()`, `installed_family(source, name)`, `is_installed(family, file)` | Read the manifest. |
| `remove_family(source, name)` | Withdraws the family's copies from the OS user font folder (user only when it has some), moves the family folder out at once, updates the manifest, then deletes the files and the licence. `remove_files` withdraws the copies of the files it removes. |
| `verify()` | Reads every installed file and reports the missing, damaged and unparseable ones. |
| `user_sources()`, `add_user_source(url)`, `remove_user_source(url)` | The sources the user added. Removing one leaves its installed families installed. |

## Telling the catalog

The catalog scans the store folder. With `with_catalog(catalog)`, every install and removal ends with `catalog.rescan()`, which reads only new and changed files, so the app sees the fonts at once. Other running apps notice through `Catalog::rescan_if_changed` (when their window is activated) or `Catalog::watch`.

## Copies in the OS font folder

Fonts in the store are for the Craft apps only. A family can also be copied into the OS user font folder, for every other program of the current user (the API calls this making it available to all apps): opt-in per family, off by default, and only from the user's own action.

| Call | What it does |
|---|---|
| `available_to_all_supported()`, `available_to_all_unsupported()` | Whether the action works here, and why not. The interface hides it when it doesn't. |
| `make_available(source, name)` | Checks for conflicts, copies the family's font files into the OS user font folder and records them in the ledger. Calling it again copies styles installed since and brings the copies up to date. Returns the copies. |
| `withdraw(source, name)` | Deletes the copies the ledger lists, each only if its sha256 is unchanged. Returns the copies left in place because they changed. |
| `is_available(source, name)`, `available_family_holding(name, files)`, `available_files(source, name)` | Read the ledger. They read the file again only when it changed, so a list can ask for every row. `available_family_holding` goes by the store files when it has them, so a family of the same name from another source doesn't count. |
| `copies_out_of_reach(source, name)` | The family's listed copies when this build can't change the OS folder (a Flatpak whose access to it was revoked). Removing the family leaves them. |
| `pending_files(source, name)` | Files of an available family not available yet: styles installed since by an agent, or whose copy is older than the store's file. |

`make_available`, `withdraw`, the installs and removals block (hashing, copying, a catalog rescan): call them off the UI thread.

### Who may change the OS folder

Only the user. An agent (`for_agent`) never changes what is in the OS font folder:

- `make_available` and `withdraw` give it `Error::NotAllowed`.
- Its install into an available family stays in the store; the new files show in `pending_files`, and the interface offers to copy them. The user's install copies them at once.
- Its `remove_family` of a family with copies, and its `remove_files` of files with copies, give `Error::NotAllowed` before anything changes.

The user's `remove_family` and `remove_files` withdraw the copies first, so the ledger never lists copies of fonts the store no longer has. Where this build can't change the OS folder (a Flatpak whose `xdg-data/fonts` access was revoked, a Snap build) but the ledger lists copies, removing the family removes its store files and leaves the copies, still listed in the ledger: a build that has the folder withdraws them, and a reinstall finds them the store's. `copies_out_of_reach` names them beforehand.

### Folders

| System | Folder (`OsFontFolder::for_env`) | Copies |
|---|---|---|
| macOS | `~/Library/Fonts` | `Craft-<file>`, side by side. macOS activates them for every app of the user. |
| Linux and the BSDs | `$XDG_DATA_HOME/fonts/Craft` (`~/.local/share/fonts/Craft`) | `<store family folder>/<file>`. fontconfig reads the folder with its subfolders; `fc-cache` refreshes the cache afterwards when it is on `PATH`, off the calling thread. |
| Flatpak | the host's `$HOST_XDG_DATA_HOME/fonts/Craft`, else `~/.local/share/fonts/Craft` | Only when `/.flatpak-info` grants write access to that folder (`--filesystem=xdg-data/fonts:create`; its entries apply in order, `!name` revokes and `:ro` grants no write). No `fc-cache`: it would refresh the sandbox's cache. |
| Snap, a sandboxed macOS build (Mac App Store), Windows, Haiku, the web | none | `Error::Unsupported`. Windows (10 1809 and later, HKCU registration) comes later. |

`FontStore::new` has no folder; `with_os_fonts(OsFontFolder::at(dir, layout))` sets one, for tests and setups the rules above don't cover.

Only font files are copied. Licences stay in the store, which remains the record: copying a font into the same user's font folder isn't redistribution under the OFL, the Apache 2.0 licence or the UFL, and macOS doesn't expect text files in `~/Library/Fonts`.

Running apps see a new font at once or only after a restart, depending on the app (GTK under GNOME and Firefox at once, Chromium, Electron and Qt apps after a restart), so the interface tells the user that open apps may need a restart.

### Conflicts

`make_available` reads the catalog given to `with_catalog` (rescanned first if the font folders changed), else a fresh catalog of the process's environment, with the store locked. It refuses with `Error::Conflict`, changing nothing and naming each file with the PostScript name it has (`None` for a file in the way), when:

- a system font has a PostScript name of one of the family's faces or named instances (ignoring ASCII case). Names come from a catalog scan of each store file and from the index. fontconfig would otherwise pick the newer version system-wide, and macOS can't activate two fonts of one PostScript name;
- another store family's copy has one of those names (Roboto from Google Fonts and from a user's source, say);
- a file the store didn't add, another family's copy or a symbolic link sits where a copy would go.

The store's own copies are told apart by file identity (device and inode on Unix), so a copy the catalog reaches by another path (Flatpak shows the user's fonts again at `/run/host/user-fonts`) is still the store's. A system file identical to the store's (same sha256) is no conflict: the file isn't copied, and the ledger notes it as `present`, never to be deleted. A file at a copy's own place with the store's sha256 is taken into the ledger as a copy. Fonts of the same family name with other PostScript names (soft conflicts) are not checked yet.

### The ledger

`os-fonts.json` (version 1) lists, per family (name, source and store folder), every file: its store file name, its path, the store file's sha256, its PostScript names, how it is there (`copy`, or `present` for a system file) and its state (`adding` or `active`).

The store deletes a file only when the ledger lists it as a copy, at the very path the store puts that file (`path_for` of the family's folder and file name), with no symbolic link on the way, and with the sha256 the ledger gives. Any other listed path is left alone and reported. A ledger that can't be read or understood (damaged, an unknown value, another version) is kept as it is: queries treat the family as not available, and everything that would change the ledger, removals of the family from the store included, gives `Error::Ledger`.

A crash leaves nothing the store loses track of:

- A copy is listed as `adding` before it is made, written to `.craft-<name>.part` (a dot-file, which fontconfig and macOS skip), checked against its sha256, and renamed into place; then the ledger lists it as `active`.
- The next change settles every `adding` entry: the temporary file goes, a file with the new sha256 (or, while replacing a copy, the old one) stays listed as `active`, and any other is dropped from the ledger and left alone.
- A copy being deleted is renamed to `.craft-<name>.removing`, checked there and then deleted, or renamed back if it changed. One a crash leaves aside goes back at the next change.
- A withdrawal deletes the files first and rewrites the ledger after, so a crash in between leaves entries whose files are gone, which the next withdrawal drops.

### Limits

- The copies stay when the suite is uninstalled, unless the user withdraws them first.
- The catalog lists a copy as a system face beside the store's face. Both have the same family, style and PostScript name, so `Catalog::faces` keeps one: the store's, wherever the folders sort.
- Copies take disk space beside the store's files; there are no hard links, so a font editor saving over a copy can't change the store's file.

## Web

The index, the lookups, `verify_font` and `IndexPin` build for `wasm32-unknown-unknown`. The store folder and the HTTP client are native only; the web keeps fonts elsewhere.

## Tests

`cargo test -p craft-font-store --all-features`. The store tests run a small HTTP server on 127.0.0.1 that stands in for jsDelivr, raw GitHub and a user's host; no test uses the network. One ignored test fetches the pinned indexes and installs a Noto family over the network: `cargo test -p craft-font-store --all-features -- --ignored`.
