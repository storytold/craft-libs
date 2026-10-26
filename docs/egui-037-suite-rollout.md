# Craft suite: egui 0.37 rollout and acceptance plan

Status: local implementation underway, updated 2026-10-24. The reviewed local egui fork is
`173cc7375d35dc5fbc5b5d0994fdf787f547c5d1`; the reviewed local winit fork is
`ae48c756a3b81125eb796c0cc6dccf60990d1181`. craft-libs is reconciled on merged PR 199 and now
records those immutable canonical sources in its manifest and lock. Its 118 craft-ui tests, strict
package clippy, canonical source audit and formatting checks pass. Full workspace, platform/native,
accepted comparative performance and clean post-publication remote-fetch gates are not implied.
No push or new pull request is authorized.

The inventory below retains the 2026-10-22 planning snapshot and source references for provenance;
the current repository status supersedes its old implementation/PR-conflict statements. Every
repository has a separate `docs/egui-037-migration-plan.md`. This document defines the shared
dependency order and acceptance contract; each repository plan supplies its actual source anchors,
commands, fixtures and behavior-specific assertions.

## Scope and current dependency state

The inventory covers 15 applications: CADCraft, DeckCraft, DesignCraft, EffectCraft, FigCraft,
FilmCraft, GridCraft, LightCraft, PdfCraft, PhotoCraft, SoundCraft, VectorCraft, WordCraft,
ArtCraft Launcher (`craft-launcher`) and ArtCraft Native (`artcraft/artcraft-native`).
It also covers `craft-libs`, `craft-ai-integrations`, `storytold/egui` and the required
`storytold/winit` companion fork. CLI, web, headless, examples, benchmarks and nested workspaces
that consume these libraries belong to their repository's migration.

The first 14 applications were present in the earlier suite audit; ArtCraft Native and
`craft-ai-integrations/crates/ai-egui` must not be lost from the rollout. `craft-fonts` and
`craft-i18n` are relevant data/API inputs, not egui rendering consumers in the inspected sources.
The older `storyteller-bevy` studio and `bevy-mocap` examples use Bevy-specific integrations
(`bevy-inspector-egui` 0.23 and 0.22); they are outside this Craft suite/fork migration. A
future expansion to those projects needs their own Bevy compatibility plan, not a root egui override.

