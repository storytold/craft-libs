# Craft-Libs Fork Technical Specification & Upstream Delta Guide

> **Repository:** `ussoewwin/craft-libs` (Branch: `next`)  
> **Upstream Tracking:** `storytold/craft-libs` (Branch: `next`)  
> **Target Consumer:** PhotoCraft & Native Craft Applications  
> **Author:** ussoewwin  
> **Standard:** Strict Rust (Clean Room, `unsafe_code = "deny"`, Zero Panics / Zero Unwraps in Non-Test Code)  

---

## 1. Overview & Architectural Scope

This specification provides comprehensive technical documentation for the `ussoewwin/craft-libs` fork. `craft-libs` is the shared Rust foundation powering the Craft application suite (including PhotoCraft, ArtCraft, DesignCraft, and VectorCraft).

The fork maintains strict synchronization with official upstream (`storytold/craft-libs:next`) while introducing critical architectural fixes, host stability improvements, format decoder enhancements, and Photoshop 1:1 behavioral parity.

### Active Core Fork Enhancements
1. **Host Accessibility Crash Prevention (`craft-ui-contract`)**: Eliminates unwrap panics in Windows UI Automation (UIA) adapters triggered by transient dangling relation references in popup/menu transitions (`drop_dangling_relations`). Upstreamed via PR #2328.
2. **Canvas Space-Key Hand Tool Preservation (`craft-ui-contract`, `craft-ui-panels`, `craft-ui-widgets`, `craft-pages`, `craft-color-ui`, `craft-draw`)**: Resolves keyboard event hijacking where pressing `Space` inadvertently activated previously focused widgets instead of delegating to the canvas Hand/Pan tool. Preserves explicit opt-in for `Button` and `FlyoutButton` via `with_space_activation()`.
3. **Resilient Peripheral Retention (`craft-print`)**: Prevents erasure of remembered printer configurations caused by transient spooler polling delays.
4. **Comprehensive East Asian Typographical Classification & Consolidated Localization (`craft-font-manager`)**: Introduces full CJK font classification heuristics (Mincho, Gothic, Maru Gothic, Kaisho, Pop), Photoshop-parity Japanese font name localization, canonical Kana sample previews (`あいうえお`), and consolidated font localization with `localized_families` fallback.
5. **Merged PSD Layer Transparency & Layr Block Handling (`craft-psd`)**: Accurately recognizes merged transparency from global `Layr` blocks with negative layer counts and ensures zero-count layers fall through to inspect `Mt16` / `Mt32` tagged blocks.
6. **Large ZIP Archive EOCD64 Scanner & Zip-Bomb Protection (`craft-zip`)**: Implements chunked scanning with overlap carry in `scan_offsets`, ensuring archives with stubs/prefixes larger than 64KiB correctly identify ZIP64 end-of-central-directory records and enforce entry limits.

### Architectural Harmonization with Upstream (v1.0.3)
- **Title Bar & Menu Bar Layout Alignment (`craft-ui-shell`, `craft-menu`, `craft-ui-widgets`)**: Upstream modernized the title bar layout pipeline via `TitleBar::show_titled`. Custom width negotiation hooks (`leading_width`, `own_width`, `inline_width`) were retired in v1.0.2 to restore clean upstream code alignment while ensuring all 10 menu items display side-by-side without overflow.
- **Tool Rail Localization Architecture (`craft-ui-panels`)**: Tool command translation was transitioned directly to the official `CommandState::title` system in `framework.rs`, allowing retirement of custom `RailLabels` hooks while strictly preserving `FlyoutButton`'s `with_space_activation()` keyboard policy.

---

## 2. Upstream Divergence Matrix

