# Windows installer verification — 2026-10-11

All work is local. No packages were published and no repositories were pushed or made into PRs.

## Initial Windows Server environment and payloads

A local QEMU Windows Server 2022 Datacenter evaluation VM with Desktop Experience was created
for this task. No existing usable Windows VM configuration was found. The guest is x64,
with two virtual CPUs and 4 GiB RAM; KVM is unavailable to the host account, so it uses software
emulation. WiX 5.0.2 uses .NET 8.0.31; tests run on Windows PowerShell 5.1.

Packages use actual app `.wxs` sources, shared UI snapshots, original icons and a synthetic
license. System notepad files supply fixture PE bytes. Launch checks use an original managed
GUI fixture which records the executable path and account SID and exits immediately. These
are installer tests, not builds or runtime tests of the Rust applications. Folder automation
activates the desktop dialog, recognizes MSI's `RichEdit20W` PathEdit and changes keyboard focus
to commit edits before Install. FigCraft's Browse/Cancel/typed-folder case was also reproduced
with direct VM keyboard input and confirmed in App Paths. The runner retains the child process
handle so PowerShell 5.1 reports exit codes reliably; phase wrappers also require an explicit
successful completion marker and reject failure output.

## Defects found and verification corrections

- Maintenance originally preceded costing. Repair/Remove feature events require the selection
  manager initialized by `CostFinalize`; clicking Repair produced repeated MSI error 2731.
  Welcome/shortcut choices remain before costing; maintenance now follows `CostFinalize`, with
  destination selection after maintenance. Native MSI table tests enforce that ordering.
- Repair's feature event forwards `REINSTALL` to the execution session without setting it in
  the client UI. The successful repair therefore displayed the install message. A private UI
  operation value now selects the repair/removal confirmation for maintenance buttons; direct
  `/x` and command-line repair still use their MSI properties. Launch is suppressed for both.
  The corrected CADCraft install/repair/maintenance-remove probe passed all three Finish screens.
- GridCraft/PdfCraft machine shortcut components used standard directory identifiers that ICE
  classifies as user directories, causing ICE38/43/57 validation errors. Public directory aliases
  resolve to the same all-users Start menu/desktop locations. PdfCraft's shortcut key paths use
  HKMU (HKLM under its enforced `ALLUSERS=1`); GridCraft retains its existing HKMU key path.
  Plain shortcut targets resolve through `INSTALLFOLDER`, avoiding cross-component file references.
  PdfCraft's explicit component GUIDs are retained. Native compilation of original/current
  GridCraft x64 sources confirmed all four component GUIDs match.
- Windows Installer COM `Execute`/`Close` return values polluted the table helper's result on
  PowerShell 5.1. They are explicitly discarded. PdfCraft's Unicode label assertion now uses a
  character code, avoiding UTF-8-without-BOM misreading on PowerShell 5.1; its native table test passes.

See Microsoft's [error 2731 definition](https://learn.microsoft.com/en-us/windows/win32/msi/windows-installer-error-messages)
and WiX's [costing requirements](https://docs.firegiant.com/wix/schema/wxs/costfinalize/).

## Native results

The first complete matrix linked and ran full ICE validation and UI table checks for all
42 app/architecture combinations after the shortcut fixes. The final UI revision also passed
actual linking and UI table checks for all 42 combinations, with full ICE validation repeated
on all 14 x64 packages. Other architectures received actual linking and final UI table checks.
The default test script validates all architectures; this emulated rerun explicitly uses `-ValidationArchitectures x64`.

The final native matrix passed for CADCraft, DeckCraft, DesignCraft, EffectCraft, FigCraft,
FilmCraft, GridCraft, LightCraft, PdfCraft, PhotoCraft, SoundCraft, VectorCraft, WordCraft and
ArtCraft Launcher on x64, x86 and ARM64. All 14 x86 packages passed silent installation,
32-bit App Paths/file checks and cached-product removal on the x64 guest. PdfCraft's own
native MSI table test also passed against the final package.

