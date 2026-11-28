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
- **The craft-ui runtime**: the folder drop-down is craft-ui's `Select`, whose list opens only in
  a window that hosts the runtime (`shell::embed`'s install and per-frame hooks, or `shell::app::run`).

Two `Host` methods are optional, and do nothing by default:

- **`menu_inserts`**: inserts of the app's own, after the pane's in an item's context menu. Each is a
  label and the params it adds to `library.insert`: GridCraft's Place in Cell is
  `{"placement": "cell"}`. They are offered when the app can place an item now.
- **`drag_hint`**: the app takes items dropped on it. An item's cell can then be dragged with a
  mouse or pen, carrying a `Dragged { file, item }` as egui's drag-and-drop payload, with its
  thumbnail beside the pointer, and its tooltip says what the hint says. A click still inserts,
  the wheel and the scroll bar still scroll the grid, and a finger's drag scrolls it as before.
  The app takes the payload where it is let go (`Response::dnd_release_payload::<Dragged>()`) and
  inserts it with `Dragged::params` and where it landed. Escape, or letting go anywhere else,
  drops nothing.

## Translation

Every word the pane shows goes through `Host::text`, which gets the English and gives back the
person's language; the default shows English. The words are the app's (its `Labels`,
`menu_inserts` labels and `drag_hint`) and the pane's own, which `words::ALL` lists for an app's catalog and its checks. A `{name}` placeholder
(`"Reading {read} of {total} files…"`) stays in the translation and the pane fills it in, so word
order can change. The `Labels` stay English constants: nothing has to be built per language.

## Layout

The pane scrolls its own grid within the part of it that can be seen, so the folder and search
controls stay in view, also when the app puts its panes in a scroll area. Only the rows of the grid
in view are drawn: a row out of view only takes its space.

```rust
let palette = craft_library_egui::Palette::from_visuals(ui.visuals());
craft_library_egui::show(ui, &mut app.library_state, &mut MyHost(app), &LABELS, &palette);
```

The pane holds no document and reads no file. It reads a few files per frame and draws only the
thumbnails in view, so a large Library fills in while the window keeps drawing. A frame starts no
other file once its 12 ms are spent, but it waits for each call to the app it makes: one slow file,
or listing the folders, holds that frame. It reads and lays out at most 300 files, and stops
reading once they hold 10,000 items; the other files are listed but not read or laid out, and the
pane asks to choose a folder or search to see them. A search also runs a few files per frame
(`library.search` with `file`), files that couldn't be read included, showing what it has found so
far. A query is at most 256 characters, as `library.search` takes. Each row of the grid keeps the
height its thumbnails gave it, even when their textures are let go. Refresh lists the folders
again, draws the thumbnails again and has the Library read again the files that couldn't be read;
adding or removing a folder also draws the thumbnails again.
