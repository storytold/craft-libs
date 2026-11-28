# Shared installation progress for the Craft Apps

This proposal gives the ArtCraft Launcher and the Craft Apps' own updaters a common
installation status view. Each app supplies its existing name, icon and package identity.
The first consumer is FigCraft in the Launcher. Other apps can adopt the view without
changing their release assets or asking users to reinstall.

## Shared view and backend

- Build the compact egui view alongside the shared updater UI in
  [`crates/update`](../crates/update). Use the existing `App` descriptor and suite
  theme tokens for the icon, title, fonts and controls. Fit the approved Windows
  layout at 400 × 150 logical pixels under ordinary settings, with room to grow for
  large text and longer translations. Follow host window conventions on Linux and macOS.
- Reuse the existing `JobPhase` and result handling. Show the current phase with an
  accessible status label. Draw the square-ended bar for measured progress, such as
  downloaded bytes with a known total. Ready, waiting, hashing, unmanaged install
  phases and terminal screens show status text with no empty track. Preserve the
  installer's real error and log path in Details.
- Treat cancellation as a backend capability. Downloads can use the existing
  pause or cancel controls. A queued job can be cancelled before it enters the
  installer. Once a native transaction starts, defer to its cancellation support.
  Keep observing the job until the backend returns. Report cancellation, failure,
  success and restart required as distinct results. Closing the view must not kill
  an installer or claim rollback before the backend confirms it.
- Keep discovery, package selection, version checks, download verification and
  installation in `craft-update`. The view consumes job state. Apps that already share
  the updater use the same view and need no separate installer profile. Web builds
  continue without a native updater.

## Platform routes

| Platform | Installation and update path | Progress in the shared view |
| --- | --- | --- |
| Windows | Use the existing MSI and `msiexec` path, including its permission prompt and transactional rollback. Follow [the Windows install policy](installation-policy.md) for scope and destination. Preserve the installed copy's context and custom location on upgrade. | Show download bytes and queue status in the shared view. Defer to the native MSI UI during the transaction. Standalone MSI UI authoring remains in [#157](https://github.com/storytold/craft-libs/pull/157). |
| Linux | Route distro packages through the existing package-manager and polkit paths. Respect the source and scope of a Flatpak and leave externally managed packages to their owner. Keep the existing launcher-managed AppImage and sandbox tarball routes. | Use byte progress for launcher downloads. Use phase text for manager work and file staging unless the backend reports a real total. |
| macOS | Keep the release DMG and signed app bundle. Retain read-only attach, bundle identity and signature checks, existing location and staged replacement. The signing and notarization requirements for release artifacts stay in the release pipeline. | Show download bytes, then text for opening the image, checking the bundle and installing it. Retain the OS permission and trust experience. |

The FigCraft prototype used Windows Installer record callbacks to draw live MSI
progress. A later Windows-specific investigation can connect that signal to the
shared view through a reviewed adapter. Current `craft-libs` workspace policy forbids
new unsafe code, so direct Win32 bindings from the prototype cannot be moved as-is.
A native callback path needs an approved adapter boundary and parity with the
existing MSI route for elevation, cancel and rollback, repair, busy files,
restart-required results and unattended installs. Keep #157 independent.

## Optimizing Linux and macOS

Measure the existing backend before adding work to it. Record download, checksum,
queue wait, staging/copy and commit times separately. On macOS also record DMG attach,
bundle verification and detach. Compare cold and cached runs on the same release and
hardware, including a large Craft app. Record wall time, downloaded bytes, peak RSS
and peak temporary disk use. Measure the window's first paint before starting disk or
network work. Use local diagnostic logs for the benchmark.

- **Both platforms:** reuse the existing HTTP resume and verified cached-file path.
  Keep transfers and disk work off the UI thread. Coalesce progress updates so the
  UI sleeps while idle. Reuse the verified local payload across the operation.
  Optimize repeated reads only after measuring them and preserving final digest checks.
- **Linux:** retain the package manager's download cache, dependency resolution and
  per-manager write lock. For managed AppImages, measure the cost of the verified
  download followed by a same-filesystem staging copy. Compare a supported reflink
  for that copy against the current copy path before adopting it, with ordinary
  copy as fallback. Keep executable permissions and the atomic rename. Test
  interruption, cross-device paths and filesystems without reflinks.
- **macOS:** use the existing disk-image and signature queues, which serialize
  conflicting work. Measure queued time separately from attach, signature checking
  and copy time. Tune download concurrency around the native queues only if
  batch benchmarks improve. Preserve full bundle verification and staged
  replacement. Compare on Apple Silicon and an Intel Mac where supported, including
  low disk space and cancellation while waiting in a queue.

## Adoption and verification

1. Land the shared view with a FigCraft opt-in in the Launcher. Auto-select the same
   package and architecture as the current path. Exercise clean install, update,
   cancellation, a running app and failures. Test keyboard focus, screen-reader
   labels, window scaling and long text. Keep the existing view usable if the pilot
   needs to be disabled.
2. Qualify Linux on a Debian-family package, an RPM-family package, Flatpak and
   AppImage. Qualify macOS with a signed and notarized release DMG from Finder and
   through the Launcher. In both cases confirm that an interrupted operation leaves
   the previous working application and user data intact and that the correct
   version is detected after success.
3. Adopt across apps through the normal shared dependency roll and a consumer
   smoke test per supported route. Include an app with former package names or a
   custom bundle ID to catch accidental identity changes. Keep performance work
   separate and attach before/after measurements for each optimization.

Prototype evidence in Windows Sandbox includes a clean per-user install of FigCraft
0.18.1, repair of a missing executable and uninstall. Synthetic MSI cases covered
upgrade, cancellation and rollback, removal and busy files. Repair of the official
machine-wide install returned MSI 1603 at ProcessComponents in the prototype.
Treat machine-wide repair as unqualified. Linux and macOS runtime and benchmark
results are pending.
