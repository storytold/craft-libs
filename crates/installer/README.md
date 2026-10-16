# craft-installer

The suite's common Windows MSI interface, rendered by a dependency-free Rust library and CLI.
All 13 apps and ArtCraft Launcher use the same native WiX v5 dialogs, typography and completion
screens. Each profile preserves the options its existing installer offered:

| Option | Apps |
|---|---|
| Destination folder | EffectCraft, FigCraft, LightCraft, PhotoCraft, VectorCraft, Launcher |
| Desktop shortcut checkbox | CADCraft, EffectCraft, GridCraft, PdfCraft, VectorCraft, Launcher |
| Launch on Finish checkbox | FigCraft, LightCraft, PhotoCraft, VectorCraft, WordCraft, Launcher |
| License acceptance | WordCraft |
| Per-user installation | Launcher; the 13 apps remain per-machine |

Defaults, previous-folder searches, shortcut persistence, launch actions, existing component GUIDs,
upgrade codes, file associations and signing stay in the app's own `.wxs`. EffectCraft updates
keep the destination locked. App-owned launch actions retain their existing execution context,
including LightCraft's unelevated shell execution. No new installer framework is introduced.

## Generate and check

From a craft-libs checkout (Rust 1.95+):

```sh
cargo run --locked -p craft-installer -- --write --app filmcraft ../filmcraft/packaging/windows/installer-ui.wxs
cargo run --locked -p craft-installer -- --check --app filmcraft ../filmcraft/packaging/windows/installer-ui.wxs
```

Use the repository name for `--app`, including `craft-launcher`. `WindowsApp::ALL` lists the
14 profiles. Rust callers use `windows_ui`, `write_windows_ui` and `check_windows_ui` with that
profile. Unknown profiles fail before writing; drift checks never modify the consumer's file.

Consumers reference `<UIRef Id="CraftInstallerUI" />` and pass the generated `installer-ui.wxs`
to `wix build` alongside their package source. Their main feature must also reference
`<ComponentGroupRef Id="CraftInstallerSettings" />`, which installs the Settings registration. Use this as the complete UI, without another
`WixUI`/`WixUI_Common` wizard. Folder profiles that already used WiX's path validator keep the
UI extension; existing launch actions keep the Util extension. Simple profiles need neither.
No bitmap or font file is bundled: Windows renders its own controls and installed UI font. Settings calls the signed system
`msiexec.exe` directly; no additional executable, script wrapper or runtime library is installed.

Check in the snapshots so each app builds independently, without a sibling checkout or new
runtime dependency. The crate sets `publish = false`; publishing is unnecessary. When changing
a template or profile, regenerate and `--check` every consumer. Templates live here, not in apps.

## Flow and completion

The welcome screen collects optional shortcut choices **before costing**. A private checkbox
publishes explicit public 1/0 values, preserving opt-out through elevation and avoiding a saved
preference being reloaded after the user unchecks it. Folder selection follows `CostFinalize`,
uses native browsing and `SetTargetPath`, and retains the existing WiX validator where applicable.
Maintenance follows `CostFinalize` before offering Repair/Remove: those feature events require
the selection manager. A private UI operation value keeps their completion messages correct even
though MSI only forwards the feature event’s `REINSTALL`/`REMOVE` flags to the execution session.
Progress, MSI errors and files-in-use dialogs use the same UI.

Install, uninstall and repair each have an explicit success message and Finish button. Failure
and cancellation have separate ending screens. Launch is offered only where it existed before,
and only for successful installation/update, never removal, repair, administrative images or
cancelled/failed setup. Every authored sequence entry requires `UILevel = 5`; `/qn`, `/qb` and
`/passive` stay unattended. Settings/Control Panel get a separate entry at
`Uninstall\Craft.<repository>` in HKLM for the apps and HKCU for Launcher. Its `UninstallString`
selects `/x {ProductCode} /qf /norestart`, and `QuietUninstallString` selects `/qn /norestart`.
The native MSI entry is hidden with `ARPSYSTEMCOMPONENT=1` while MSI registration remains intact.
Its displayed name appends `(Craft Apps)`, preventing older Launchers from interpreting its
non-GUID registry key as an MSI product code. The hidden native entry retains the canonical
name and publisher for Launcher discovery. The visible entry omits `WindowsInstaller`, so Settings honors its command rather than invoking
basic MSI removal. `ModifyPath` opens full maintenance; the existing `ARPNOMODIFY` policy is
retained, including the implicit policy from the replaced stock WiX wizards. The visible entry is removed by MSI components, including during quiet removal.
Plain command-line `/x` still uses Windows' default UI; use `/qf` when calling it interactively. Launcher-managed jobs have their own persistent completion dialog.