| Crate | Modified Files | Core Issue in Upstream | Fork Technical Resolution | Status / Upstream Alignment |
|---|---|---|---|---|
| **`craft-ui-contract`** | `src/runtime/egui/access.rs`<br>`src/runtime/egui/plugin.rs` | UIA adapter crashes with abort on Windows when `Node::controls()` unwraps IDs missing from current update (Issue `photocraft#15597`). | Prunes dangling control relation IDs in `output_hook` so only nodes carried in the current update packet are referenced. | Adopted upstream via PR #2328 / #2239. |
| **`craft-ui-contract`** | `src/runtime/egui/response.rs`<br>`src/runtime/focus/policy.rs`<br>`src/runtime/keys/route.rs`<br>`tests/it/runtime/activation.rs` | Focused widgets intercept `Space` as an activation key (`activate_on_space`), hijacking canvas Hand/Pan tool and firing menu items (e.g. File dropdown). | Configures `activate_on_space: false` on click-sensing roots; pointer default sets `ActivateKeys::Enter`; routes unopted Space to `PressRoute::Free`. | Fork baseline; PR #2239. |
| **`craft-ui-panels`** | `src/rails/flyout.rs`<br>`tests/it/focus/roving.rs` | Blanket Space suppression would disable keyboard activation of rail buttons. | Explicitly declares `const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_space_activation();` on `FlyoutButton` and `Button`. | Fork baseline; essential keyboard parity. |
| **`craft-pages`** | `src/ui/reading.rs`<br>`tests/it/views.rs` | Blanket Space suppression originally disabled keyboard column turning in `ReadingView`. | Explicitly configures `ActivateKeys::Both` on `ReadingView` and adds verified screen test. | Fork baseline; 112 screen tests passing. |
| **`craft-print`** | `src/ui/mod.rs` | Transient empty printer enumeration from system spooler clears the remembered/default printer setting. | Added non-empty slice guard before clearing remembered printer settings. | Fork baseline; 51 tests passing. |
| **`craft-font-manager`** | `src/picker/rank.rs`<br>`src/picker.rs`<br>`src/picker/rows.rs`<br>`tests/it/picker.rs` | Missing CJK classification (Mincho, Gothic, Maru Gothic, Kaisho, Pop); non-parity Japanese font names; missing Kana sample; classifier collisions (`WindSong` classified as Serif, `Pop Script` as Handwritten). Redundant classifier logic existed downstream in `photocraft/crates/text`. | Implements comprehensive CJK heuristics with strict precedence; foundry-prefixed Song checking; Photoshop Japanese name parity; Kana sample preview; consolidated `localized_families` fallback directly in `rank.rs`. | Fork baseline; 60 tests passing. |
| **`craft-psd`** | `src/merged.rs` | 8-bit PSD documents with empty primary layer info carry layer info in global `Layr` blocks; a zero layer count prematurely stopped scanning before reading `Mt16` transparency markers. | Evaluates negative count in global `Layr` block as merged transparency; allows zero layer count to fall through to global tagged blocks (`Mt16`, `Mt32`, `Mtrn`, `MTrn`). | Fork baseline; 189 tests passing. |
| **`craft-zip`** | `src/guard.rs` | Stubs or self-extracting prefixes exceeding 64KiB shift EOCD64 and central directory headers outside the fixed trailing buffer window, bypassing entry count guards. | Implements bounded 64KiB chunked scanning (`scan_offsets`) carrying 3 overlap bytes to discover EOCD64 records across arbitrary offsets. | Fork baseline; 45 tests passing. |

---

## 3. Deep Technical Analysis & Fix Specifications

### 3.1. AccessKit Dangling Controls Pruning (`craft-ui-contract`)

#### Problem Statement
In `accesskit_consumer`, the implementation of `Node::controls()` unconditionally unwraps every `NodeId` present in the node's relation list:
```rust
// accesskit_consumer / UIA adapter lookup
for id in node.controls() {
    let target = tree.node_by_id(id).unwrap(); // ABORTS if id not in tree update!
}
```
In PhotoCraft running on Windows with UI Automation enabled, clicking top-level menu bar entries (such as **Image**, **Edit**, **Layer**, or **Filter**) triggered an immediate application crash (`photocraft#15597`).

#### Root Cause Analysis
During menu expansion or transition:
1. `crates/ui-contract/src/runtime/access/emit.rs` registers the open popup's ID into the triggering button's `controls` relation.
2. In egui frame passes, menu state reporting and popup node rendering occur across distinct layout phases. A frame's `TreeUpdate` packet frequently carried a button node referencing a popup `NodeId` that was not yet emitted in `TreeUpdate.nodes`.
3. When the Windows UIA bridge inspected the updated tree, `Node::controls()` attempted to resolve the missing popup node, causing an uncatchable panic in the accessibility consumer.

#### Implementation
In `crates/ui-contract/src/runtime/egui/access.rs`:
```rust
pub(crate) fn drop_dangling_relations(output: &mut FullOutput) {
    let Some(update) = output.platform_output.accesskit_update.as_mut() else { return };
    prune_controls(update);
}

fn prune_controls(update: &mut egui::accesskit::TreeUpdate) {
    let ids: std::collections::HashSet<egui::accesskit::NodeId> = 
        update.nodes.iter().map(|(id, _)| *id).collect();
        
    for (_, node) in update.nodes.iter_mut() {
        if node.controls().iter().all(|id| ids.contains(id)) {
            continue;
        }
        let kept: Vec<egui::accesskit::NodeId> = 
            node.controls().iter().copied().filter(|id| ids.contains(id)).collect();
        node.set_controls(kept);
    }
}
```
In `crates/ui-contract/src/runtime/egui/plugin.rs`:
```rust
pub(crate) fn output_hook(output: &mut FullOutput) {
    access::drop_dangling_relations(output);
}
```
Adopted upstream in PR #2328.

---