The [launcher's pinned catalog](https://github.com/storytold/craft-launcher/blob/9419d8977bc70f80373da8558356e4ba7387c7b2/crates/core/src/catalog.rs)
enumerates the 13 creative apps above. Repository discovery also covered all 68 repositories
returned for `storytold` (the next page was empty), with a separate code search excluding the
19 migration repositories so one large app could not crowd out smaller consumers. Search is a
discovery aid, not proof about unindexed code. Manifest/source inspection confirmed the additional
ArtCraft Native and AI UI consumers. ArtCraft-X is a Tauri/Vite frontend, the repository-status
app uses a TypeScript UI, and ArtCraft Services' inspected workspace contains service/CLI code;
the latter's old Docker comments mentioning egui are not a dependency declaration. Asset/data
repositories and the empty AniCraft/CraftHub repositories do not require an egui port. Re-run
this inventory before final integration to catch newly added consumers or branches.

Most inspected suite roots still resolve egui 0.36.2. CADCraft at
`1977a493c7d8340249185126744c5143f096dc86` and SoundCraft at
`52471b37e957feaea56dfd48932ef16d314cc74a` have already bumped direct dependencies to 0.37,
while their pinned `craft-ui` still requires 0.36. Both lockfiles therefore contain two egui versions.
Their first deliverable is a coherent graph and preserved behavior, not another version bump.

The existing application code already uses the root-Ui lifecycle on 0.36.2; some web shells
implement only `App::ui` and inherit default `logic`. Do not attribute that split to 0.37
or rewrite it mechanically. Inspect the remaining incompatibilities and
revalidate the lifecycle, especially hidden windows, child viewports and repeated UI passes.

## Pinned starting points and existing work

| Source | Exact reference | Use |
|---|---|---|
| Upstream egui 0.37.0 | `841a328ba2dd8f6996beb7189ebb7fc00e668a2a` | Release commit, not moving main or the annotated tag object |
| Upstream egui 0.36.2 | `49682f8baa058bf49e011035cfbd6e825f88a5ef` | Stock comparison, where useful |
| Published storytold egui | `767b1cad294120689a514bb4bd68c077e46679b9` | Existing `craft/0.36.2` patch inventory |
| aiongg egui candidate | `a4f9a17f64a6bae61350fb85804c9df0d9865916` | `craft/0.37`; review E1–E21 before integration |
| Published storytold winit | `ffae5198770d7730e718043f035a772b7dbc3b1b` | Existing `craft/0.30.13` behavior |
| aiongg winit candidate | `8b35388bd719e8cc86a95971ba0bc026c9a8f058` | `craft/0.30.13-lang`; review companion patches and downstream vendors |
| craft-libs plan baseline | `49190884278f16c9a9f5823d7698a8ffb5deb062` | Current shared-library tree used by this plan |
| craft-libs PR 199 | `b68de8c5f4f1c060904d211a7b1320aa0f7bce78` | Existing aiongg API port; open and conflicting at inspection |

[PR 199](https://github.com/storytold/craft-libs/pull/199) supplies reusable API-port hunks,
not a mergeable replacement tree. Its head `b68de8c` and historical audit tree `ec8840a`
omit current `craft-platform` and revert security, settings, docking and printing fixes.
Apply the port from `8614e4a04fae516fba16c22b31ff1008cb6be10b` and relevant README
changes from `1764bca` onto current `4919088`, preserving intervening fixes and provenance.
Re-run actual gates; neither the PR nor historical checks qualify the resulting tree.
[Issue 198](https://github.com/storytold/craft-libs/issues/198) is a broader language-support
proposal, not a merged dependency. Its locale/formatting roadmap is not silently included in
this upgrade. Changes required to consume the selected fork correctly are included: remove
double bidi processing, maintain logical text at OS/accessibility boundaries, preserve IME and
shortcut behavior, and ensure installed/bundled font coverage under each app's asset policy.
Do not import prohibited font assets from the proposal; existing licensing rules still apply.

The candidate also has a confirmed IME integration gap: winit W27 exposes outgoing
surrounding-text support but egui-winit never sends it. Egui E21b/P7b adds the bounded
transient payload, TextEdit producer and Wayland adapter. Word/Vector/Fig/Effect/Film/
Design/Deck/Photo custom editors require model/preedit/byte-index adapters; this is
part of their atomic compatibility work. Outgoing state, focus/password clearing,
long selections and native IME evidence are explicit acceptance gates. The input-owner generation
exists even for `Unavailable`; an empty available document is distinct. W27 must clear cached and
protocol surrounding state at an owner/unavailable transition using a committed disable/enable
boundary and cancel old pending preedit/commit/delete batches. Ordinary updates retain composition.
Within a current focus epoch, mismatched text-input-v3 `done` serials still apply incoming edits
once while deferring outgoing state until a match; old epochs cannot edit a new owner. Egui's
latest completed pass, including `IMEOutput = None`, replaces earlier IME output. Fork P7b/W27
own these fixes and tests before downstream compatibility PRs; see their detailed contracts.
P7b also separates the web TextAgent DOM-position equality key from transient IME payload
equality, testing browser composition and no extra style writes for unchanged caret geometry.

The winit plan must pin the candidate's full SHA and compare its tree against the published
`craft/0.30.13`. The selected egui language APIs and winit language APIs form one compatibility
unit. winit remains 0.30.13 and wgpu remains 30 unless a separately justified change is needed.

## Implementation and publication dependency order

1. Finish each plan, then obtain a fresh independent adversarial review of that exact document.
   Resolve concrete findings, retain the review and final document hash, and prepare documentation
   PRs. Obtain publication approval after showing the proposed branches, files and checks.
   Implementation begins after the plans land; local drafts are not described as landed.
2. Add regression and benchmark harnesses on the recorded baseline revisions before changing
   production behavior. Record pre-existing failures separately. For CADCraft and SoundCraft,
   locate the last coherent pre-bump revision; keep both the current broken graph and the
   coherent performance baseline identified. No fabricated baseline from a failing binary.
3. Integrate the reviewed winit patch series, new W15P PRIMARY support and tests. Integrate egui
   E1–E21 plus E21b outgoing IME state and E4/P2b PRIMARY integration in dependency order,
   including P7 held-key pairing and P7c selection-action/hidden-child fixes, then final P8;
   include only justified performance fixes. Preserve patch authorship, source links, test mappings
   and removal conditions. Publish approved dependency commits before consumers reference them.
4. Port craft-libs using the selected PR 199 hunks, including `craft-ui`, egui-enabled
   `craft-quit`/`craft-window` and all relevant optional features. The current shared workspace
   has no binary/example targets; excluded vendor examples retain their separate disposition.
   Add narrowly justified shared helpers and their
   regressions. Port `craft-ai-egui` in parallel once its dependency contracts are fixed.
5. Port each application in its own reviewable PR series. Atomic compatibility changes include
   manifests, lockfiles, source adaptations and harness changes; never land an intermediate
   dependency bump that leaves main unbuildable. Eleven craft-ui consumers adopt the capture
   helper from CL-1 + CL-2F in that atomic stage; DesignCraft/GridCraft explicitly pilot the
   reviewed CL-2 prototype before CL-2F. Only Deck/Fig/Art Native use local capture adapters.
   Non-capture helpers may follow separately when compatible. Required IME/bidi adapters
   belong in the first fork pin; later language-policy and expanded coverage PRs stay focused.
6. Run per-app functional, visual, platform and performance acceptance, then a fresh independent
   adversarial implementation review at the exact candidate revision. Fix findings and re-run
   affected checks. Perform a final suite-wide dependency and capability audit before rollout.

Consumers can be implemented in parallel after the shared contracts stabilize, using separate
source checkouts, build resources and test VMs. Start integration with ArtCraft Launcher plus
CADCraft/SoundCraft to expose graph/API problems, then the office apps and complex canvas/media
apps. This order is risk management, not permission to skip a later app or its platform checks.
FilmCraft/EffectCraft cross-repository pins require an explicit reverse-dependency check.

Each push and each new PR still requires the user's explicit approval for that publication.
Implementation authorization does not authorize pushing, merging, releasing or closing other
contributors' issues. Public descriptions contain the problem, change and validation, without
agent/model/tool signatures. Contributor metadata remains username-only.

## Dependency and vendoring acceptance

- Record manifests and lockfile package IDs, sources, versions, revisions and features for every
  root and nested workspace. Inspect target-specific, dev and build dependencies, not just the
  default executable. Parse `cargo metadata` and `cargo tree` under native and wasm feature sets.
- Every selected egui family crate, including tests, renderers, `epaint_default_fonts` and
  `egui_system_font_provider` when used, must resolve from the intended immutable fork revision.
  A package version match alone is insufficient: registry and git packages have different identities.
- Root `[patch]` sections must exist where resolution occurs; a dependency's patch section is
  ignored by its consumer. Verify fork-specific APIs under real consumer roots. Align craft-libs
  revisions wherever shared public types cross crate boundaries; do not churn unrelated pins blindly.
- Resolve features for every accepted native/wasm/package-test root with `cargo tree -e features`.
  Egui 0.37 has no default `default_fonts`; eframe's default feature set is different. Each root
  declares its bundled/provider policy and passes a no-tofu fixture without relying on another
  workspace member's feature unification. Check clipboard, accessibility, color-font and renderer
  features explicitly. All-family source audits include optional lock-only crates such as `egui_glow`.
- Record the full coherent version/source set in a canonical manifest and check all consumer
  roots against it in CI. Tooling must catch a missing patch, duplicate egui/winit, an old transitive
  `craft-ui`, a moving branch dependency, and a machine-local path. Test these failure cases.
- Compare PdfCraft's vendored winit and egui-wgpu fixes against the selected fork semantically,
  including fixes added since the old audit. Transfer regression coverage before deleting only
  superseded vendors. Preserve krilla, hayro (including hayro-cmap), lopdf, vello and VectorCraft's windows-link patches unless a
  separate comparison proves a migration-related replacement. Review open Android/IME/Wayland
  vendor work as evidence; do not merge unrelated changes or close its PRs as a side effect.
- A clean checkout must fetch every approved revision and pass `--locked` without local overrides.
  Before merge, recheck remote heads for drift and rebase/revalidate affected source and evidence.

The initial root-patch disposition is exhaustive for the 19 recorded roots. Nested workspaces,
vendored manifests and command-line overrides remain separately enumerated in the static
inventory and must be checked under their actual resolution roots:

| Root | Existing override | Migration disposition |
|---|---|---|
| DesignCraft | winit `storytold/winit@3d51077fd1a966f84df9ef287839dc84fd5b4ca7` | Replace with final reviewed W14/W15P/W27-compatible storytold revision; preserve cancelable-quit regression |
| PdfCraft | `vendor/winit`, `vendor/egui-wgpu` 0.36.2 | Transfer all window/GPU fixes and guards centrally, prove equivalence, then remove these overrides/vendors atomically |
| craft-libs | local krilla, hayro, hayro-interpret, hayro-syntax, hayro-jbig2 | Retain PDF/printing fixes, provenance and tests |
| DesignCraft | krilla and four hayro packages at craft-libs `89dcbb3060377a26c99a6d80caa16084366704c4` | Retain independently of the UI pin |
| DeckCraft, WordCraft | krilla at craft-libs `c348732…` | Retain exact existing revision from each manifest |
| EffectCraft | hayro-syntax at craft-libs `ca50d63b773c99f96e75c77ce5a1d21e7c9cda0d` | Retain |
| VectorCraft | hayro-syntax, hayro-interpret, hayro-jbig2, krilla at the same `ca50d63…` | Retain; also preserve Windows 7 packaging's command-line windows-link path override and its dedicated test |
| PdfCraft | local hayro, hayro-interpret, hayro-syntax, hayro-cmap, hayro-jbig2, lopdf, vello_common | Retain unrelated PDF fixes |
| Other 12 roots | No root patch at the recorded source | Add required immutable family/winit overrides at the actual consuming roots |

Candidate fork branch dependencies are research inputs; final manifests use approved reachable
storytold revisions containing the completed series. CI checks actual package IDs, not textual
version matches. Negative fixtures inject DesignCraft's old winit override, PdfCraft's old path
override, a missing family patch and an unapproved moving branch; each must fail the graph gate.
Inspect inverse trees for winit and egui-wgpu under every selected root/target/feature set.
The canonical inventory contains all 19 sources, including ArtCraft Native, AI UI and winit;
gate inventory/ledger source-set equality and reject missing or unreadable tracked manifests.
The synchronized `dependency-inventory.json` at this planning workspace root and in the task's
`ssd2/task-evidence/egui037-migration-20261022` directory contain the same 19-source inventory;
record its hash in each gate, compare against Git's tracked manifests, and reject stale copies.

## Shared components: justified boundaries

Screenshot completion has 14 consumers: CADCraft, DeckCraft, DesignCraft, EffectCraft, FigCraft,
FilmCraft, GridCraft, LightCraft, PdfCraft, PhotoCraft, SoundCraft, VectorCraft, WordCraft and
ArtCraft Native. Eleven creative apps already depend on craft-ui and will adopt its new helper after CL-2F
(the DesignCraft/GridCraft pilots first use the reviewed CL-2 prototype).
DeckCraft, FigCraft and ArtCraft Native do not depend on it and keep small local adapters
with the same tested contract. Exact manifest-key inspection distinguishes `craft-ui`
from each app's own `*-ui-egui` crate name. The launcher has no screenshot consumer.
egui 0.37 replaces screenshot input events with a one-shot callback.
The screenshot command itself requests a hidden paint; callback arrival does not. A request
queued in app logic but issued only in UI, including every remaining settling frame, must
send `RequestPaintWhileHidden` to reach issuance. A wake alone may run hidden logic only.
The callback receives only `Arc<ColorImage>` and does not schedule another repaint itself.
See the [release callback implementation](https://github.com/emilk/egui/blob/841a328ba2dd8f6996beb7189ebb7fc00e668a2a/crates/egui/src/data/screenshot_callback.rs).

Keep app request IDs, output paths, serialization, timeouts, settling policy and reply schemas
in each app. A shared mechanism may provide a bounded, cancelable per-request completion slot,
capture correlation and viewport-aware wakeup. The callback must not encode PNGs, write files,
run commands, hold a long lock or borrow mutable app state. After timeout/close, a late callback
must release its image without sending another reply. Limits must bound pending images and
requests; counter wrap and a reused viewport must not bind an old capture to a new document.

The selected starting design is one completion slot per admitted request, with the callback holding
only a weak reference to that slot. Several app plans need the same one-result lifecycle, while
their request records already own IDs, deadlines and reply routing. Keeping the slot inside that
record gives correlation without a second global token map. Dropping/canceling the record releases
its image; a late callback cannot retain the app or produce a file/reply. Keep any context/wakeup
handle in the live slot, not strongly captured by a callback that may itself be queued in that
context. Release the slot lock before waking the owning viewport. The app retains admission limits
and must bound aggregate pending image memory, not merely the number of results in each slot.

The default capture admission policy is 4 pending requests, 128 MiB image bytes per request
and 256 MiB aggregate reserved in-flight/completed bytes; apps may select stricter explicit
limits (CADCraft uses its documented smaller budget). Use checked dimensions × 4 arithmetic,
reject excess admission without evicting earlier requests, and measure transient readback/encoder
RSS separately. Callback arguments contain pixels only. The default adapter records target-UI
logical content/screen bounds and a geometry epoch at issuance, then rejects/retries any intervening
resize/DPI/zoom/surface epoch change before save, even a change back. With an unchanged epoch,
derive pixel scale from returned image dimensions and verify aspect/rounding. Do not assume
Wayland `inner_rect` exists or that callback arrival supplies capture metadata. A renderer-specific
frame annotation is an alternative only after an actual tested API is established. Distinct-image
correlation tests manually complete callbacks in reverse order; kittest's ordinary renderer shares
one image across callbacks, so real two-window pixels require native evidence. Before
baselines, reconcile every app admission table to these defaults or stricter limits; no
app silently retains 8/16 requests or larger per-image/aggregate budgets.

Stress concurrent captures, cancellation, shutdown, delayed GPU readback, out-of-order completion,
memory and hidden-window wakeup. Compare a bounded nonblocking channel prototype with explicit
overflow/error handling if these tests or measurements expose a weakness in the slot design.
Do not encode/save images on the callback thread or add a generic UI compatibility framework.
Applications without craft-ui need not adopt the entire crate solely for this helper. Prototype
the shared API in CL-2, then integrate DesignCraft's multiple viewports and GridCraft's single
window against an approved provisional revision. Review those adapters before CL-2F freezes the
API. This is an initial topology check, not a reduction of scope: all eleven shared
consumers and the three local adapters must pass their capture/cancel/close/DPI tests.

The DockArea direction policy is explicit and opt-in: custom RTL tab/control chrome,
hit tests, drag insertion and keyboard traversal share one geometry calculation;
serialized physical splits, rectangles and logical tab order are preserved. The
shared-library plan defines the builder and tests. No app may reverse saved panel
order to simulate RTL.

Other candidates are a tested wheel-event adapter that preserves `MouseWheelSource`, and focused
font/direction/IME helpers with multiple named callers. Retain native widget roles/names rather
than hiding all role changes behind `Unknown`. Existing app themes, units, document models,
canvas engines and undo stacks remain owned by the applications.

## Shared regression matrix

| Risk | Required observable assertion |
|---|---|
| Callback screenshot routing | Distinct requests for two viewports receive the right pixels and exactly one reply; timeout/close/cancel prevents a late file write or reply; pending queue drains |
| UI passes and hidden logic | A forced second UI pass does not duplicate a command/undo step; background results, autosave and media continue under logic-only iterations without replaying stale input |
| Hidden parent and child viewports | With the parent occluded or overdue, open, update and close a child; the visible child receives content and close acknowledgement without restoring the parent |
| Root UI and panels | Existing Ui is passed through; no recursive `root_ui` borrow; panels reserve space in order; canvas geometry, clip and hit positions agree after resizing |
| Stable widget identity | Changing language, hiding a panel or inserting/reordering a row does not transfer focus/edit/drag state to another item; persisted workspace restores |
| Accessibility | Named inputs expose correct roles, values, selected/disabled/focus states; hidden controls disappear; tab/arrow/Enter/Space actions trigger the same commands |
| Fonts and text | Controlled licensed fonts cover fixture glyphs; no tofu, clipped combining marks or doubled bidi; logical clipboard/save/accessibility strings remain unchanged |
| Selection regression | Select text, click a toolbar/context action and verify the intended selected range is used; pointer and keyboard paths agree |
| IME/graphemes | Preedit, replacement, commit and cancel produce one intended edit/undo step; surrounding byte ranges cannot split UTF-8; emoji clusters and RTL caret/selection remain consistent |
| Scroll and zoom | Wheel, trackpad and momentum preserve source, phase and modifiers; overscroll uses the correct clamped/unclamped coordinate for painting and hit tests; zoom stays anchored |
| Input/native platform | Child focus/close/reopen, mixed DPI, native pickers, clipboard/PRIMARY, file drop and pen/tablet behavior are observed in real native workflows |
| Wayland PRIMARY | Password selection is never offered; unchanged selection does not reclaim ownership; pointer- and keyboard-triggered ownership (including keyboard-only focus), fresh triggering serials per seat/origin, owner/seat changes, empty/absent offers, self-paste and slow/canceled transfers work under wlroots, KWin and Mutter with supported capabilities recorded |
| GPU/resources | Surface limits/configuration errors do not panic; textures are updated/freed correctly after DPI/resize/device errors; color emoji and nearest/linear/wrap sampling stay correct |
| Persistence | Edit, save, close and reopen a synthetic document plus layout; compare document meaning, selection/workspace expectations and undo semantics |

Use deterministic synthetic fixtures and explicitly pinned fonts/DPI/theme/viewport. Test small
windows and 1x/1.5x/2x scaling in both themes; add RTL/CJK/combining/emoji cases where the app's
UI or document text uses them. Keep geometry and semantic assertions alongside rendered images.
Review each intended image change; never bulk-refresh goldens to make CI pass. Demonstrate detection
with bounded negative controls (wrong callback/viewport association, missing accessible name, wrong clip/offset,
dropped callback or forced duplicate command), then remove the deliberate defect.

Rerun's [testing guide](https://github.com/rerun-io/rerun/blob/42677ef9b096abbce742f0929dea58bb75f740ad/TESTING.md)
provides relevant prior art: drive viewer interactions through the harness and examine image diffs.
Apply that principle to Craft workflows; direct state setup is for fixtures, not a substitute for
clicking the changed control. Use egui_kittest's strict missing-name and missing-glyph behavior.
Native IME, focus, system dialogs and compositor behavior still need native evidence.
W15/W15P use bounded asynchronous read/write transfer handles, a total monotonic deadline,
byte/request/worker limits and cancellation. The candidate's two-second inactivity poll timeout
is not a total deadline. Egui E4/P2b routes completions only to a live paste request with unchanged
input owner/generation; focus changes cancel it. Test slow trickles, oversized offers, cancellation
and concurrent peers while measuring event-loop responsiveness. Serial selection uses the actual
triggering pointer/keyboard event and seat; no arbitrary first-seat fallback.

Text conformance fixtures must match the data in the selected dependencies: the candidate locks
`unicode-bidi` 0.3.18 (Unicode 16) and `unicode-segmentation` 1.13.3 (Unicode 17). Test the fork's
layout/editing adapters against versioned [bidi](https://www.unicode.org/reports/tr9/) and
[grapheme](https://www.unicode.org/reports/tr29/) cases; do not silently use the latest data as the
locked contract. For [Wayland text-input-v3](https://cgit.freedesktop.org/wayland/wayland-protocols/tree/unstable/text-input/text-input-unstable-v3.xml),
assert the full preedit/deletion/commit sequence, valid UTF-8 byte boundaries and freshly updated
outgoing surrounding text. Cover deletion-only batches, focus/reset/serial changes and two windows.
A [similar Mozilla report](https://bugzilla.mozilla.org/show_bug.cgi?id=2080478) motivates the
stale-surrounding-text case; it is not evidence that this fork has that bug.

## Performance protocol and stop conditions

Capture baseline before production edits. Rebuild baseline and candidate with the same compiler,
profile, features, fonts, fixture, renderer/backend, driver, viewport and DPI. Record full Git and
lockfile identities, CPU/GPU/OS, raw samples and any shared-host load. Use separate allocations,
then alternate execution order to reduce thermal/cache bias. Never benchmark concurrently with
builds or another noisy workload and claim a small improvement.

Use at least 10 warmups and 30 timed iterations for steady operations, and at least 20 fresh
processes for startup. For frame distributions capture at least 1,000 frames and report p50/p95/p99;
do not claim a reliable p99 from 30 samples. Report median/p95 for discrete operations with the
sample count, a paired confidence interval where suitable, peak RSS, allocations/texture uploads,
idle CPU/repaint count, and cold versus warm caches separately. Preserve startup/font-provider
timings and raw/compressed WASM sizes. Software renderers and real GPUs are separate results.

Preliminary gates, to be frozen in baseline reports before candidates are measured: investigate
a repeatable >5% median or >10% p95 latency regression, >5% peak-RSS increase, any new idle
busy-loop or audio underrun, and any loss of the app's existing absolute performance budget.
For sub-millisecond measurements require the relative increase also exceed 0.05 ms or corroborate
it with stable instruction/allocation counts. Reject noisy comparisons as inconclusive and rerun;
do not expand tolerances after seeing a failure. Any exception needs a documented tradeoff and
review before acceptance. Existing stricter repository budgets win.

These sample counts are minimums and the investigation thresholds are maximum tolerances across
the suite: an app may tighten them, never silently loosen them. Any proposed exception must be
documented and approved before candidate measurements; an exploratory small sample is not tail
acceptance. Reconcile every app's table against this contract before baseline capture. In
particular, the earlier FilmCraft five-warmup, EffectCraft three-warmup/relaxed startup and winit
three-warmup proposals are superseded: use the shared floor and investigate at the shared latency
thresholds unless an explicit pre-measurement exception has been approved. FilmCraft’s old
absolute +20 ms startup allowance is also superseded by the shared relative thresholds.

The first baseline report and every investigated regression include an equivalent stock-0.37
control alongside 0.36 and the selected fork. Fork-only app code need not compile unchanged on
stock: use the same operation in a pinned minimal adapter/harness and explain any remaining
semantic differences. Record all three source/lock identities; unavailable equivalence remains
an attribution limitation, not evidence that an upstream regression is unavoidable.

App plans select meaningful workloads: canvas pan/zoom/selection and text editing; spreadsheet
scroll/recalculation; timeline scrub/playback and waveform updates; slide transition/presenter
windows; thumbnail grids and RAW previews; PDF page navigation/annotations/search; launcher catalog
and download progress; AI job history/progress. Include idle and first-use paths, not just engine
CLI throughput. Rendering hashes and document state must remain correct during measurement.

Three upstream reports require targeted reproduction on our own fixtures and environments:

- [#8731](https://github.com/emilk/egui/issues/8731): plain-text tessellation regression associated
  with color-glyph handling. Compare unchanged 0.36, stock 0.37 and selected fork. If confirmed,
  evaluate a row-level fast path and cached color-glyph metadata while testing mixed/color rows.
- [#8736](https://github.com/emilk/egui/issues/8736): DX12 FXC shader compilation cost even when
  predictable filtering is disabled. Measure renderer construction separately from first frame.
  Compare an override-constant design with semantically equivalent wrap arithmetic only if needed;
  clamp-only output is not a valid fix for Repeat/MirroredRepeat.
- [#8512](https://github.com/emilk/egui/pull/8512): a system-font startup-cost report in the
  discussion. Measure discovery separately, preserve glyph coverage, and compare current app
  loaders with explicitly configured system providers before enabling a new default everywhere.

These are external reports, not reproduced measurements or promises of a speedup.

Known functional reports also require explicit decisions before the fork is frozen:

- [#8763](https://github.com/emilk/egui/issues/8763) and unmerged
  [#8764](https://github.com/emilk/egui/pull/8764): reproduce selection loss when clicking an
  outside toolbar/context action. The fork owns the selected fix, with pointer and keyboard
  regressions proving that the intended range reaches the action in that pass. A necessary app
  workaround must preserve the range and undo semantics and have its own removal condition.
- [#8748](https://github.com/emilk/egui/pull/8748) is closed and unmerged, not behavior in the
  selected release. The fork owns a tested hidden-parent/deferred-child solution; exercise
  overdue redraw with unknown visibility as well as actual occlusion/minimization. Do not infer
  phase from `visible()`, or describe ordinary logic-only calls as full UI passes.
- [#8657](https://github.com/emilk/egui/issues/8657) and
  [#8428](https://github.com/emilk/egui/pull/8428) inform the clipboard/PRIMARY matrix;
  [#8577](https://github.com/emilk/egui/pull/8577) informs logical-text/bidi regressions.
  Their status is evidence to inspect, not permission to assume a reported behavior is fixed.

For a reproduced required workflow failure, a reviewed fork fix or tested app workaround is
required. An explicit deferral records an unresolved rollout gate; it is not a passing result.

## Platform evidence, review and rollback

Each app's required gates include its existing fmt/lints/tests/assets/layers/wasm commands plus
selected feature checks that default CI misses. Record actual selected test counts and failures.
Linux covers native X11 and Wayland with compositor identity; Windows uses a uniquely owned
baseline clone and visible edited/saved/reopened workflows. Browser workflows must run, not merely
compile to wasm. macOS IME, Metal/GL, keyboard layouts and native termination need real macOS
evidence; cross-compilation is not acceptance. FreeBSD/Android rows follow the app's support claims.
Unavailable environments remain explicit rollout blockers, not implicit passes.

Current readiness and allocation plan (recheck immediately before each wave):

| Wave/environment | Current evidence | Required preparation and capacity gate |
|---|---|---|
| Static planning | 19 owned checkouts; no tests/builds run | No build reservation needed; preserve review artifacts |
| Shared/fork CPU and headless regressions | No qualified runtime yet | Estimate one baseline/candidate build's growth from dependencies, reserve through task.py, execute sequentially and preserve paired binaries; defaults do not fit current capacity |
| Linux X11 | No inherited DISPLAY; Xvfb/window manager absent at inspection | Provision an isolated display/window manager, record versions and qualify native focus/clipboard; label virtual-display/software results accurately |
| Linux Wayland | No inherited WAYLAND_DISPLAY; Weston/Sway/KWin absent | Provision the compositor/input-method environments required by the matrix; qualify keyboard/IME/PRIMARY protocol behavior and identify unsupported capabilities |
| Linux GPU | Parent can access i915 8086:3E96 renderD128; worker sandbox may hide it | Managed renderer/device probe must identify actual driver/backend; access alone is not GPU acceptance |
| Windows | Sealed baseline uses Basic Render Driver; 128 GiB default clone cannot currently fit | Estimate guest source/build/evidence growth; allocate a unique full clone if capacity permits or a baseline-backed overlay with an adequate reservation (at least 10 GiB); otherwise block this wave |
| Browser | Wasm compile is not runtime evidence | Provision/pin browser and WebGPU/WebGL backend; run interactive fixtures and record browser/adapter identity |
| macOS and physical Windows/audio | No qualified environment in this task | Obtain real platform access and owned execution/evidence before those gates pass; software VM results do not qualify these rows |

The allocator snapshot had about 23.6 GiB and 37.7 GiB allocatable on the two separate drives;
they cannot be combined for one target. Overlay creation is already authorized by the standing
workflow; its budget must cover expected growth, not merely the minimum. Per-wave reservations
are set from that estimate and current capacity before submission. Do not invent a fixed small
budget now to claim all platform gates are ready.

All new host tests/builds and owned-VM commands use the managed run/status/finish workflow.
Keep exact source, evidence and resource identities; export logs, screenshots, fixtures, hashes,
source patches and deliverables before cleanup. No existing job/VM is adopted or stopped.

Schedule builds and VMs from current allocator capacity, not from the number of available review
workers. At planning inspection, both drives together had less than one default 64 GiB host-build
reservation available after existing reservations. Before each validation wave, inspect capacity
and budget each owned task from its expected growth; do not lower an estimate simply to pass the
guard. Start only the workloads that fit, retain paired baseline/candidate executables outside
disposable targets, and finish a resource only after its required reviews and evidence are complete.
Use a smaller reservation only for a demonstrably smaller workload. Resize before substantial
growth. If a required build or clone cannot fit, preserve its records and report the capacity gate;
do not reclaim another task, move an active allocation or bypass the allocator. Keep performance
measurement sequential even when correctness builds can safely run concurrently.

Every plan and every implementation gets a new independent reviewer with a bounded assignment.
Reviewers inspect source, tests, dependency identities and negative cases; a passing build or an
author's own checklist is insufficient. Track finding, severity, evidence, resolution, recheck and
reviewed hash. No unresolved correctness finding is silently downgraded to a future task.

The authoritative ledger records all 19 repository plans plus this suite document separately:
current hash, review-request ID, reviewed hash, verdict, finding resolutions and fresh closure
review. Preserve earlier drafts and reports. A changed document invalidates its earlier approval;
implementation remains blocked until all 20 current documents have accepted reviews and their
documentation PRs have landed. The evidence copy of the ledger is a synchronized snapshot.

Rollback restores the entire previous coherent manifests/lockfiles/source set, not just an egui
version string. Keep prior dependency revisions reachable. Avoid persistence format changes for
this upgrade; if one is necessary, specify backward compatibility and preserve unreadable data.
Do not release a mixed suite dependency graph or delete old source/evidence during cache cleanup.
At each dependency/app integration stage, rehearse restoration of the previous coherent source,
lockfiles and settings using preserved source refs and the baseline executable. Reopen the saved
fixture and verify the old behavior before accepting the stage. If rebuilding is necessary, use
a separate managed allocation. Keep the old executable/source identities reachable through the
rollback window; restoring only an old lockfile does not constitute the rehearsal.


## Local implementation clarification — 2026-10-23

The user explicitly corrected the publication-first workflow: all reviewed plans stay local while implementation proceeds. Published dependency reachability remains a release requirement, not a condition for local prototypes, source ports, tests or reviews. Local development may use isolated candidate worktrees and explicit local Cargo patches; no machine-local path enters a publication draft.

Canonical integration sources were refreshed for all19 repositories; sixteen advanced by220 commits in total. See the task evidence `REMOTE-REFRESH-SUMMARY.json`, canonical ref/diff snapshots and the per-repository local-update appendices. Keep the original review source refs for historical comparison. Current-main paired measurements must use refreshed integration sources so unrelated security/performance work is not attributed to the toolkit migration.
