# Install and uninstall across the Craft App Suite

Source audit: 2026-10-10; suite UI unified locally 2026-10-11. Covers the 13 application repositories and ArtCraft Launcher. The
`print-*` directories in this workspace are separate worktrees of PdfCraft, DesignCraft and
craft-libs, rather than additional products.

## Recommendation and implemented boundary

Keep the current native packaging stack. Windows already uses WiX v5 throughout, macOS uses
signed/notarized application bundles in DMGs, and Linux uses nfpm plus AppImage/Flatpak tooling.
ArtCraft Launcher already supplies a single Rust install/remove backend for the suite. Replacing
these with a different installer framework would create more work than the completion-screen fix.

Extract the repeated native UI source into [`craft-installer`](../crates/installer/README.md),
not the application-specific component trees or the entire Launcher backend. All 14 Windows
packages now reference the same UI, rendered with profiles retaining their existing options. Checked-in fragments let
each repository build independently, including before this unpublished local work is merged.
The generator's `--check` mode detects drift against the canonical crate templates and profile.

The Launcher UI now queues successful install/update/removal results for its 12 catalogued apps and shows an explicit
completion dialog until dismissed. Results wait behind an open settings, notes or confirmation
dialog, and survive the window being hidden in the tray. Batch results appear one at a time.
The existing translated backend message identifies the app and operation; the UI does not infer
success from a filesystem scan or display success for paused/cancelled/failed jobs.

## Windows inventory

All 14 repositories use WiX MSI plus a portable ZIP. The following is the source state before
this change; extension-based wizards supply their terminal dialogs through WiX itself.

| App | Revision inspected | Existing full UI | Action |
|---|---|---|---|
| CADCraft | `06f742c` | Custom `CadcraftUI`, WiX Common support | Adopt; preserve options |
| DeckCraft | `f448576` | None | Adopt `CraftInstallerUI` |
| DesignCraft | `a127c6d` | None | Adopt `CraftInstallerUI` |
| EffectCraft | `1b4d47f` | Original destination/maintenance/progress/outcome wizard | Adopt; preserve folder/upgrade policy |
| FigCraft | `5b321a7` | `WixUI_InstallDir`, versioned Finish, launch option | Adopt; preserve options |
| FilmCraft | `794a25f` | None | Adopt `CraftInstallerUI` |
| GridCraft | `5b15827` | Original desktop-shortcut UI, success/cancel/error dialogs | Adopt; preserve saved shortcut choice |
| LightCraft | `964d9338` | Custom WiX wizard, destination and launch option | Adopt; preserve options |
| PdfCraft | `a6bc63a` | Original welcome/maintenance/progress/outcome UI | Adopt; preserve desktop-shortcut choice |
| PhotoCraft | `0c72d954` | `WixUI_InstallDir`, launch option | Adopt; preserve options |
| SoundCraft | `330a698` | None | Adopt `CraftInstallerUI` |
| VectorCraft | `6f7f32c6` | `WixUI_InstallDir`, version text and launch option | Adopt; preserve options |
| WordCraft | `fc0c11a` | `WixUI_Minimal`, launch option | Adopt; preserve options |
| ArtCraft Launcher | `bd1182e` | Per-user `WixUI_InstallDir`, saved destination/shortcut, launch option | Adopt; preserve self-update contract |

The applications install per machine; the Launcher installs per user. Stable `UpgradeCode` and
component GUIDs, `AllowSameVersionUpgrades`, associations, signing and shortcut registration are
not shared policy. All 14 modified package definitions preserve those identities and physical installation scopes.
Native validation required public directory aliases for GridCraft/PdfCraft machine shortcuts;
PdfCraft uses HKMU shortcut key paths resolving to HKLM, with its explicit GUIDs unchanged.
Original/current GridCraft native MSIs confirm every component GUID matches.
Their generated UI uses native MSI controls and no external artwork. Existing app-owned launch
custom actions and WiX path validation are retained.

