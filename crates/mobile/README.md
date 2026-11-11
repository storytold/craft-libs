# craft-mobile

Runs a Craft app (egui + wgpu) on iPad and Android tablets inside a native shell: **SwiftUI** on
iPadOS, **Jetpack Compose** on Android. The app's Rust UI draws into a native view; files,
commands and panel state cross a small bridge, so native chrome (Liquid Glass toolbars,
inspectors, the system document picker) can grow around the canvas.

```
SwiftUI / Compose shell  ──actions (JSON)──▶  craft-mobile host  ──▶  your CraftApp
  CraftKit / craftkit    ◀─requests─────────  (egui + wgpu)          (eframe::App + 4 methods)
                            pick files · export bytes · panel state
```

## What's shared, what's per app

| Shared (this crate)                                  | Per app (~60 lines Rust, ~15 Swift, ~20 Kotlin) |
|------------------------------------------------------|---------------------------------------|
| `src/host.rs`: egui + wgpu into a native surface, eframe's frame order, touch → pointer | `impl CraftApp`: build the app, map actions to its commands, describe its items |
| `src/platform.rs`: file pick / save in place / export | Wire its file services to `Platform` (pick, export) |
| `src/ios.rs`, `src/android.rs`: the C and JNI interfaces, all the `unsafe` | `craft_mobile::export!(MyApp);` |
| `swift/CraftKit`: canvas, toolbar, items inspector, picker/exporter, open-from-Files | `@main` with `CraftShell(documentTypes:)` and an Info.plist |
| `android/craftkit`: the same in Compose + Material 3 | A `ComponentActivity` and a manifest |

## Bringing an app over

1. **Crate.** Add `apps/<app>-mobile` with `crate-type = ["staticlib", "cdylib", "rlib"]`, a
   dependency on `craft-mobile` (by git, like the other craft-libs crates), and its own `[lints]`
   table (`unsafe_code = "deny"`: the one exported start symbol comes from `export!`). The native
   shells build CraftKit and craftkit from this crate's `swift/` and `android/` folders, so keep a
   craft-libs checkout at the same commit beside the app (PhotoCraft's scripts look for
   `../craft-libs/crates/mobile` and take `CRAFT_MOBILE_DIR` to point elsewhere).
2. **`CraftApp`.**
   - `create`: build the app exactly as the desktop does, minus desktop-only services.
   - `wgpu_configuration`: only if the app needs adapter limits beyond egui-wgpu's defaults.
   - `open_file`: push into the app's inbox (most apps already have one for the web build).
   - `action`: `Open/Save/Undo/Redo` → the app's menu ids, run *as the menu runs them*
     (dialogs and enable rules included); `Select`/`SetVisible` → its item commands;
     `Command` → anything by id.
   - `snapshot`: the active document and its items (layers, tracks, pages), top first.
   - Optional: `busy` (work the user waits for, such as a save encoding on a worker: the shells
     keep the app running in the background meanwhile), `power` (hot or saving power: lower
     preview costs), `suspend` (start writing recovery data on workers and return at once; stay
     `busy` until it's on disk), `trim_memory`, `stylus`, `written`. Show paths to the user with
     `craft_mobile::with_file_names`, so a document path reads as its file name.
3. **Files.** Point the app's file-dialog service at `Platform::pick_files` (answer with
   contents, as the web build does) and its write service at `Platform::save`. A file the shell
   can write back (picked, or opened in place from Files) has a `PickedFile::document_path`
   (`inplace://<id>/<name>`): open it as a file from disk with that path, and `Platform::save`
   to it writes the original without a sheet; `CraftApp::written` hears how it went, and only
   then is the document saved. Any other path goes to the exporter, which can be cancelled
   silently: never mark the document saved from it.
   Photos and sharing: `Platform::pick_photos` shows the system photo picker (the photos arrive
   through `open_file`, as copies) and `Platform::share` the share sheet, for the app's own
   menu items (PhotoCraft's File › Import from Photos… and Share…).
   **Clipboard:** copies go out as they do on the desktop (egui's copy commands,
   `Platform::copy_image`). The system clipboard is read only when the user pastes (iPadOS may
   ask them to allow it): ⌘V / Ctrl+V, egui's `ViewportCommand::RequestPaste`, or
   `Platform::request_paste` from the app's own Edit › Paste. The answer comes as egui's
   `Event::Paste` / `Event::PasteImage`, with `Platform::pasted` holding it unpremultiplied for
   that frame; while the clipboard still holds the image the app copied it isn't read back
   (`Pasted::OwnImage`), so the app pastes its own copy at full fidelity.
4. **`craft_mobile::export!(MyApp);`**
5. **iPadOS.** `ios/MyApp.swift` (`WindowGroup { CraftShell(documentTypes: [...]) }`),
   `ios/Info.plist` (document types; copy PhotoCraft's), `ios/build.sh` calling
   `swift/build-ios.sh <package> <dir> <Name>`.
6. **Android.** See `android/` (craftkit) and PhotoCraft's `apps/photocraft-mobile/android`.

PhotoCraft (`storytold/photocraft`, `apps/photocraft-mobile`) is the reference app.

## `unsafe`

The workspace forbids `unsafe_code`; this crate can't, so it doesn't inherit the workspace lints.
Its own `[lints]` table denies `unsafe_code` and copies the workspace's clippy set (plus
pedantic, `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`). Each use is an item with
`#[expect(unsafe_code, reason = "...")]`, and each `unsafe` block has a `SAFETY` comment: wgpu
surfaces made from the shell's raw view handle (`host.rs`, `driver.rs`), the exported symbols
(`export!`), and the C (`ios.rs`) and JNI (`android.rs`) entry points with the buffers that
cross them.

## Development

- iPad simulator: `ios/build.sh sim`, then `xcrun simctl install booted <app>` and launch. The
  simulator can't be tapped or handed files from a script, so CraftKit reads development
  variables (pass them with `SIMCTL_CHILD_` prefixed): `CRAFT_DEMO_TAPS="x,y;…"` (points),
  `CRAFT_DEMO_OPEN="/path/a.png"` (opened in place: Save writes it back), `CRAFT_DEMO_OPEN_COPY`
  (opened as a copy), `CRAFT_DEMO_ACTIONS="open;save;undo;toggle-first-item;command:file.saveAs"`,
  `CRAFT_DEMO_SHOW_ITEMS=1`, `CRAFT_DEMO_EXPORT=save|share|photos`, `CRAFT_DEMO_THERMAL=serious`,
  `CRAFT_DEMO_LOW_POWER=1`; `CRAFT_DEMO_STEP=0.05` and `wait:<s>` time the actions, and
  `background`/`foreground` act as the scene leaving and returning. The full list is on
  `CraftDevelopment` in `CraftCanvas.swift`.
- Android emulator: real taps work (`adb shell input tap`); `cmd thermalservice override-status 3`
  and `cmd power set-mode 1` exercise the power state.
- iOS 27 SDK apps must use the UIScene life cycle; winit 0.30 can't, which is one reason the
  shell owns the app and this crate never uses winit.
- Tests: `cargo test -p craft-mobile` (pure logic: actions, picker replies, sizes, touch →
  pointer). The FFI layers are exercised by running an app.