### 3.2. Canvas Space-Key Hand Tool Preservation (`craft-ui-contract` & Downstream Crates)

#### Problem Statement
In canvas applications, holding `Space` temporarily engages the Hand (Pan) tool. In upstream `craft-libs`, any UI widget holding keyboard focus treated `Space` as an activation key (`ActivateKeys::Both`), causing `Space` over the canvas to activate previously focused buttons or the menu bar's File dropdown.

#### Implementation & Architecture

1. **Root Interaction Default (`policy.rs`)**:
   In `crates/ui-contract/src/runtime/focus/policy.rs`:
   ```rust
   Input::Pointer => (KeepKeys::None, ActivateKeys::Enter, OwnerClass::Widget),
   ```
   Pointer controls default to `ActivateKeys::Enter`.

2. **Framework Synthetic Click Suppression (`response.rs`)**:
   In `crates/ui-contract/src/runtime/egui/response.rs`:
   ```rust
   let options = egui::InteractOptions { activate_on_space: false, ..Default::default() };
   ```

3. **Key Route Dispatch (`route.rs`)**:
   In `crates/ui-contract/src/runtime/keys/route.rs`:
   ```rust
   None if press.key == Key::Space && plain && stop.keys.space_activates => PressRoute::Space,
   ```
   Unopted controls fall through to `PressRoute::Free`, keeping Space available for the canvas Hand tool.

4. **Preserved Opt-In for Buttons (`flyout.rs`, `button.rs`)**:
   In `crates/ui-panels/src/rails/flyout.rs`:
   ```rust
   impl<T: Value + 'static> Component for FlyoutButton<'_, T> {
       const ROLE: Role = Role::BUTTON;
       /// Space runs the face's command as a tap does: the focus has no held press, so it opens no flyout.
       const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_space_activation();
       const SHOWS_LABEL: bool = false;
   ```
   `Button` and `FlyoutButton` explicitly configure `.with_space_activation()` to ensure intentional keyboard button presses continue to fire on Space release.

---

### 3.3. Resilient Remembered Printer Retention (`craft-print`)

In `crates/print/src/ui/mod.rs`:
```rust
if !self.printers.is_empty() && !self.printers.iter().any(|p| p.name == self.settings.printer) {
    self.settings.printer =
        self.printers.iter().find(|p| p.default).or(self.printers.first()).map(|p| p.name.clone()).unwrap_or_default();
}
```
A non-empty guard ensures temporary spooler discovery silence does not wipe the user's remembered printer.

---

### 3.4. CJK Typographical Classification & Localization (`craft-font-manager`)

In `crates/font-manager/src/picker/rank.rs`:
- **Foundry-Guarded Song / Mincho Classification**: `is_cjk_song(word)` requires foundry prefixes (`st`, `fz`, `hg`, `hy`, `df`, `gb`, `sim`, `adobe`, `noto`) to prevent Latin script collisions (such as `WindSong`).
- **Classifier Precedence**: Evaluates Script markers before `Hei` / `Pop` / `Maru` so that `Pop Script` and `Xhei Script` correctly categorize as `FontClass::Script`.
- **Photoshop Japanese Name Parity**: `shown_name` lists Japanese font families in Japanese when in a Japanese locale, with consolidated `localized_families` fallback.
- **Kana Sample Preview**: Supplies canonical `const KANA_SAMPLE: &str = "\u{3042}\u{3044}\u{3046}\u{3048}\u{304A}";` ("あいうえお").

---

### 3.5. Merged PSD Layer Transparency & Layr Block Handling (`craft-psd`)

In `crates/psd/src/merged.rs`:
```rust
// The count's sign is the marker itself: negative means the first alpha channel holds the merged
// transparency. A count of zero has no layer records, so the global tagged blocks that follow can
// still carry one (Mt16/Mt32) and the scan below reaches them; a positive count would need every
// record parsed to find them, which this scan does not do.
let count = layer.get(at..at + 2).map(|b| i16::from_be_bytes([b[0], b[1]]));
if count.is_some_and(|count| count < 0) {
    return true;
}
if count.is_none_or(|count| count != 0) {
    return false;
}
...
// `Layr` is the same layer info in the global blocks (an 8-bit document whose primary layer info is
// empty), and its count is signed the same way.
if matches!(key, b"Lr16" | b"Lr32" | b"Layr") && layer.get(at..at + 2).is_some_and(|b| i16::from_be_bytes([b[0], b[1]]) < 0) {
    return true;
}
if matches!(key, b"Mtrn" | b"MTrn" | b"Mt16" | b"Mt32") {
    return true;
}
```
Ensures 8-bit documents with empty primary layer info correctly read merged transparency from `Layr` blocks, and documents with zero layer counts fall through to check `Mt16`/`Mt32`.

---