All 14 x64 profiles passed full UI (`/qf`) installation, missing-file repair and cached-product
removal. Each operation displayed the correct persistent completion text, hid launch during
repair/removal, waited for Finish and returned 0. The six folder profiles accepted a path with
spaces/non-ASCII; all Start menu shortcuts were present then removed, all four desktop profiles
honored opt-out through repair, and the external user-document sentinel survived removal.

FigCraft also passed cancellation (Finish returned 1602), maintenance Remove and unattended
`/qn`, `/qb`, `/passive` installation/removal. The UI matrix resumed after automation corrections;
CADCraft/DeckCraft/DesignCraft and EffectCraft passing prefixes were retained, and the remaining
profiles plus additional cases completed in the final phase. No package UI changed during those
runner corrections.

All six destination profiles retained their custom directories through a 90.0.0 → 90.0.1
upgrade, including EffectCraft's locked upgrade directory. All four desktop profiles passed
opt-in, deleted-shortcut repair, upgrade opt-out and removal. GridCraft also passed removal as
SYSTEM through a scheduled task: the Public desktop link and machine marker were removed,
and no HKCU marker was created. FigCraft's blocked downgrade displayed the failure ending,
returned 1603 and left the newer installation/path intact.

All six launch profiles passed a checked Finish launch exactly once under the desktop account,
unchecked Finish without a launch, repair without a launch and removal without a launch.
LightCraft exercised its retained `WixUnelevatedShellExec` action; the other five exercised their
retained `WixShellExec` actions. The original managed fixture records the path/account SID; the
built-in Administrator limitation below still applies.

| Check | Result |
|---|---|
| Final native linking/UI tables | 42/42 packages: 14 profiles × x64/x86/ARM64 |
| Full ICE validation | First complete 42-package matrix; final revision repeated on 14/14 x64 packages |
| Full UI install / repair / cached uninstall | 14/14 each; 42 distinct successful operations |
| Custom folder / upgrade retention | 6/6 profiles |
| Desktop default / opt-out / opt-in / repair / upgrade opt-out | 4/4 profiles |
| Checked / unchecked launch and no launch on repair/removal | 6/6 profiles |
| Cached x86 silent install/removal | 14/14 on x64 Windows |
| Cancellation / maintenance Remove / downgrade failure | FigCraft: 1602 / 0 / 1603 with correct ending screens |
| Unattended `/qn`, `/qb`, `/passive` | FigCraft install and removal; all completed without Finish interaction |
| SYSTEM removal | GridCraft shortcut and registry marker cleanup passed |

The options phase completed 66 additional full UI endings plus the SYSTEM cleanup assertion.
The final state check confirms no related fixture products or test scheduled tasks remain.
Verbose logs, control inventories and screenshots are retained as private VM artifacts.
Normal WiX CLI commands are provided in the crate. To reduce repeated .NET startup costs under
emulation, the final linking/options runs invoke the same pinned WiX CLI entry point in a resident
private process; this helper is not shipped in any app or crate.

Existing nonfatal warnings are retained: ICE61 for the intentional same-version upgrade policy;
Launcher also emits ICE69 for its existing cross-component shortcut file references and ICE91
for its explicitly per-user installation directory. Validation errors are not suppressed.

## Reproduce and limits

