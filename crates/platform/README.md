# craft-platform

Small platform lifecycle services for Craft applications. Application state, settings, palettes,
document policy and native command routing remain in their current owners. Only the services a host enables
pull in heavier dependencies.

| Feature | Shared responsibility |
| --- | --- |
| default | Typed dialog outcomes; `PlatformText`; `open` (see below); no backend or toolkit. |
| always (not wasm) | `dialogs`: reserve waiting workers before eager UI-thread native picker creation; serial requests, failure delivery and completion wakeups. The backend factory belongs to the app. |
| always | `From<appearance::Reading> for craft_theme::SystemAppearance`, so a host hands the OS answer to `Base::resolve`. |
| `pointer` | Move the system pointer (`pointer::warp`), installed by the app as craft-ui's `Scrub::install_warp` service for a scrub's screen-edge wrap; macOS only, `false` elsewhere. |
| `appearance` | Linux/BSD XDG desktop preference observation (colour scheme, contrast, reduced motion, GNOME's text scale, the window-button layout of every desktop); reduced motion on Windows and macOS; browser contrast and motion media queries; owned cancellable subscriptions, bounded reads/helper capture, legacy compatibility and reconnect. On Linux and the BSDs it also uses x11rb (the X settings manager and the window manager's name), roxmltree (window managers' XML files) and libc (`O_NONBLOCK`, to open a settings file without waiting). |
| `browser` | Wasm download link/object URL ownership, cleanup on setup failure or abandonment, and cleanup timer registration before clicking; the file picker and save. |
| always | `clock::Clock`: monotonic seconds (`web-time`, so also in the browser) for animation, autosave and deadlines, with a manual mode tests set and advance; `monotonic_ms`, `unix_ms`. |
| always | `pacing`: `FrameCap` (desktop frame-rate cap without vsync), `WindowVisibility` (shown or hidden), `WorkBudget`/`WorkSlice` (background work per call, paused or sliced while hidden). No egui. |
| always | The app log: `log` records to standard error and `<dir>/<app>.log` with `RUST_LOG`-style filters, rotation to `.1`/`.2`, per-instance logs while another instance runs, a size cap; `logs::crash`: the panic hook, crash log (redacted message, written to standard error only when no logger takes errors), last panic for the error window, and `catch`. |
| always | `session::Session` and `SessionKeeper`: the documents open at the last quit and where the user was in each, stored in the app's settings (serde) for a "Reopen Last Session" preference; `to_reopen` skips what is already open; a stored list longer than 64 documents is cut. No egui. |
| `usage` | `usage::sample`: the process's resident memory and the memory it may still use (`MemorySample`, `None` when unknown), for the status readout's Scratch and Efficiency items. Linux reads `/proc`; other platforms return `None`. |
| always | `open::FileIntake` with `IntakeSender` (OS open requests, second-instance hand-over, the web host and `IntakeSender::dropped` and `quit` feed it; `open::apple_events` under `apple-events` on macOS): one queue, in arrival order, for files the OS asks the app to open (`open`, as `Intake::Open`), files it asks to open for one of the app's own actions such as combining (`open_for`, as `Intake::OpenFor` with the action's name; made absolute and sorted like `open`), files that arrive as bytes with no path (`open_bytes`, as `Intake::Bytes`), files dropped on the window (`DroppedFile`, `DropTarget`: canvas, tab strip or window) and the OS asking the app to quit (`Intake::Quit`). Each batch stays separate, empty input is ignored, and every sender call wakes the UI. `natural_cmp` and `same_file` for opening order and duplicates; `reveal` (on the caller's thread; on Linux and BSD it can wait up to 2 s for the file manager, so hosts run it on their `Jobs`), `open_external` (on the web, a new tab without `window.opener`), `open_with` and `is_application`; the shared open-error texts. No egui. |
| always | `autosave::Autosave` (the timer: which documents are due, results by revision), `recovery_dir`, `adopt_legacy_recovery` and `RecoveryStore`: the recovery folder of copies with a JSON sidecar each (`RecoveryEntry`, the fields craft-ui's `RecoveryDialog` lists), `AUTOSAVE_FAILED`. The app serializes its documents. |
| `clipboard`, `clipboard-system` | `clipboard` has no system crates: `Clipboard`, `EventClipboard` (with `LateOffers`), `MemoryClipboard` and the probe. `clipboard-system` adds `SystemClipboard`. `clipboard::Clipboard` over a `ClipboardBackend`: every copy and paste goes through it, with size limits (`ClipboardLimits`, `TooLarge`), a fence that keeps synthetic (automation) input from replacing what the user copied, and `holds_ours` to tell the app's own copy from another app's. `SystemClipboard` (arboard) reads text, HTML, RTF, one bitmap and copied files, and writes one offer; `EventClipboard` is the browser's, fed from paste events (`flavors_from_event`). `EventClipboard` keeps an offered paste for the pass it was offered in and counts passes itself (the pass clock the host gives it), so a menu Paste in a later pass finds nothing and the app has nothing to call at the end of a pass. A write reports which flavor the backend wrote; `holds_ours` asks the backend's owner marker first (`ClipboardBackend::owns`) and compares that flavor's content only when the backend cannot tell. The web writes only the best flavor (eframe makes each copy command its own `navigator.clipboard` write, so a second replaces the first). `has` and `formats` never read or decode: they list the formats on offer (`offer::formats`: Windows' and macOS' formats, Wayland's MIME types, X11's `TARGETS`); a bitmap counts only as `image/png`, the one arboard reads on Linux. `ClipboardProbe` asks a second handle on a worker thread whether Paste has anything, so a slow owner never stalls a frame. Bitmaps are read as straight RGBA8, not PNG: the `BITMAP` and `RGBA8` reads return an `image/x-rgba8` flavor (width and height as little-endian `u32`s, then the pixels) and `read_image` returns `ClipboardImage`; the crate has no PNG encoder, and `SystemClipboard::decoder` takes the app's PNG decoder for writes. SVG and PDF flavors are neither written nor read by `SystemClipboard`; RTF is read, and RTF and MathML are written beside arboard's flavors (a write of only unsupported flavors fails with `Other`); a bitmap offered only as `image/jpeg` or `image/bmp` on Wayland reads as `None`. |
| always | `watch::FileWatch` and `Changed`: open files that changed on disk. One worker thread (`craft-file-watch`) compares each file's metadata (`FileStamp`) every 250 ms and sleeps while nothing is watched; a changed file is reported once it has settled and, with `ReadPolicy::Whole`, looks complete. The app's own saves are recognised by `known_now`; `disk_differs` answers at Save time; `take_changes` runs on the frame the worker's `Wake` asks for. |
| always (native) | `default_app::make_default`: the Preferences button that makes the app the system default for its file types. Windows opens Settings › Default apps (`SettingsOpened`), Linux and BSD run `xdg-mime default` (`MadeDefault`, refused inside Flatpak or without an installed desktop entry), macOS answers `Unsupported`. `path_from_arg` and `rejoin_split_paths` read the paths a launcher hands the app. Only a button may call it. |
| always (native) | `temp::TempDir` and `TempFiles`: private temporary folders created exclusively with owner-only permissions (`0700`, files `0600`, `create_new`), named with the process id and a counter, removed on drop. |
| always (native) | `owner_only`: files only their user can read on Windows, as mode `0600` makes them on Unix. `restrict_to_owner` gives a file an access list of the user and SYSTEM only, `create_private_dir` and `create_private_file` create a folder or file with that access list from the start, `read_with_sddl` reads a file with its security descriptor through one handle, and `foreign_access` says why a descriptor is not the user's alone. They run the tools Windows ships (`whoami`, PowerShell, `icacls`, from System32) because the security API would need `unsafe`. A run of a tool that takes longer than `TOOL_LIMIT` (10 s) is ended, and the call fails with `ErrorKind::TimedOut`. |
| always | `PlatformText`: a user-visible message of a platform service (id, English text with `{name}` placeholders) that the app looks up in its own catalog. The open, clipboard, watch, default-app and temp services and craft-dialogs' save flow use it. No words table lists them yet, so the missing-translation list does not show them. |
| `screen` | Screen capture for the screen colour picker (`craft_platform::screen`). |
| `apple-events` | Apple Events requests on macOS (`open::apple_events`), through `fmv-macos-events`. |


## Native dialogs

`dialogs::wait_prepared(create, deliver)` reserves the worker first. Call it on the native UI thread.
`create` must call `pick_file`, `save_file`, or the equivalent backend method **before returning its
future**. Putting that call inside an async block delays native sheet creation until the worker
polls it. Worker reservation failure never opens the sheet. No live picker has a timeout.

`NativeDialogs<Request, Answer>` provides an optional serial queue. Pump it on the event-loop
thread before consuming answers and after code that can request a picker. Replies become visible
only after the pump releases transport ownership, so a continuation can enqueue the next picker.
Dropping a stale reply does not release the actual picker. A host-specific preflight is optional;
it runs off-thread and must be bounded. Unknown capability permits a picker. Proven absence may
request an application-owned fallback. Custom save-format sheets and portal choices remain intact.

`Outcome::Selected`, explicit `Cancelled`, `Failed` and `Indeterminate` have different evidence.
An rfd `None` is `Indeterminate`; it cannot prove user cancellation rather than backend failure.
Disconnected workers are failures. Apps own localization, document identity/revision checks, filters,
actual save receipts, chained continuations and the decision to close.

Retain the actual parent window through queued requests, preflight, the native picker AND any
unconsumed application continuation. A close request that arrives meanwhile never grants permission to
discard work: the app runs its normal close policy again afterward.
`appearance::Watcher` caches the desktop preference and wakes only on changes. Subscription precedes
the first read. ReadOne/legacy Read, the standard namespace and GNOME fallback are handled together;
on GNOME the "default" (Light) setting and portal value 0 read as light; elsewhere 0 is no preference.
Malformed values retain the last valid answer, and explicit no-preference clears it.
`SystemReader::read()` gives colour scheme, contrast and reduced motion in one call on every target: the watcher on Linux/BSD, the browser's contrast and motion queries on the web, and the colour scheme the host reports from its windowing integration (`report_theme`; the watcher's answer wins where it has one). macOS and Windows have a reduced-motion source and no contrast source. `SystemReader::generation()` rises on each change a source reports (the watcher's theme, contrast or motion, a reported theme), the change notice the craft-ui runtime and apps re-resolve themes on.

`Watcher::wait_first(timeout)` blocks until the first answer, a known absence, or the timeout (about 250 ms in an eframe creator), so the first frame uses the desktop theme. Disconnection
retains the last valid value until a fresh answer. A later service owner triggers a reread; session
reconnection has bounded backoff. Dropping cancels asynchronous work and waits at most 100 ms
for acknowledgement, joining only an already-finished worker. A synchronously blocked OS call or caller wake callback can outlive
the watcher: that exceptional worker is detached and retains its resources until it returns,
possibly until process exit. Wake callbacks must be nonblocking. No per-helper reader threads
are created.

App-selected Light/Dark/System, custom palettes, window decorations and unknown-theme fallback remain
application policy. `set_active(false)` suppresses wake notifications while continuing to cache.
Four existing custom-palette service hooks retain their current signatures and receive change-only
notifications even in manual mode; they do not change the chosen palette.

### Window-button layout

`Watcher::caption_layout()` is the window-button layout of the running desktop, as a
`craft_host::caption_layout::LayoutReading`: the value in its source's own grammar, which source gave it, and
where it was read. Each source keeps its own answer and the best one present decides, in a fixed order, so a slow
better source replaces a quick worse one and never the other way round:

1. the settings portal's standard `org.freedesktop.appearance` `button-placement` (`(asas)`, leading and trailing);
2. the window manager's own setting: GNOME's, Cinnamon's or MATE's `button-layout` exported by the portal, then read
   with `gsettings get` and followed with one owned `gsettings monitor`; xfwm4's Xfconf property over D-Bus;
3. the window manager's configuration file, looked at once a second while the app is active: KWin's `kwinrc` with
   the system's files and their `[$i]` locks, xfwm4's channel file, Openbox's or labwc's `rc.xml` (Budgie's in
   `budgie-desktop/labwc`);
4. GTK's `Gtk/DecorationLayout` from the X settings manager (XSETTINGS 0.5, on an X11 session, with the manager's
   selection followed; not on MATE, whose manager does not follow Marco's setting), then GNOME's key through the
   portal where the desktop's own session keeps it in step with its window manager (Plasma's GTK bridge, Xfce 4.20's
   settings daemon, Budgie's labwc bridge). On LXQt, Cinnamon and MATE, and for KWin or xfwm4 outside their own
   sessions, GNOME's key holds GNOME's default and is not read;
5. GTK's `gtk-decoration-layout` in `gtk-3.0/settings.ini`.

Which window manager's setting counts follows `craft_host::caption_layout::LayoutDesktop`: the session's names,
the X11 window manager's EWMH name on an X11 session, LXQt's chosen window manager and labwc's `LABWC_PID`. It is
found once, for every source: the portal is asked for that window manager's keys only. Nothing of this runs before
the first `caption_layout()` call: no X11 connection, file poll, helper or portal read of the layout keys, so an
app that never asks pays for none of them. Only an X11 session's X server is asked, on a short-lived thread with a
500 ms deadline: a connection on a Wayland session could start Xwayland. Every source shares the watcher's one
worker, so none waits in a blocking call: every file the worker reads (the window managers', GTK's and the
reduced-motion fallback's) is opened without waiting and read only when it is a regular file of at most 1 MiB
(256 KiB for the motion fallback's), so a FIFO, device or socket in its place is no answer. In a Flatpak `gsettings`
is not asked (it reads the sandbox's own settings), system configuration folders are the runtime's and not read,
and a window manager's built-in default is not taken for a file out of the sandbox's reach; Xfconf needs
`--talk-name=org.xfce.Xfconf` and a file its own `--filesystem` grant. GNOME's key is in step only where the
desktop's bridge runs, which the session's names can't tell; without the file and the bridge a sandbox may get
GNOME's default. A window manager started with another configuration file (Openbox's `--config-file`, labwc's
`-C`, labwc's `--merge-config`) is not followed, except Budgie's known folder. A lost portal keeps the last answer; a new
portal that has no such key gives way to the next source. `None` when no source answers; the caller keeps the
desktop's default. A change runs the wake callback; `wait_first` does not wait for it. `set_active(true)` looks at
the files at once.

`Watcher::button_layout()` stays for older callers: GNOME's `org.gnome.desktop.wm.preferences` `button-layout`
string as the portal (or `gsettings` once when the portal has no answer) reports it.

### Contrast and reduced motion

`Watcher::contrast()` (`Contrast::Standard` or `High`) and `Watcher::reduced_motion()` follow the
portal's `org.freedesktop.appearance` `contrast` and `reduced-motion` keys. Reduced motion
combines the portal key, KDE's animation setting and GNOME's `org.gnome.desktop.interface`
`enable-animations`: any source that turns animation off wins. The portal's three keys (colour
scheme, contrast, reduced motion) are read beside each other, so `wait_first` returns with them,
and a change runs the same wake callback. `None` means no answer yet or no such key; where the
portal has none, the desktop's settings files are read (`appearance/files.rs`), each opened without
waiting and only when it is a regular file; the `gsettings` fallback doesn't read them.
On macOS and Windows a worker polls the system's reduced-motion setting once a second
(`NSWorkspace`, `UISettings`) and runs the wake callback when it changes. On the web, `browser_contrast()` and `browser_reduced_motion()` evaluate the
`prefers-contrast: more` and `prefers-reduced-motion: reduce` media queries on each call, with
no change notification.

## Browser downloads

`PreparedDownload::{bytes,blob}` owns a detached anchor and URL until click or drop. Call `click`
with the application's positive URL lifetime; setup/attachment/timer failure releases resources
without clicking. A successful return means a click was issued, not that the browser saved a file.
Applications own names, MIME types, pacing, capture hooks and user activation. File System Access
and print iframe lifetimes remain separate. This service has no native filesystem semantics.

## Integration

Enable only the services a host uses. Applications retain palettes, settings, document identity,
close policy, filters, names, MIME types and native command routing. Existing craft-settings,
craft-atomic, craft-print, craft-docpath and craft-quit remain canonical. `craft-window` owns
decoration selection and window-button layout; appearance observation here
do not choose a frame or draw an application header.