### 3.6. Large ZIP Archive EOCD64 Scanner & Zip-Bomb Protection (`craft-zip`)

In `crates/zip/src/guard.rs`:
```rust
fn scan_offsets<R: Read + Seek>(reader: &mut R, from: u64, to: u64, needle: &[u8; 4]) -> std::io::Result<Vec<u64>> {
    const CHUNK: u64 = 64 * 1024;
    let mut found = Vec::new();
    let mut at = from;
    let mut carry: Vec<u8> = Vec::new();
    while at < to {
        let len = usize::try_from(CHUNK.min(to - at)).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
        let mut buf = vec![0u8; len + carry.len()];
        buf[..carry.len()].copy_from_slice(&carry);
        reader.seek(SeekFrom::Start(at))?;
        reader.read_exact(&mut buf[carry.len()..])?;
        let base = at - carry.len() as u64;
        for (i, _) in buf.windows(4).enumerate().filter(|(_, window)| *window == needle) {
            found.push(base + i as u64);
        }
        let keep = buf.len().saturating_sub(3);
        carry = buf[keep..].to_vec();
        at += len as u64;
    }
    Ok(found)
}
```
Scans archives in bounded 64KiB chunks with 3-byte carry buffers across boundaries, ensuring that packages with self-extracting stubs or headers exceeding 64KiB reliably find `EOCD64` records and enforce entry limits.

---

## 4. Verification & Quality Assurance Suite

All changes are validated using the MSVC x64 developer toolchain.

### Test Execution Commands
```powershell
cmd.exe /c 'call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" && cargo test -p craft-ui-contract'
cmd.exe /c 'call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" && cargo test -p craft-ui-canvas'
cmd.exe /c 'call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" && cargo test -p craft-ui-widgets'
cmd.exe /c 'call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" && cargo test -p craft-ui-panels'
cmd.exe /c 'call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" && cargo test -p craft-font-manager'
cmd.exe /c 'call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" && cargo test -p craft-print'
cmd.exe /c 'call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" && cargo test -p craft-psd'
cmd.exe /c 'call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" && cargo test -p craft-zip'
```

### Measured Test Results
- `craft-ui-contract`: **538 passed; 0 failed**
- `craft-ui-canvas`: **810 passed; 0 failed**
- `craft-ui-widgets`: **814 passed; 0 failed**
- `craft-ui-panels`: **222 passed; 0 failed**
- `craft-font-manager`: **60 passed; 0 failed**
- `craft-print`: **51 passed; 0 failed**
- `craft-psd`: **189 passed; 0 failed**
- `craft-zip`: **45 passed; 0 failed**
- **Total: 2,729 passed; 0 failed** (Clippy: **0 warnings, exit code 0**)

---

## 5. Automated Synchronization & Fork Preservation Protocol

To guarantee that continuous synchronization with official upstream (`storytold/craft-libs:next`) never degrades, overwrites, or strips fork capabilities, automated integration tools (such as `photocraft_sync_mcp_server.py`) must enforce the following non-negotiable policies:

1. **`crates/zip` Non-Upstream Protection**:
   `crates/zip` is a customized fork crate carrying `scan_offsets` EOCD64 chunked scanning. It must NEVER be added to `upstream_only_prefixes` or wiped via `git checkout --theirs`.
2. **Keyboard Associated Constants Preservation**:
   In `crates/ui-panels/src/rails/flyout.rs` and `crates/ui-widgets/src/buttons/button.rs`, the associated constant:
   ```rust
   const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_space_activation();
   ```
   is mandatory. It ensures keyboard activation for rail buttons without sacrificing canvas Space-key Hand/Pan tool isolation. It must NEVER be removed when cleaning up obsolete UI hooks.
3. **Conflict Resolution Review, One Hunk at a Time**:
   A conflict hunk carrying a known fork identifier (`drop_dangling_relations`, `prune_controls`, `activate_on_space`, `with_space_activation`, `space_activates`, `ActivateKeys::Enter`, `PressRoute::Space`, `PressRoute::Free`, `!self.printers.is_empty()`, `KANA_SAMPLE`, `is_cjk_song`, `shown_name`, `localized_families`, `Layr`, `Mt16`, `Mt32`, `Mtrn`, `MTrn`, `scan_offsets`, `EOCD64`, `carry`) is a hunk to read, not one to resolve with `ours`. Each such hunk is resolved with both sides kept where they change different things, and with the fork's side only where the upstream change is already present or contradicts a fork rule. Resolving every matching hunk with `ours` silently drops the upstream fix that shares the hunk, and the merge then passes tests while the fix is gone.
4. **Pre-Push MSVC Verification Gate**:
   Prior to pushing synchronized commits to `fork:next` or retagging releases, all eight test suites above (2,729 tests) must execute and pass under MSVC x64 with 0 failures and 0 Clippy warnings.