The [crate README](../crates/installer/README.md#validation) has commands for linking/table tests,
full UI checks, options/upgrades/launches and x86 silent installation/removal. Runtime scripts
require an explicitly disposable desktop, fixture version 90.0.0 and no existing related
products. They preserve verbose logs, control inventories and completion screenshots.

The runtime runner selects `/qf` explicitly. On this guest, plain `msiexec /x` uses
`UILevel=3`, confirms removal and bypasses authored dialogs. Use full UI or maintenance Remove
for the completion screen. This initial revision did not override Settings removal; the
Windows 11 follow-up below adds the explicit full UI command entry.

The evaluation guest runs as built-in Administrator. UAC consent and a genuinely unelevated
launch token are not verified; launch tests can confirm the account and launch count only.
Native ARM64 execution, real app payloads, release signing, Windows 10, screen readers,
non-default DPI, locked files/Restart Manager, reboot-required outcomes and all command-line
option overrides remain release acceptance checks. The VM's credentials and OS image are
private build artifacts outside the repositories.

## Windows 11 Settings follow-up — 2026-10-12

The follow-up uses a dedicated validated clone of the provisioned clean-tools baseline:
`/home/codex/work/VMs/craft-windows/clones/win-installer-settings-20261011-01`.
The primary VM and baseline were preserved. The clone runs Windows 11 Pro build 26200,
KVM, four vCPUs and 16 GiB RAM, with the `craftagent` desktop and normal UAC enabled.
The environment verifier performed a native Rust MSVC compile/link/run. Actual task sources
were separately exported to `C:\CraftApps\worktrees\installer-settings-20261011-01`, with
repository HEADs, tracked patches and SHA-256 for every exported source file. Native Cargo
outputs use separate targets under `C:\CraftApps\targets\installer-settings`.

The Settings entry is owned by MSI components, with full UI `UninstallString`, unattended
`QuietUninstallString`, and the existing Modify policy. Native MSI registration remains
hidden and available for repair/upgrades/Launcher discovery. The visible label appends
`(Craft Apps)` to avoid matching existing Launchers' canonical-name detector. The updated
Launcher additionally requires a GUID key and `WindowsInstaller=1` before passing a key to MSI.
The stock wizards' implicit `ARPNOMODIFY=1` is retained explicitly for FigCraft, PhotoCraft,
VectorCraft, WordCraft and Launcher, alongside CADCraft and LightCraft's explicit policies.

Completed native checks for the final package sources:

- All 42 packages linked and passed full ICE validation and MSI table assertions: all 14
  profiles on x64, x86 and ARM64. Validation errors were not suppressed; the same pre-existing
  ICE61 and Launcher ICE69/91 warnings remain.
- The actual shared Rust crate compiled and passed all seven tests and clippy on Windows.
  Launcher native backend compiled and passed 62 tests, with one existing network test ignored,
  and clippy with warnings denied. Linux craft-libs passed 86 tests and workspace formatting/clippy;
  Launcher full `xtask ci` passed formatting, clippy, tests and both WASM checks.
- Actual Windows Settings menu and confirmation buttons were clicked for the per-user Launcher
  and per-machine CADCraft. Both dispatched the registered `/x {ProductCode} /qf /norestart`
  command on the active desktop. CADCraft required and received Windows Installer UAC consent;
  Launcher did not. Both displayed the correct persistent uninstall completion, hid launch,
  waited for Finish, exited 0, and removed their product/visible Settings registration.
- All 14 profiles on both x64 and x86 passed registered-command full UI removal: 28 correct
  uninstall endings, each persistent until Finish, returning 0 with launch hidden. Their
  visible entries retained the expected scope, display metadata and Modify policy; hidden
  native entries retained canonical names/publisher/WindowsInstaller identity. All product
  and visible/native entry cleanup assertions passed.
- Unattended `/qn` installation passed 56 times (two per profile/architecture), and the
  registered quiet uninstall passed 28 times. All returned 0 without desktop input or Finish.
- The options runner passed all 66 full UI endings: six custom-folder upgrades, four desktop
  opt-in/repair/upgrade-opt-out profiles, six checked/unchecked launch profiles, no launch on
  repair/removal, and FigCraft's blocked downgrade (expected failure ending, exit 1603).
  GridCraft removal as SYSTEM cleaned up its machine shortcut/marker. Upgrade checks verify
  the Settings version/ProductCode refresh; deleted Settings keys were restored during repair
  for all four desktop profiles. Launch fixtures confirm the account SID and count, not token
  integrity level.
- An initial test assumed a per-user native MSI ARP entry would always live in HKCU. On this
  elevated test desktop, Windows registered Launcher's hidden native entry in HKLM while its
  visible entry remained HKCU and its files remained in the user's LocalAppData. The test now
  searches both native hives as Launcher does, while still enforcing HKCU for the visible entry.
  This correction did not change any package or production source.

The fixture payload limitation from the initial run still applies: package tests use actual
WiX definitions/icons and synthetic PE/license/launch fixtures, not all 13 Rust app binaries.
Native ARM64 execution, release signing, Windows 10, accessibility/DPI, locked-file/reboot cases
and additional command-line overrides remain release acceptance checks.

Astra reviewed all source changes and untracked installer assets across the 15 worktrees.
It found the old-Launcher alias compatibility problem and the five lost implicit Modify
policies described above. Both were fixed and re-reviewed. Astra's final source review has
no remaining actionable findings; it also reviewed the repair/upgrade and native-hive tests.

Evidence is retained in the clone's `state/` and guest `C:\CraftApps\evidence\installer-settings`:
MSIs, source identity/patches, full matrix/native Cargo logs, Settings dispatch process records,
control inventories, desktop screenshots and runtime logs. `source-qualification.txt` verifies
that the final runtime source export has byte-identical compiled WiX/icon/generator/template
sources to the successful matrix. No packages were published, no repositories pushed and no PRs opened.

The final cleanup check found no related fixture products, visible Settings aliases or test
scheduled tasks. Settings was closed normally, evidence was copied to the host and ZIP integrity
verified. The task clone was gracefully stopped and `running=false` confirmed; the clone and
evidence are retained. The shared primary VM and clean-tools baseline were not changed.


## Fresh Windows VM and native Launcher payload — 2026-10-12

This run follows `/home/codex/.codex/AGENTS.md` in a separate, checksum-verified full-copy
clone: `/home/codex/work/VMs/craft-windows/clones/win-installers-verify-20261012-02`
(SSH 2225, VNC 5905). The Windows 11 Pro 26200 baseline and primary VM remain untouched.
The environment verifier passed a real native Rust/MSVC compile/link/run. Task sources,
native targets and evidence are separated under the `installer-verify-20261012-02` name.
The source manifests record every exported source file against the host worktrees, including
unpublished patches and all 14 generated UI snapshots; the final export contains 315 files.

The first fresh fixture run passed all 42 MSI links/full ICE/table checks, 44 full UI endings,
28 registered interactive removals across x64/x86, all registered quiet removals, and the
66 options endings plus GridCraft SYSTEM cleanup. A Launcher shortcut-property sharing
warning (MSI 1946, HRESULT 32) required acknowledgment during full UI repair. It exposed two
shared UI defects: missing `ErrorIcon` (diagnostic 2835) and attempting to initially focus a
hidden Launch checkbox during repair (2836). The shared template now authors ErrorIcon with
a local Binary sourced from the package's existing `IconPath`, and puts Finish first among
focusable completion controls. The icon fallback adds no extension requirement. MSI retains
its native warning message and response buttons; warnings are not suppressed.

Astra caught that the initial icon solution depended on an extension that some real packaging
commands do not load. The native matrix now matches every release command's extension set,
and asserts the icon Binary row and completion `Control_First=Finish`. All 14 snapshots were
regenerated natively and passed drift checks. The shared crate passed its seven Windows tests
and clippy with warnings denied. Astra's final source review found no remaining actionable issues.

The actual Launcher workspace passed 168 native tests, with two pre-existing tests ignored,
and produced a freshly compiled Windows x64 GUI executable with static CRT. An initial snapshot
harness failure was corrected by creating its output directory, then the complete test suite
was rerun successfully; both attempts are retained. Build outputs used a fresh task-specific
target, not the baseline's application cache. The existing `package.ps1 -SkipBuild` recipe
packaged that exact executable, passed PE/icon checks and full native MSI validation. It is
an unsigned opt-level=0/debug=0 acceptance build, not an optimized release-performance or
signing qualification. The executable SHA-256 is
`46D3081AB5C50F25EF1F631C47221F2B27B5D9AA5B755864EB737661A36B3B89`.

Host evidence lives in `/home/codex/work/evidence/installer-verify-20261012-02`; the clone also
retains screenshots in `state/` and full guest records in
`C:\CraftApps\evidence\installer-verify-20261012-02`. Logs retain existing ICE61 and Launcher
ICE69/91 warnings. The Binary identifier also produces WiX's merge-module length warning;
these packages are standalone MSIs. No validation errors or runtime warnings were suppressed.


Final checks and targeted follow-up:

- All 42 variants passed full linking/ICE/UI table verification with the actual release
  extension sets. The corrected UI passed all 44 install/repair/removal/cancellation endings,
  all 28 x64/x86 registered interactive removals, 56 unattended fixture installations and 28
  registered quiet removals. FigCraft also passed `/qn`, `/qb`, `/passive` install/removal.
- The options rerun completed all 66 endings and SYSTEM cleanup. A scan of its verbose logs
  found one additional 2836: EffectCraft's upgrade locks its destination controls, but initially
  focused the disabled Folder field. Its Install control now comes first, preserving the
  lock and all installation events. The other 13 generated snapshots are byte-identical to
  the accepted 42-package matrix. EffectCraft alone was recompiled and passed full ICE/table
  checks on x64/x86/ARM64, superseding its three earlier artifacts.
- The final EffectCraft desktop probe passed editable custom-folder first install, missing-file
  repair, locked-folder 90.0.0 → 90.0.1 upgrade, refreshed Settings metadata and removal, with
  no 2835/2836 diagnostics. The harness initially assumed PathEdit already had focus; direct
  UI Automation focus was also unsupported by this native control. It now tabs to the field
  and checks its actual HWND via `GetGUIThreadInfo` before setting text and committing through
  focus loss. Both failed probes and the successful repeat are preserved. The pending probe
  was cancelled normally; the first installed fixture was removed through its owned MSI.
- The freshly built Launcher MSI installed into a chosen path containing spaces/non-ASCII,
  honored desktop opt-out and launched the exact qualified executable from Finish on the
  active Explorer desktop (session 2). Its real window was inspected and quit normally through
  Settings. Missing-file repair restored the identical SHA-256 without launching. Its Start
  menu shortcut retained the correct target and `ai.storyteller.artcraft-launcher` notification ID.
- Actual Windows Settings menu and confirmation buttons removed that Launcher package via
  the registered system `msiexec /x {ProductCode} /qf /norestart`. Completion waited for Finish,
  hid launch and exited 0. The executable and visible entry were removed; the external test
  document and the real saved `app.ron` settings remained byte-identical.
- The actual Launcher package also passed all six `/qn`, `/qb`, `/passive` install/removal
  operations without desktop input, Finish or app launch. Each quiet installation’s payload hash and final saved-settings
  retention were checked.

Astra re-reviewed the icon dependency fix, EffectCraft's focus/order change, regenerated
snapshots and the final keyboard-focus harness. No actionable source findings remain.
The final acceptance audit treats only the explicitly superseded EffectCraft upgrade log as
obsolete; all current runtime logs must be free of 2835/2836. The original 1946 sharing warning
is preserved in the first run's evidence and was not suppressed. The final run and actual
Launcher repair did not reproduce it.

Only Launcher received a fresh native Rust payload build in this run; the other app payloads
remain synthetic installer fixtures. Native ARM64 execution, Windows 10, release signing,
optimized release performance, non-default DPI/accessibility and general locked-file/reboot
scenarios are outside this run's qualification. No packages were published, no repositories
pushed, and no PRs opened.


The final audit passed: 42 matrix entries, 44 shared UI endings, 14 registered x64 and
14 registered x86 uninstall endings, 66 options endings, and four corrected EffectCraft
endings. Current accepted logs contain zero 2835/2836 diagnostics and zero 1946 warnings.
No related fixture/actual Launcher products or visible Settings aliases remain, and the
actual app is closed. Native targets, source, unsigned packages and all evidence are retained.


## Remaining 13 real app payloads — 2026-10-12

All 13 remaining x64 Windows installers now pass with freshly compiled native Rust GUI and
CLI binaries. This extends the earlier real Launcher qualification to the entire 14-app suite.
The existing WiX implementation and shared `craft-installer` UI remain in use. Nothing was
published, pushed or made into a PR.

This run uses the independent baseline-verified overlay clone
`/home/codex/work/VMs/craft-windows/clones/win-installer-real-apps-20261012-03`, with separate disk,
UEFI, TPM, desktop, source and native targets. It runs Windows 11 Pro build 26200, four vCPUs,
16 GiB RAM, Rust MSVC and PowerShell 5.1. The native environment verifier compiled, linked and
ran before app builds. The primary VM and read-only baseline were preserved.

The initial source export contains 9,497 files from the 13 app worktrees and craft-libs.
`source-identity.json` records repository HEADs and every file's SHA-256; the transferred ZIP
and guest files were checked. Two subsequent code/test deltas are recorded separately below.
Sources and outputs live under the task-specific Windows worktree and target directories,
without reusing baseline app binaries or Cargo targets. Each native build records its full
arguments, stdout/stderr, exit codes, unfiltered test counts, profile and GUI/CLI hashes.

| App | Version | Engine tests passed | Ignored | Installer workflows |
|---|---|---:|---:|---|
| CADCraft | 0.4.0 | 188 | 0 | Passed |
| DeckCraft | 0.4.0 | 42 | 0 | Passed |
| DesignCraft | 0.5.0 | 276 | 0 | Passed |
| EffectCraft | 0.7.0 | 485 | 2 | Passed |
| FigCraft | 0.3.1 | 444 | 2 | Passed |
| FilmCraft | 0.5.0 | 512 | 3 | Passed |
| GridCraft | 0.4.0 | 97 | 0 | Passed |
| LightCraft | 0.5.0 | 636 | 5 | Passed |
| PdfCraft | 0.4.0 | 96 | 1 | Passed |
| PhotoCraft | 0.6.0 | 1,074 | 11 | Passed |
| SoundCraft | 0.4.0 | 108 | 0 | Passed |
| VectorCraft | 0.8.0 | 1,646 | 0 | Passed |
| WordCraft | 0.4.0 | 164 | 0 | Passed |
| **Total** | | **5,768** | **24** | **13/13** |

For every app, the actual packaging script linked its MSI and passed full ICE validation.
GUI/CLI PE architecture/subsystem checks and packaged, installed and repaired binary hashes
match the qualified native payloads. LightCraft and PhotoCraft include their required HEIF
features. Existing ICE61 same-version upgrade warnings and the standalone-product error-icon
identifier warning remain visible; validation errors were not suppressed. Packages are unsigned.

The 39 successful full UI operations cover installation, missing-executable repair and actual
Windows Settings removal for each app. Each completion displays the appropriate install,
repair or uninstall text, remains open until Finish and exits 0. Repair and removal hide launch.
Actual app windows were inspected on Explorer's active console, and installed CLI `--version`
commands returned nonempty output with exit 0. Five destination profiles accepted folders with
spaces and non-ASCII characters; CADCraft, GridCraft and PdfCraft retained desktop opt-out through
repair. The five launch profiles launched their installed executable from Finish; the others
launched through their installed Start menu shortcut. WordCraft retains its license acceptance.

All 13 Settings removals used the real Installed apps row, menu and confirmation. Recorded
process command lines match the registered `/x {ProductCode} /qf /norestart` command on the
console session. Windows Installer UAC prompts were inspected before consent. Every app also
passed `/qn`, `/qb` and `/passive` installation and removal: 78 operations with no desktop input,
Finish interaction or app launch. Actual saved configuration files remained byte-identical
through repair, Settings removal and every unattended cycle. FilmCraft and LightCraft preferences
were created through their normal app commands; no synthetic configuration fixture substitutes
for saved settings. The external document sentinel is an additional retention check.

The final audit passed on 2026-10-12 at 14:24:19 UTC. It ties each MSI's SHA-256/ProductCode to
its native build attempt, source HEAD, exact payload hashes and UI/unattended evidence; checks
nonempty saved settings; and confirms no app process, related installed product, visible Settings
alias or installed executable remains. Earlier phase records without an MSI hash additionally
must follow package validation time and match the recorded installed GUI hash and ProductCode.

### Corrections and retained failed attempts

- FigCraft's signing call now explicitly binds `-Files` to the GUI/CLI path array. PowerShell 5.1
  previously combined the remaining arguments into one invalid path, even for unsigned packaging.
  The corrected actual package passed native linking, ICE and all runtime workflows.
- LightCraft's test-only scratch-path helper rebuilds paths from native components, matching the
  backslash paths returned by Windows directory scans. Production code and assertions are unchanged;
  the complete engine suite passed after the fix.
- EffectCraft's performance-sensitive tracking/raster crates, LightCraft's faces/raster/codecs/scenes,
  PdfCraft's engine/JavaScript/XFA and Boa crates, and PhotoCraft's engine/document/composition/
  operations/paint/raster crates use external opt-level 3 overrides. Other acceptance crates use
  opt-level 0, debug 0 and no LTO, with static CRT and two build jobs. LightCraft, PdfCraft and
  PhotoCraft retries run the complete suites serially. Original performance deadlines, XFA budgets,
  PhotoCraft nesting depth/stack limits and test assertions were retained. Original failures and
  complete successful retries remain in the evidence.
- Harness corrections cover CLI version invocation, alias removal timing, nonempty settings,
  PowerShell 5.1 nested arrays and final audit object aggregation. Settings automation reacquires
  the confirmation after settling, checks foreground state and allows two minutes for observed
  interactive UAC consent. Final acceptance requires the actual removal dispatch and exit; a
  helper status alone is insufficient. EffectCraft's direct screenshot-guided Settings fallback
  and PdfCraft's consent observation timeout are retained with their subsequent verified removals.
- PhotoCraft displayed its CPU compositor fallback notice on the VM's Microsoft Basic Render Driver.
  The notice was observed and accepted, and the installed app continued successfully.

Astra's source and harness follow-ups accepted these fixes without remaining actionable findings.
The final evidence review is recorded in the task's `astra-review.md`.

### Evidence and qualification limits

Durable host evidence is under `/home/codex/work/evidence/installer-real-apps-20261012-03`.
Guest evidence is under `C:\CraftApps\evidence\installer-real-apps-20261012-03`, with desktop
screenshots in the clone's `state/`. It includes native logs and failed attempts, source identities
and deltas, unsigned MSIs/portable archives, validation results, actual Settings dispatch records,
completion screenshots, settings hashes and the final acceptance JSON. The final source delta
archive also records subsequent host documentation changes separately from compiled inputs.

This qualifies x64 installer behavior, real GUI startup, CLI execution and configuration retention.
Document editing/save/reopen workflows, full workspace CI/WASM, external fonts/corpora, optimized
release performance, signing, native ARM64/x86 app execution, Windows 10, physical GPU/audio,
accessibility/DPI and locked-file/reboot outcomes remain outside this run. Earlier x86/ARM64
installer fixture qualification remains recorded above.

Final session cleanup: Settings and all apps are closed, only this task's temporary scheduled-task registrations were removed, and the clone was gracefully stopped with `running=false` confirmed. No guest files were deleted. All VM state, native targets, sources, unsigned deliverables and evidence are retained. This task created no host Cargo build directory, so there were no host build outputs to remove; pre-existing tools and other sessions' artifacts were left intact. `cleanup.json` records retained paths and host free space before/after.
