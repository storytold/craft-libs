# craft-library-egui

The Library pane in egui, over the `library.*` commands of
[`craft-library`](../library/README.md): a strip of tabs, a folder drop-down (Add Folder…,
Remove, Refresh), a search box, and the items of each file as a thumbnail grid. Clicking an item
inserts it. Its context menu can insert it as a whole item, as shapes, or one named element
(listed with the app's `library.shapes`). It depends on egui, `craft-ui` and `craft-library`, and on no app.

## What an app supplies

- **`Host`**: `act` runs a command that changes the document (`library.insert`, `addFolder`,
  `removeFolder`), `ask` runs a query (`list`, `search`, `thumbnail`, `shapes`). The app also gives
  its folder list, an optional folder picker, whether an item can be placed now, a status line,
  and draws any tabs of its own in `tab`.
- **`Labels`**: the app's words in English, its nouns (the same as the commands' JSON keys, never
  shown), its tabs (the shared `ITEMS` tab first, then the app's own) and the thumbnail aspect
  ratio. `insert_shapes` and `insert_element` are optional; a menu item is offered only when it is
  set. With `one_per_file` (pictures, icons), every file's one item goes in a single grid, named
  under its thumbnail, instead of each file having its own heading.
- **`Palette`**: colours from the app's theme, or `Palette::from_visuals`.

## Translation

Every word the pane shows goes through `Host::text`, which gets the English and gives back the
person's language; the default shows English. The words are the app's `Labels` and the pane's
own, which `words::ALL` lists for an app's catalog and its checks. A `{name}` placeholder
(`"Reading {read} of {total} files…"`) stays in the translation and the pane fills it in, so word
order can change. The `Labels` stay English constants: nothing has to be built per language.

## Layout

The pane scrolls its own grid within the part of it that can be seen, so the folder and search
controls stay in view, also when the app puts its panes in a scroll area.

```rust
let palette = craft_library_egui::Palette::from_visuals(ui.visuals());
craft_library_egui::show(ui, &mut app.library_state, &mut MyHost(app), &LABELS, &palette);
```

The pane holds no document and reads no file. It reads a few files per frame and draws only the
thumbnails in view, so a large Library fills in without stalling the window. At most 300 files
are laid out at once.
