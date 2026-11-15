# craft-library

An app's Library: the user's own folders of documents, listed, searched and drawn as thumbnails,
behind one `library.*` command contract that every Craft app answers the same way. First asked
for in [storytold/deckcraft#958](https://github.com/storytold/deckcraft/issues/958). It depends
only on `serde_json`, and on no app.

The pane that shows it is in [`craft-library-egui`](../library-egui/README.md).

## What an app supplies

- **`Fs`**: its file access (`read_dir`, `stat`, `canonical`, `read`). Every folder listing and
  file read goes through it, so the app's own confinement applies to all of them. A test makes
  sure the crate never opens a file itself.
- **`Kind`**: what its Library holds: the file extensions, how to open a file, how many items it
  has (slides, pages, sheets), each item's title, hidden flag and text, and how to draw an item's
  thumbnail as PNG. Its `Nouns` (`slide`/`slides`, `page`/`pages`…) name the items in every answer.
  A kind whose every file is one item known from its name (a picture, an icon) also gives
  `item_from_name`: such files are listed, indexed and searched without being read, and a file is
  read only to draw its thumbnail or for the app's insert.

```rust
struct Pages;

impl craft_library::Kind for Pages {
    type Doc = MyDocument;
    fn extensions(&self) -> &[&str] { &["mydoc"] }
    fn nouns(&self) -> craft_library::Nouns { craft_library::Nouns { one: "page", many: "pages" } }
    fn open(&self, name: &str, bytes: &[u8]) -> Result<MyDocument, String> { /* … */ }
    fn title(&self, doc: &MyDocument) -> Option<String> { /* … */ }
    fn count(&self, doc: &MyDocument) -> usize { /* … */ }
    fn item(&self, doc: &MyDocument, index: usize) -> craft_library::Item { /* … */ }
    fn thumbnail(&self, doc: &MyDocument, index: usize, width: u32) -> Result<craft_library::Thumbnail, String> { /* … */ }
}
```

The app keeps the folder list in its own settings and passes it in.

## The commands

`Library<K>` answers, with the app's nouns as JSON keys:

| Command | Does |
|---|---|
| `library.folders` | the folders and how many files each holds |
| `library.addFolder`, `library.removeFolder` | change the app's folder list (associated functions) |
| `library.list` | the files of every folder or one, or one file's items |
| `library.search` | items whose title, text or file name matches |
| `library.thumbnail` | one item drawn as base64 PNG |

Inserting stays with the app, because only the app knows its document. `Library::file` and
`Library::doc` give an app's own `library.insert` the checked file and its opened document.

## Bounds

Everything read is hostile:

| What | Limit |
|---|---|
| Folders | 32 |
| Entries looked at per folder | 5,000 |
| Folder depth | 8 |
| File size | 256 MB (larger files are listed with an error) |
| Items per file | 2,000 |
| Links | never followed |

A file that can't be read is listed with its error. A reader that panics on a hostile file gives
an error for that file alone. Indexes, opened documents and thumbnails are cached within fixed
budgets, and a file whose size or time changes is read again.
