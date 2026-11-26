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
| Off until the user turns it on | `FontStore::network()` is `Network::Off` until `enable_network()`. The setting is `settings.json` in the store folder, so it holds for every app. A host may give its own default for a user who hasn't chosen (`FontStore::with_default_network`); it is never saved, and the user's `enable_network()` or `disable_network()` beats it. A `settings.json` that exists but can't be read, is too large or doesn't parse reads as `Network::Off`, whatever the host's default, and the next saved setting starts from that. |
| Only the user turns it on | `FontStore::for_user` is for the app's own interface; call `enable_network()` from a setting or from the user's Install click. `FontStore::for_agent` (MCP, CLI, scripting) gets `Error::NotAllowed`. An agent can install while the user has the network on. |
| No requests at startup | Nothing in the crate makes a request on its own. Only `fetch_indexes` and the installs do, and they return `Error::NetworkOff` without one while the network is off. |
| Optional preview service | Off unless set: craft-libs has no default address. `CRAFT_FONT_PREVIEW_URL`, else the `preview_service` field of `settings.json` (`FontStore::set_preview_service`, user only, `Error::NotAllowed` for an agent), else the host's default (`FontStore::with_default_preview_service`), which is never saved, so clearing the setting brings it back and other apps sharing the store don't see it. `FontStore::preview_service()` returns the one in effect and `preview_service_source()` says whether it is `Env`, `Saved`, `Default` or `None`; the variable's name is `PREVIEW_SERVICE_ENV`. A `settings.json` that exists but can't be read or parsed means no saved service and no host default. Installs and `legal_text` try the service's mirror only for curated families (the built-in sources, `Family::is_curated`), so the hashes of files from sources the user added are never sent to it. https only; http is accepted only for the addresses `127.0.0.1` and `::1`, never for a host name, which a resolver can point off the machine. A trailing slash is dropped. Example: `CRAFT_FONT_PREVIEW_URL=https://fonts.example.com`. |
| Previews setting | `previews` in `settings.json`, on by default: `FontStore::previews_enabled()`, `set_previews_enabled(bool)` (user only, `Error::NotAllowed` for an agent). The app fetches previews from a set service only while it is on; installs ignore it. |
| Preview client | `preview`: subset keys and URLs (`PreviewKey`, `Codepoints`, `SampleHash`), the disk cache (`DiskCache`, least recently used out past 50 MB) and the pool that fetches and decodes WOFF2 subsets on threads of its own (`Previews`). The pool reads the service, the previews setting, the client and the cache folder through `PreviewStore`, which the font manager's `Installer` builds on. It requests nothing without a service or with previews off; cached subsets still load. |
| Craft apps only setting | `craft_apps_only` in `settings.json`, off by default: `FontStore::craft_apps_only()`, `set_craft_apps_only(bool)` (user only, `Error::NotAllowed` for an agent). It is the default for families with no choice of their own (`available_to_all` below): while it is off, craft-font-manager copies each family it installs into the OS user font folder. The setting only records the choice; installed families keep theirs. |
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
previews/            preview subsets (WOFF2) the preview pool caches, at most 50 MB
<family folder>/     the font files and the licence
language-packs/      installed Language packs, one folder per locale
language-packs.json  which Language packs are installed
.staging/            downloads being installed
.lock                held while the store changes
```

A Google Fonts family's folder is google/fonts' own folder name (`lato/`); another source's is `<source>/<family>` (`notofonts/noto-sans-cjk-jp/`, `user-fonts-example-org-myfonts-bfeb06e2/some-family/`).

`manifest.json` lists each installed family: its name, source, folder, licence and licence file, per file its name, path in its repository, size, sha256, the index commit it came from, and its PostScript names, and `available_to_all`: the user's choice to copy the family into the OS user font folder (`true`) or keep it in Craft apps only (`false`), absent when none is recorded. `set_available_to_all_choice(source, name, on)` records it (user only); it doesn't copy or withdraw. An entry is one family of one source, so the same name from two sources is two entries. A manifest that can't be read is moved aside to `manifest.json.bad` and counts as empty.

| Call | What it does |
|---|---|
| `installed()`, `installed_family(source, name)`, `is_installed(family, file)` | Read the manifest. |
| `remove_family(source, name)` | Withdraws the family's copies from the OS user font folder (user only when it has some), moves the family folder out at once, updates the manifest, then deletes the files and the licence. `remove_files` withdraws the copies of the files it removes. |
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

## Copies in the OS font folder

Fonts in the store are for the Craft apps only. A family can also be copied into the OS user font folder, for every other program of the current user (the API calls this making it available to all apps). The calls below work per family; whether an app copies every family the user installs or lets the user choose per family is the app's decision. Only the user's store (`for_user`) changes the OS folder.

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
| macOS | `~/Library/Fonts` | `Craft-<family folder>-<file>`, side by side. macOS activates them for every app of the user. |
| Linux and the BSDs | `$XDG_DATA_HOME/fonts/Craft` (`~/.local/share/fonts/Craft`) | `<store family folder>/<file>`. fontconfig reads the folder with its subfolders; `fc-cache` refreshes the cache afterwards when it is on `PATH`, off the calling thread. |
| Flatpak | the host's `$HOST_XDG_DATA_HOME/fonts/Craft`, else `~/.local/share/fonts/Craft` | Only when `/.flatpak-info` grants write access to that folder (`--filesystem=xdg-data/fonts:create`; its entries apply in order, `!name` revokes and `:ro` grants no write). No `fc-cache`: it would refresh the sandbox's cache. |
| Windows 10 1809 and later | `%LOCALAPPDATA%\Microsoft\Windows\Fonts\Craft` | `<store family folder>/<file>`, each listed as `<full name> (TrueType)` (or `(OpenType)` for CFF outlines) under `HKCU\Software\Microsoft\Windows NT\CurrentVersion\Fonts`, loaded into the running session with `AddFontResourceW`, then `WM_FONTCHANGE` is broadcast off the calling thread. A registry value that already names another file is a conflict. Withdrawing unloads the font, deletes the value, then deletes the file; a file another program still holds can't be deleted, so the withdrawal fails and the ledger keeps the copy for the next try. The Win32 calls are in `craft-font-os-windows`. |
| Snap, a sandboxed macOS build (Mac App Store), a packaged (MSIX) Windows build, older Windows, Haiku, the web | none | `Error::Unsupported`. |

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
