# craft-browse

Browsing for Craft apps: folders and volumes, linked files, file thumbnails, and the widgets that show them. The app supplies its items through small read-only traits and gets events back; every listing, status check, search and decode runs as a job on `craft_host::queue::JobQueue`.

| Module | What it gives the app |
| --- | --- |
| `places` | The roots of a folder tree (standard folders, volumes, favourites, recents, the folder being browsed), the hidden and kept lists as one settings section (`PlacesSettings`), and `FolderCache`, lazy folder listings refreshed when `FileWatch` reports a change. Folder access goes through `Directories` (`SystemDirectories` on desktop, `FakeDirectories` in stories). |
| `links` | `LinkMonitor`: link statuses kept current as files change, each change reported once. The link rules themselves are `craft_docpath::links`. |
| `thumbs` | File thumbnails: the app implements `ThumbDecoder` (and optionally `ThumbStore`); with `ui`, `FileThumbSource` is a `ThumbSource` over files for `craft_ui::canvas::textures::Thumbnails`. |
| `ui::grid` | craft-ui's `list::TileGrid` and `ThumbnailSlider`, re-exported with the grid's stories: a virtualised thumbnail grid with groups, badges, rating, selection, check mode, reorder, rename (`start_rename`), marquee, hover scrub, keys and drag, over the app's `TileSource`; `paint_tile` draws one tile for widgets that lay out their own. |
| `ui::bin` | `MediaBin`: the project or media panel with List, Icon and Freeform views, search, sort, rename (`start_rename`), bin drops and a footer (`BinEvent::Footer` and `FooterDrop`), over the app's `BinModel`; the List view is a `craft_ui::list::Table`. |
| `ui::places` | `PlacesTree`: the folder tree with lazy children, reveal of the browsed folder and the keep, hide and reveal menu, and the app's own rows through `row_menu` (`PlacesEvent::Menu`). |
| `ui::compare` | `CompareTile`: one fitted image with zoom and pan shared across tiles and a caption strip; the two-pane and survey arrangements. |
| `ui::relink` | `LinkList::new(id_salt, links, statuses, &mut LinkListState)` (the Links panel; its sort, filter and columns are `LinkListSettings`) and `RelinkDialog` (find missing files, match options, folder search, candidates; the table layout comes back in `RelinkOutcome::layout`). |

`places`, `links` and `thumbs` work without egui; the `ui` feature adds the widgets and root re-exports of each widget, its labels and its style.
