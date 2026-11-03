# craft-platform

Small platform lifecycle services for Craft applications. Application state, settings, palettes,
document policy and native command routing remain in their current owners. Defaults add no
third-party dependencies. Enable only the services a host uses.

| Feature | Shared responsibility |
| --- | --- |
| default | Typed dialog outcomes; no backend or toolkit. |
| `dialogs` | Reserve waiting workers before eager UI-thread native picker creation; serial requests, failure delivery and completion wakeups. The backend factory belongs to the app. |
| `egui` | Combine with `dialogs` for the deferred-close guard or with `appearance` for native theme mapping. |
| `appearance` | Linux/BSD XDG desktop preference observation, owned cancellable subscriptions, bounded reads/helper capture, legacy compatibility and reconnect. |
| `browser` | Wasm download link/object URL ownership, cleanup on setup failure or abandonment, and cleanup timer registration before clicking. |


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
unconsumed application continuation. `DeferredClose::update` cancels close during pending work and
returns a request to retry the application's normal close policy afterward. It never grants
permission to discard work. Run those guards before `craft_quit::on_close_request` in `App::logic`.

## Desktop appearance

`appearance::Watcher` caches the desktop preference and wakes only on changes. Subscription precedes
the first read. ReadOne/legacy Read, the standard namespace and GNOME fallback are handled together;
malformed values retain the last valid answer, and explicit no-preference clears it. Disconnection
retains the last valid value until a fresh answer. A later service owner triggers a reread; session
reconnection has bounded backoff. Dropping cancels asynchronous work and waits at most 100 ms
for acknowledgement, joining only an already-finished worker. A synchronously blocked OS call or caller wake callback can outlive
the watcher: that exceptional worker is detached and retains its resources until it returns,
possibly until process exit. Wake callbacks must be nonblocking. No per-helper reader threads
are created.

App-selected Light/Dark/System, custom palettes, window decorations and unknown-theme fallback remain
application policy. `set_active(false)` suppresses wake notifications while continuing to cache.
Four existing custom-palette service hooks retain their current signatures and receive change-only
notifications even in manual mode; they do not change the chosen palette. Never infer Auto from the
egui theme preference when an app implements Auto by explicitly applying a Light/Dark palette.

`sync_native_window_theme(ctx, observed)` forwards the cached Linux/BSD desktop answer to native
decorations when egui's window policy is System, falling back to the current toolkit answer.
Its output hook runs after egui's automatic
window synchronization, including child-window output. Concrete Light/Dark preferences retain
precedence, absence of both answers releases the native pin, and `sync_window_theme = false` opts out.
It does not change styles or theme preference. Hosts whose custom palette is manual but whose
native window follows System keep observation active for the native window. Native decoration
rendering still depends on the window manager and requires visible acceptance testing.

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
decoration selection and window-button layout; appearance observation and native color hints here
do not choose a frame or draw an application header.