Settings uses an MSI-owned `Craft.<app>` registry entry with an explicit full UI uninstall
command and a separate quiet command. The native MSI entry is hidden rather than removed,
keeping managed product registration and Launcher detection. The visible display name appends
`(Craft Apps)` so existing Launchers cannot mistake the alias for a product code. Stock wizard
`ARPNOMODIFY` policies remain explicit in consumers. These shared registry components are the
only newly extracted installation ownership; executable/file associations stay app-owned.

The completion fragment distinguishes installation, repair and `REMOVE="ALL"`. Success, error
and cancellation have separate `OnExit` entries in both install and administrative UI sequences.
All authored dialogs require `UILevel = 5`, so unattended installs cannot hang on Finish.
This matches [WiX's terminal dialog sequencing](https://docs.firegiant.com/wix/schema/wxs/show/).

## Other platforms and removal ownership

| Platform/format | Install path | Removal path | Completion owner |
|---|---|---|---|
| macOS DMG | Copy signed `.app` to Applications; Launcher mounts, stages and swaps it | Finder/Launcher moves bundle to Trash | Finder for manual copies; persistent Launcher result for managed jobs |
| Windows MSI | `msiexec`; Launcher uses `/passive /norestart`, serializes its own MSI jobs | Windows Installer product registration, `/x` | Native Finish for full UI and Settings removal; persistent Launcher result for passive jobs |
| Linux DEB/RPM | nfpm packages, distro installer under polkit through Launcher | Same distro package manager | Distro UI or persistent Launcher result |
| Linux Flatpak | App-owned manifests and bundle scripts, Flatpak | Flatpak remove | Flatpak UI or persistent Launcher result |
| Linux AppImage | Portable file, menu integration in app/Launcher | Remove app file and matching desktop entry | Persistent Launcher result for managed jobs; manual deletion otherwise |
| Linux tarball | Extract FHS tree; FilmCraft additionally includes `install.sh` and terminal completion text | Manual installation has no common ownership manifest/uninstaller | Terminal/manual flow; no new graphical wizard |
| FreeBSD tarball | Existing package scripts; Launcher records installed files and prefix | Launcher removes recorded files, using polkit when required | Persistent Launcher result for managed jobs; manual removal otherwise |
| Windows portable ZIP | Extract and launch | Delete extracted directory | Manual operation |
| Web | Deploy static WASM output | Remove hosting files/browser shortcut | Hosting/browser operation |

The common contracts are package ownership, result handling and completion presentation, rather
than one set of cross-platform file operations. Documents and settings stay outside installer
ownership. Package-manager hooks refresh integration caches on install and remove; adding GUI
dialogs in those hooks would interfere with unattended package operations.

FigCraft is the thirteenth app audited here, but is not in the Launcher's current `SUITE` catalog.
Its native MSI now uses the shared completion screen; adding it to the Launcher catalog is outside
this installer UI change.

## Existing Rust backend and extraction cost

`craft-launcher/crates/native/src/install.rs` already centralizes download/checksum verification,
format selection, retries, queueing, installing, updating and removal. Its callers are
`backend.rs` and `self_update.rs`; state is shared with app catalog/status/protocol types,
HTTP cancellation, installed-file manifests, platform detection and configured paths. The pure
`core` and egui UI also compile to WASM, while native filesystem/process code stays target gated.

Moving that backend to craft-libs today would move or adapt those dependencies and self-update
contracts, with only one concrete backend consumer. It would not reduce duplicate installers in
the 13 app repositories, which contain package-building scripts rather than copies of this engine.
Keep it in the Launcher. The first shared crate therefore owns a complete native UI template and the Settings registration
and its generator/checker; it has no runtime dependencies or dependencies on application code.

The generator now has explicit profiles for the richer wizards. Six profiles retain folder
selection, four retain optional desktop shortcuts, six retain launch checkboxes and WordCraft
retains license acceptance. The Launcher's per-user folder/shortcut registry remains readable by
self-update. GridCraft's saved shortcut policy stays app-owned. The shared checkbox publishes
explicit 1/0 values before costing so unchecking survives elevation; folder selection follows
CostFinalize. EffectCraft's upgrade folder remains locked. Launch actions retain their original
privilege context and are gated off during removal, repair and administrative image creation.

## Rust ecosystem comparison

[`cargo-packager`](https://docs.rs/cargo-packager/latest/cargo_packager/) 0.11.8 is a concrete
Rust CLI/library option: its documentation lists macOS app/DMG, Linux DEB/AppImage/Pacman and
Windows NSIS/WiX MSI. It accepts custom packaging configuration and can be used as a library.
Its documented format list does not include this suite's RPM, Flatpak or FreeBSD distribution
flows. Adopting it would still require adapters and revalidation of existing MSI component
identities, associations, signing/notarization and the Launcher's asset/self-update contracts.
That migration is unnecessary for the requested completion fix. No new production library is added.

## Validation and practical limits

Completed locally: craft-libs workspace formatting/clippy and all 86 tests; Launcher full
`xtask ci` from the completion-dialog change (formatting, workspace clippy, passing workspace tests,
core/web WASM checks). Two pre-existing Launcher tests remain ignored: the storyteller-web
sample contract and a real-network release download. The new crate passes a WASM library check.
Launcher completion screenshots were rendered and inspected using the existing egui test harness.

- Shared crate tests cover all 14 profiles, the three terminal outcomes in both sequences,
  removal/repair text, folder and shortcut sequencing, launch guards and CLI drift/error handling.
- All 14 consumer snapshots pass the generator's drift check. Old/new package, feature, component,
  file, shortcut, registry, search, destination and upgrade attributes were compared for unchanged
  installation ownership and identities. UI state/control definitions are replaced; folder
  properties retain secure propagation after removal of the old WiX UI extension wizard. Native
  shortcut validation fixes change GridCraft/PdfCraft directory references and PdfCraft key-path
  root authoring while preserving physical scope and component identities.
- All 42 generated UI/architecture combinations compile to WiX 5.0.2 libraries for x64/x86/ARM64.
  WiX warns that Linux is unsupported, so these checks cover source compilation, not MSI linking
  or runtime behavior. All 41 packaging/test PowerShell scripts parse with PowerShell 7.4.6.
- Launcher tests cover persistent install/removal completion after the old toast expiry,
  ordered batch dismissal, failed/cancelled jobs, hidden-window state and existing-dialog behavior.
- The new `crates/installer/tests/windows-suite.ps1` builds all 14 actual package definitions with
  fixture files on Windows for all three architectures, validates MSIs and inspects their UI tables.
  It does not install anything. Existing EffectCraft/GridCraft tests compile the new UI, and
  PdfCraft's table checks now verify explicit checked/unchecked shortcut events.
- Native Windows linking/UI tables passed for all 42 fixture packages; all 14 x86 silent
  install/cached-remove checks passed. Validation and functional GUI testing are recorded in the
  [Windows verification record](windows-installer-verification.md). The local evaluation VM
  exposed and verified fixes for maintenance sequencing and client completion state. The dedicated
  Windows 11 clone additionally verifies actual Settings removal for both scopes, including
  machine UAC consent, and all-profile x64/x86 interactive/quiet registered removal. Fixture
  tests do not verify real app payloads, accessibility, Restart Manager or reboot acceptance.

Two existing backend details limit how much a generic result screen can claim: Windows treats
MSI return codes `3010`/`1641` as success without forwarding restart information, and FreeBSD's
`remove_files` currently ignores individual file-removal failures. Neither is changed by this
UI extraction. A future backend outcome type should carry reboot requirements, and recorded-file
removal should propagate permission/I/O errors before reporting success. Manual Linux/FreeBSD
tarball installation also needs an ownership manifest before a safe common uninstaller is viable.

No repositories were pushed, no packages/sites/releases were published, and no PRs were created.