## Validation

`cargo test -p craft-installer` checks all 14 capability profiles, shortcut propagation, folder
and maintenance sequencing, upgrade restrictions, launch guards, all three endings in both UI
sequences, dialog references, keyboard dismissal and CLI drift/error behavior.

The [Windows verification record](../../docs/windows-installer-verification.md) records native
linking, table checks, actual install/remove results and remaining limits. GridCraft/PdfCraft's
shortcut directory aliases and key paths were corrected during native validation; physical
locations, machine scope and component GUIDs remain stable. No package or release was published.

On Windows, compile and check actual package definitions without installing anything:

```powershell
powershell -NoProfile -File crates/installer/tests/windows-suite.ps1 -Repositories C:\work\CraftApps -Work C:\CraftInstallerTest\compiled
```

This builds all 14 sources with fixture files for x64, x86 and ARM64. By default it runs full ICE
validation on every architecture, then inspects UI tables. `-StartAt` resumes at an app;
`-ValidationArchitectures x64` still builds/checks all three architectures but repeats ICE only
for x64, useful after the package ownership rules have already passed a complete matrix.
Requires WiX 5.0.2 and its UI/Util extensions. Windows PowerShell 5.1 and PowerShell 7 are supported.

Run the destructive runtime checks **only on a clean disposable Windows desktop** with an
administrator account. The fixtures share the real products' UpgradeCodes: running them on a
normal development machine could replace real apps. Each runner requires `-DisposableMachine`,
checks fixture version 90.0.0 and rejects existing related products before starting. The UI
runner accepts `-StartAt <repository-name>` after a failed fixture has been cleaned up; it still
checks every profile for existing installations before resuming.

```powershell
powershell -NoProfile -File crates/installer/tests/windows-ui.ps1 -DisposableMachine -Packages C:\CraftInstallerTest\compiled
powershell -NoProfile -File crates/installer/tests/windows-options.ps1 -DisposableMachine -Packages C:\CraftInstallerTest\compiled -Repositories C:\work\CraftApps
powershell -NoProfile -File crates/installer/tests/windows-settings.ps1 -DisposableMachine -Packages C:\CraftInstallerTest\compiled
powershell -NoProfile -File crates/installer/tests/windows-x86.ps1 -DisposableMachine -Packages C:\CraftInstallerTest\compiled
```

The UI runner explicitly selects `/qf` and checks every x64 app's full install, missing-file repair, cached-product uninstall,
correct completion text and explicit Finish, destination paths containing spaces/non-ASCII,
Start menu cleanup, desktop opt-out and a retained user-document sentinel. It also exercises
cancellation, maintenance removal and `/qn`, `/qb`, `/passive`. Screenshots, control inventories
and verbose MSI logs remain under the fixture output directory.

The Settings runner checks all 14 entries, interactive and quiet commands, hidden native MSI
registration, Modify policy and removal of both entries. Exercise the actual Settings button
workflow separately on the visible desktop. The options runner checks folder retention through upgrades, shortcut opt-in/repair/upgrade
opt-out, GridCraft removal as SYSTEM, downgrade failure and all six launch checkboxes using an
original GUI fixture that records launches/account identity. The x86 runner checks every app's
silent install, 32-bit App Paths registration and cached removal. These are packaging/UI checks,
not tests of real application binaries, signing, screen readers, UAC consent or ARM64 execution.
Existing EffectCraft/GridCraft install tests also consume the shared UI; PdfCraft's table test
checks scope, shortcut ownership and native controls.

For release acceptance, additionally use real packages on Windows 10/11 and native ARM64;
check keyboard/DPI/accessibility, UAC and launch privileges, files in use/Restart Manager,
reboot-required success, command-line option overrides and actual document/settings locations.

The shared UI adapts the existing PdfCraft, GridCraft and EffectCraft sources under MIT OR
Apache-2.0; see NOTICE. macOS/Linux packaging and the Launcher backend remain platform-owned.
See the [suite audit](../../docs/install-uninstall-audit.md) for the extraction rationale.
