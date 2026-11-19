# Craft-Libs Fork Technical Specification & Upstream Delta Guide

> **Repository:** `ussoewwin/craft-libs` (Branch: `next`)  
> **Upstream Tracking:** `storytold/craft-libs` (Branch: `next`)  
> **Target Consumer:** PhotoCraft & Native Craft Applications  
> **Author:** ussoewwin  
> **Standard:** Strict Rust (Clean Room, `unsafe_code = "deny"`, Zero Panics / Zero Unwraps in Non-Test Code)  

---

## 1. Overview & Architectural Scope

This specification provides the comprehensive technical documentation for the `ussoewwin/craft-libs` fork. `craft-libs` is the shared Rust foundation powering the Craft application suite (including PhotoCraft, ArtCraft, DesignCraft, and VectorCraft).

The fork maintains strict synchronization with official upstream (`storytold/craft-libs:next`) while introducing critical architectural fixes, host stability improvements, and Photoshop 1:1 behavioral parity enhancements:

1. **Host Accessibility Crash Prevention (`craft-ui-contract`)**: Eliminates unwrap panics in Windows UI Automation (UIA) adapters triggered by transient dangling relation references in popup/menu transitions.
2. **Canvas Space-Key Hand Tool Preservation (`craft-ui-contract`)**: Resolves keyboard event hijacking where pressing `Space` inadvertently activated previously focused widgets instead of delegating to the canvas Hand/Pan tool.
3. **Resilient Peripheral Retention (`craft-print`)**: Prevents erasure of remembered printer configurations caused by transient spooler polling delays.
4. **Comprehensive East Asian Typographical Classification & Localization (`craft-font-manager`)**: Introduces full CJK font classification heuristics, Photoshop-parity Japanese font name localization, and canonical Kana sample previews.
5. **Title Bar Leading Width & Menu Bar Sizing (`craft-ui-shell`, `craft-menu`, `craft-ui-widgets`)**: Enables the menu bar in integrated title bars to declare its titles width, preventing Window and Help menus from falling into the overflow menu while avoiding unwanted empty gaps before the bar.
6. **Content-Sized Container Inline Header Sizing (`craft-ui-contract`)**: Enforces finite positive inline header width reservation in `wanted_width`, preventing child body layout collapse in content-sized containers.

All modifications conform to workspace layering rules, compile with zero warnings under `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`, and pass all test suites across affected crates.

---

## 2. Upstream Divergence Matrix

| Crate | Modified Files | Core Issue in Upstream | Fork Technical Resolution | Status / Upstream Alignment |
|---|---|---|---|---|
| **`craft-ui-contract`** | `src/runtime/egui/access.rs`<br>`src/runtime/egui/plugin.rs` | UIA adapter crashes with abort on Windows when `Node::controls()` unwraps IDs missing from current update (Issue `photocraft#15597`). | Prunes dangling control relation IDs in `output_hook` so only nodes carried in the current update packet are referenced. | Adopted upstream via PR #2328 / #2239. |
| **`craft-ui-contract`** | `src/runtime/egui/response.rs`<br>`src/runtime/focus/policy.rs`<br>`src/runtime/keys/route.rs`<br>`tests/it/runtime/activation.rs` | Focused widgets intercept `Space` as an activation key (`activate_on_space`), hijacking canvas Hand/Pan tool and firing menu items (e.g. File dropdown). | Configures `activate_on_space: false` on click-sensing roots; pointer default sets `ActivateKeys::Enter`; preserves guarded opt-in for widgets that explicitly require Space. | Fork baseline; regression test in `activation.rs`. |
| **`craft-pages`** | `src/ui/reading.rs`<br>`tests/it/views.rs` | Blanket Space suppression originally disabled keyboard column turning in `ReadingView`. | Explicitly configures `ActivateKeys::Both` on `ReadingView` and adds verified screen test. | Fork baseline; 112 screen tests passing. |
| **`craft-print`** | `src/ui/mod.rs` | Transient empty printer enumeration from system spooler clears the remembered/default printer setting. | Added non-empty slice guard before clearing remembered printer settings. | Fork baseline; 43 tests passing. |
| **`craft-font-manager`** | `src/picker/rank.rs`<br>`src/picker.rs`<br>`src/picker/rows.rs`<br>`tests/it/picker.rs` | Missing CJK classification (Mincho, Gothic, Maru Gothic, Kaisho, Pop); non-parity Japanese font names; missing Kana sample; classifier collisions (`WindSong` classified as Serif, `Pop Script` as Handwritten). | Implements comprehensive CJK heuristics with strict precedence; foundry-prefixed Song checking; Photoshop Japanese name parity; Kana sample preview; zero-cost testing without private `FaceInfo` literals. | Fork baseline; 54 tests passing. |
| **`craft-ui-shell`** | `src/shell/title_bar/bar.rs`<br>`tests/it/shell/menu_bar_room.rs` | Integrated title bar's leading region previously applied declared width to the caption header (`props.header`), creating an unwanted 380pt empty region before the menu bar on Windows. | Directs `leading_width` strictly to the leading content via `allocate_ui_with_layout`, keeping the caption header at its natural measured width (0pt on Windows) with zero leading offset. | Fork baseline; 94 tests passing. |
| **`craft-ui-widgets`** & **`craft-menu`** | `craft-ui-widgets/src/menu/bar.rs`<br>`craft-menu/src/bar.rs` | Menu bar filled entire row by default, squeezing trailing titles into overflow menu when embedded beside a title strip. | Added `MenuBar::own_width` / `MenuBarUi::width` to lay out titles at declared width and report `titles_width`, allowing host to preserve full menu rows without overflow. | Fork baseline; 3 tests passing. |
| **`craft-ui-contract`** | `src/contract/body/size.rs`<br>`src/contract/body/regions.rs` | `wanted_width` did not count finite positive declared inline header widths, risking child body collapse in content-sized containers, and did not match `regions.rs` zero-width fallback. | Incorporates finite positive declared inline widths (`width.is_finite() && *width > 0.0`) in `wanted_width` while treating zero as no declaration (falling back to measured header). | Fork baseline; 269 tests passing. |

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

In PhotoCraft running on Windows with UI Automation enabled, clicking top-level menu bar entries (such as **Image**, **Edit**, **Layer**, or **Filter**) triggered an immediate application crash (tracked in `photocraft#15597`). 

#### Root Cause Analysis
During menu expansion or transition:
1. `crates/ui-contract/src/runtime/access/emit.rs` registers the open popup's ID into the triggering button's `controls` relation.
2. In egui frame passes, menu state reporting and popup node rendering can occur across distinct layout phases. Consequently, a frame's `TreeUpdate` packet frequently carried a button node referencing a popup `NodeId` that was not yet emitted or included in `TreeUpdate.nodes`.
3. When the Windows UIA bridge inspected the updated tree, `Node::controls()` attempted to resolve the missing popup node, causing an uncatchable panic in the accessibility consumer.

#### Implementation & Architecture
The fix intercepts the outgoing accessibility packet in `output_hook` before dispatch to the platform accessibility adapter.

In `crates/ui-contract/src/runtime/egui/access.rs`:

```rust
/// Drops relations that name a node this update does not carry.
///
/// A menu reports its popup open on a frame where the popup's own node is not emitted (the ordering
/// differs per menu), and `runtime::access::emit` puts that id in `controls`. A host's UIA adapter
/// unwraps every `controls` lookup (`accesskit_consumer`'s `Node::controls`), so one id with no node
/// in the same update aborts the app: photocraft#15597.
pub(crate) fn drop_dangling_relations(output: &mut FullOutput) {
    let Some(update) = output.platform_output.accesskit_update.as_mut() else { return };
    prune_controls(update);
}

/// Prunes every `controls` id of `update` that names no node carried in the same update.
/// Other relations keep their ids: they are built from nodes this pass wrote.
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

#### Verification & Tests
- `controls_keep_the_ids_the_update_carries_and_drop_the_rest`: Verifies that IDs referencing omitted nodes are stripped while valid IDs are preserved.
- `controls_with_every_node_carried_are_left_alone`: Verifies zero allocation/mutation overhead when all IDs are present.

---

### 3.2. Canvas Space-Key Hand Tool Preservation (`craft-ui-contract` & Downstream Crates)

#### Problem Statement
In digital painting and photo manipulation software (Photoshop, PhotoCraft), holding the `Space` key temporarily activates the Hand (Pan) tool regardless of document or panel context.

In upstream `craft-libs`, any UI widget that held keyboard focus (such as a toolbar tool button, an adjustment slider, or the menu bar's "File" button) treated `Space` as an activation trigger. Clicking a tool and subsequently pressing `Space` over the canvas would activate the last-focused widget instead of panning the viewport. In PhotoCraft, this caused pressing `Space` to open the File menu and execute "New Document".

#### Root Cause Analysis
1. **Root Interaction Defaults**: In `crates/ui-contract/src/runtime/egui/response.rs`, root widgets queried egui's default `ui.interact(rect, id, sense)`, where egui defaults `InteractOptions.activate_on_space = true`.
2. **Pointer Focus Policy**: In `crates/ui-contract/src/runtime/focus/policy.rs`, `KeyUse::of(Input::Pointer)` configured `ActivateKeys::Both` (both Space and Enter).
3. **Key Routing**: In `crates/ui-contract/src/runtime/keys/route.rs`, key presses were routed to `PressRoute::Activate` unconditionally when matching `ActivateKeys::Both`.

#### Implementation & Architecture
The fork establishes a clear separation between keyboard navigation (`Enter`) and canvas panning (`Space`):

1. **Disable `activate_on_space` on Root Widgets**:
   In `crates/ui-contract/src/runtime/egui/response.rs`:
   ```rust
   pub(crate) fn widget(ui: &Ui, id: Id, rect: Rect, sense: Sense) -> Response {
       let ctx = ui.ctx();
       let held = ctx.memory(|memory| memory.has_focus(id));
       // Enter activates the focused widget; Space does not. egui makes Space the keyboard's activation of
       // whatever widget holds the focus, and it drops that focus as it does, so a canvas's Hand key would press
       // a toolbar button or a panel row that was clicked last instead of taking the key.
       let options = egui::InteractOptions { activate_on_space: false, ..Default::default() };
       let response = ui.interact_opt(rect, id, to_egui(sense), options);
       ...
   }
   ```

2. **Pointer Control Key Policy**:
   In `crates/ui-contract/src/runtime/focus/policy.rs`:
   ```rust
   impl KeyUse {
       pub const fn of(input: Input) -> KeyUse {
           let (keep, activate, owner) = match input {
               // Enter activates a pointer control; Space does not. Space is the canvas's Hand key.
               Input::Pointer => (KeepKeys::None, ActivateKeys::Enter, OwnerClass::Widget),
               Input::Text => (KeepKeys::InlineArrows, ActivateKeys::Neither, OwnerClass::Text),
               Input::Editor => (KeepKeys::BlockArrows, ActivateKeys::Neither, OwnerClass::Text),
               _ => (KeepKeys::None, ActivateKeys::Neither, OwnerClass::Widget),
           };
           KeyUse { keep, activate, owner }
       }
   }
   ```

3. **Guarded Key Route with Space Passthrough**:
   In `crates/ui-contract/src/runtime/keys/route.rs`:
   ```rust
   // Space activates only a stop that asks for it with `ActivateKeys::Space` or `ActivateKeys::Both`
   // (e.g. a reading view turning a column on Space). `ActivateKeys::Space` enables Space without also
   // enabling Enter, while `ActivateKeys::Both` enables both. Other stops do not opt into Space (Pointer
   // controls default to Enter, while Text and Editor controls default to Neither), so Space stays the
   // canvas's Hand key where the app has a canvas.
   None if press.key == Key::Space && plain && stop.keys.activate.space() => PressRoute::Activate,
   None if press.key == Key::Space => PressRoute::Free,
   ```
   Because `ActivateKeys::space()` returns `true` for both `ActivateKeys::Space` and `ActivateKeys::Both`, a component can opt in to `Space` activation either alongside `Enter` (`ActivateKeys::Both`) or exclusively without enabling `Enter` (`ActivateKeys::Space`). Stops that do not opt into Space (pointer stops default to `ActivateKeys::Enter`, while `Input::Text` and `Input::Editor` default to `ActivateKeys::Neither`) return `PressRoute::Free`, so the activation route does not take `Space`. A focused `Input::Text` or `Input::Editor` field still receives the typed space character. When the canvas holds `Space` (`canvas_space`), the canvas keeps `Space` from every field and `Space` acts as the Hand tool shortcut.

4. **Preserved Opt-in for Reading View**:
   In `crates/pages/src/ui/reading.rs`:
   ```rust
   // ReadingView explicitly declares ActivateKeys::Both to allow turning columns on Space
   let keys = KeyUse { activate: ActivateKeys::Both, ..KeyUse::of(Input::Pointer) };
   ```

#### Verification & Tests
- `crates/ui-contract/tests/it/runtime/activation.rs`: `space_does_not_activate_a_focused_widget_and_enter_does` asserts that pressing Space on a focused button emits no activation event, while Enter triggers activation.
- `crates/pages/tests/it/views.rs`: `a_reading_view_turns_on_space_and_enter` asserts that `ReadingView` turns exactly one column on Space and one column on Enter.

---

### 3.3. Resilient Remembered Printer Retention (`craft-print`)

#### Problem Statement
When opening the print setup dialog in `craft-print`, if the operating system print spooler took longer than a single frame to enumerate installed printers, or if a network printer experienced discovery latency, `craft-print` wiped out the user's previously configured printer selection.

#### Root Cause Analysis
In `crates/print/src/ui/mod.rs`:
```rust
// Upstream logic:
if !self.printers.iter().any(|p| p.name == self.settings.printer) {
    self.settings.printer = self.printers.iter().find(|p| p.default)...unwrap_or_default();
}
```
When `self.printers` is an empty slice `[]`, `iter().any(...)` evaluates to `false`. Consequently, `self.settings.printer` was overwritten with `String::default()` (`""`).

#### Implementation & Architecture
The fork introduces a non-empty precondition:
```rust
// In crates/print/src/ui/mod.rs:
// An empty list says the spooler could not be asked just now (it is busy, or the system
// gave nothing back), not that the remembered printer is gone: keep it and let the panel
// say there are none. Only a list that does not contain it says it went away.
if !self.printers.is_empty() && !self.printers.iter().any(|p| p.name == self.settings.printer) {
    self.settings.printer =
        self.printers.iter().find(|p| p.default).or(self.printers.first()).map(|p| p.name.clone()).unwrap_or_default();
}
```

---

### 3.4. CJK Typographical Classification & Localization (`craft-font-manager`)

#### Problem Statement
In upstream `craft-font-manager`, font classification was restricted to Latin-centric PANOSE and IBM Family Class heuristics. This produced severe parity gaps:
1. **Unclassified East Asian Fonts**: Common Japanese (Mincho, Gothic, Maru Gothic, Kyokasho), Chinese (Song, Hei, FangSong, Kaiti), and Korean (Myeongjo, Dotum, Gulim) fonts had no class representation and could not be filtered in the font picker.
2. **Japanese Font Name Parity with Photoshop**: In Adobe Photoshop, when running in a Japanese environment, Japanese font families are always displayed using their native Japanese names (e.g. "ＭＳ ゴシック", "小塚明朝 Pr6N", "ヒラギノ角ゴ ProN", "游ゴシック") even if the user has enabled the "Show Font Names in English" preference. Upstream `shown_name` unconditionally forced English family names.
3. **Kana Preview Sample**: Upstream font sample generator only provided Latin and Arabic preview strings. Japanese fonts with no Latin coverage displayed empty preview rectangles.
4. **Classifier Substring Collisions**:
   - `ends_with("song")` matched the Latin cursive script font `WindSong`, incorrectly classifying it as `FontClass::Serif`.
   - `pop` (Japanese pop/casual display) matched before `Script`, misclassifying `Pop Script` as `FontClass::Handwritten`.
   - `hei` (Chinese sans-serif) matched before `Script`, misclassifying `Xhei Script` as `FontClass::SansSerif`.
5. **Test Inaccessibility**: Upstream tests attempted to construct `FaceInfo` literals directly; however, `FaceInfo.blob` is `pub(crate)` and not accessible to integration tests, breaking test compilation.

#### Implementation & Architecture

1. **Foundry-Guarded Song / Serif Classification**:
   In `crates/font-manager/src/picker/rank.rs`:
   ```rust
   fn is_cjk_song(word: &str) -> bool {
       const FOUNDRIES: [&str; 9] = ["st", "fz", "hg", "hy", "df", "gb", "sim", "adobe", "noto"];
       ["song", "songti", "sungti"].iter().any(|ending| word.len() > ending.len() && word.ends_with(ending))
           && FOUNDRIES.iter().any(|prefix| word.starts_with(prefix))
   }
   ```
   Ensures that words like `WindSong` do not trigger CJK Song serif classification.

2. **Strict Classifier Precedence Hierarchy**:
   The `classify` pipeline (`crates/font-manager/src/picker/rank.rs`) evaluates font table metadata first, and then falls back to family-name heuristics in a strict `if / else if` precedence order:
   - **Stage A — Font Table Metadata & Early Checks** (evaluated before family-name rules):
     1. **Fixed-Pitch & PANOSE Proportion**: `fixed_pitch || (panose[0] == 2 && panose[3] == 9)` → `FontClass::Monospace`.
     2. **Blackletter Fragments**: `"fraktur"`, `"blackletter"`, `"black letter"`, `"textura"`, `"old english"`, `"schwabacher"`, `"rotunda"` → `FontClass::Blackletter`.
     3. **PANOSE Family Kind (`panose[0]`)**: `3` → `FontClass::Script` when `family_class >> 8 == 10`, else `FontClass::Handwritten`; `4 | 5` → `FontClass::Decorative`.
     4. **OS/2 IBM Family Class (`family_class >> 8`)**: `1..=4 | 7` → `FontClass::Serif`; `5` → `FontClass::SlabSerif`; `8` → `FontClass::SansSerif`; `9 | 12` → `FontClass::Decorative`; `10` → `FontClass::Handwritten` if the name contains `"hand"`, `"marker"`, `"brush"`, `"pen"`, or `"chalk"`, else `FontClass::Script`.
     5. **PANOSE Latin Text Serif Style (`panose[0] == 2`, `panose[1]`)**: `11..=15` → `FontClass::SansSerif`; `6` → `FontClass::SlabSerif`; `2..=10` → `FontClass::Serif`.
   - **Stage B — Family-Name Fallback Rules** (evaluated when metadata does not classify the face; `Sans-Serif / Gothic` precedes `Serif / Mincho`, so a name containing both `Gothic` and `Mincho` resolves to `FontClass::SansSerif`):
     1. **Monospace**: whole words matching `"mono"`, `"monospace"`, `"monospaced"`, `"code"`, `"courier"`, `"等幅"`, words starting with `"consol"`, or substring `"等幅"` → `FontClass::Monospace`.
     2. **Slab Serif**: substring `"slab"` → `FontClass::SlabSerif`.
     3. **Sans-Serif / Gothic**: substrings `"sans"`, `"grotesk"`, `"grotesque"`, `"gothic"`, `"ゴシック"`, `"ｺﾞｼｯｸ"`, `"meiryo"`, `"メイリオ"`, `"黑体"`, `"黑體"`, `"丸ゴシック"` → `FontClass::SansSerif`.
     4. **Serif / Mincho & Song**: substrings `"serif"`, `"roman"`, `"garamond"`, `"times"`, `"bodoni"`, `"baskerville"`, `"caslon"`, `"mincho"`, `"明朝"`, `"宋体"`, `"宋體"`, or tokenized words matching `"sung"`, `"song"`, `"sungti"`, `"songti"`, or `is_cjk_song(w)` → `FontClass::Serif`.
     5. **Handwritten / Kyokasho & Pop**: substrings `"hand"`, `"marker"`, `"brush"`, `"chalk"`, `"kyokasho"`, `"教科書"`, `"ポップ"`, `"ﾎﾟｯﾌﾟ"`, suffix `"poptai"`, or tokenized word `"poptai"` → `FontClass::Handwritten`.
     6. **Calligraphic / Script**: substrings `"script"`, `"gyosho"`, `"行書"`, `"kaisho"`, `"楷書"`, `"sousho"`, `"草書"`, `"reisho"`, `"隷書"`, `"kanteiryu"`, `"勘亭流"` → `FontClass::Script`.
     7. **Hei (Sans-Serif)**: tokenized word `"hei"` or `"heiti"`, or word ending with `"hei"` or `"heiti"` (evaluated *after* Script markers so `Xhei Script` resolves to `Script`, while `SimHei` resolves to `SansSerif`) → `FontClass::SansSerif`.
     8. **Pop (Handwritten)**: tokenized word `"pop"` (evaluated *after* Script markers so `Pop Script` resolves to `Script`, while `HG創英角ﾎﾟｯﾌﾟ体` resolves to `Handwritten`) → `FontClass::Handwritten`.
     9. **Maru (Rounded Sans-Serif)**: tokenized word `"maru"` (evaluated *after* Handwritten and Script markers so `Maru Script` and `Maru Hand` keep their own class, while standalone `Maru` resolves to `SansSerif`) → `FontClass::SansSerif`.

3. **Photoshop Japanese Name Parity**:
   ```rust
   pub(super) fn shown_name(localized: &[LocalizedName], family: &str, language: &str, english: bool) -> String {
       let primary = |tag: &str| tag.split(['-', '_']).next().unwrap_or(tag).to_ascii_lowercase();
       let is_ja = primary(language) == "ja";
       // In a Japanese interface, Photoshop always lists Japanese font names in Japanese,
       // regardless of the English font names preference. In other languages, English names preference decides.
       if (!is_ja && english) || primary(language) == "en" {
           return family.to_owned();
       }
       let exact = |n: &&LocalizedName| n.language.as_deref().is_some_and(|l| l.replace('_', "-").eq_ignore_ascii_case(&language.replace('_', "-")));
       let same_language = |n: &&LocalizedName| n.language.as_deref().is_some_and(|l| primary(l) == primary(language));
       localized.iter().find(exact).or_else(|| localized.iter().find(same_language)).map_or_else(|| family.to_owned(), |n| n.name.clone())
   }
   ```

4. **Kana Sample Generation**:
   Defines `const KANA_SAMPLE: &str = "\u{3042}\u{3044}\u{3046}\u{3048}\u{304A}";` ("あいうえお"). If the font charmap draws these canonical vowels, the picker automatically supplies the Kana preview.

5. **Clean Room Testing Refactor**:
   `shown_name` accepts `&[LocalizedName]` rather than `&Info`, enabling integration tests to verify localization behavior without fabricating internal `FaceInfo` structs.

#### Verification & Tests
- `classifies_japanese_and_cjk_families`: Validates correct classification for `MS Mincho`, `ＭＳ 明朝`, `Yu Gothic`, `游ゴシック`, `Meiryo`, `メイリオ`, `HG行書体`, `HG正楷書体-PRO`, `UD デジタル 教科書体`, `HG創英角ﾎﾟｯﾌﾟ体`, and `MS Gothic 等幅`.
- `a_japanese_interface_lists_japanese_family_names`: Verifies that Japanese UI displays "ＭＳ ゴシック" regardless of `english` flag, while English/French UI respects the setting.
- `a_kana_sample_needs_every_canonical_vowel`: Verifies strict charmap inspection for Kana previews.

---

### 3.5. Title Bar Leading Content Width & Menu Bar Sizing (`craft-ui-shell`, `craft-menu`, `craft-ui-widgets`)

#### Problem Statement
In an integrated window chrome layout (such as PhotoCraft on Windows), the menu bar sits in the leading slot of the `TitleBar`. Originally:
1. `MenuBar` declared `fills = Some(Axis::X)`, expanding to fill the entire horizontal line and squeezing subsequent titles (`Window`, `Help`) into the overflow dropdown.
2. An initial attempt to declare `inline_width` on `TitleBar::leading_width` inadvertently set `self.props.header(header.inline_width(width))`. Because `props.header` corresponds to the start caption buttons (e.g. macOS traffic lights, which are empty on Windows), this created a 380pt empty region before the menu bar, displacing the File menu 380pt to the right.

#### Implementation & Architecture
1. **Targeted Leading Allocation in `TitleBar`**:
   `TitleBar::leading_width` was decoupled from `self.props.header`, allowing the caption header to retain its natural measured width (0pt on Windows, ~70pt on macOS). In `show_titled`, the declared `leading_width` is directly applied to the leading content closure:
   ```rust
   // crates/ui-shell/src/shell/title_bar/bar.rs
   pub fn show_titled(...) -> TitleBarOutput {
       ...
       self.trailing(trailing).show(ui, |ui, cx| {
           if let Some(width) = leading_width {
               ui.allocate_ui_with_layout(
                   egui::vec2(width, ui.available_height()),
                   *ui.layout(),
                   |ui| leading(ui, cx),
               );
           } else {
               leading(ui, cx);
           }
           ...
       })
   }
   ```
2. **Explicit MenuBar Width & Natural Width Reporting**:
   `MenuBar::own_width` configures `MenuBar` to lay out titles without filling the entire row (`fills` returns `None`). It records the natural unconstrained width of all titles in `MenuBarOutput::titles_width`, allowing host applications to read back the measured width and declare it on the subsequent frame.

#### Verification & Tests
- `crates/ui-shell/tests/it/shell/menu_bar_room.rs`: `a_declared_leading_width_is_the_menu_bars_room` asserts that `offset < 1.0` (zero empty spacing before the bar) and `narrow.width() <= 380.0 + 1.0` (bar strictly adheres to declared width).

---

### 3.6. Content-Sized Inline Container Header Reservation (`craft-ui-contract`)

#### Problem Statement
In `crates/ui-contract/src/contract/body/size.rs`, `BodyHooks::wanted_width` calculates the horizontal extent a content-sized container requests from its parent. Previously:
1. `wanted_width` only inspected `self.measured_header`. When an inline header declared an `inline_width` larger than its measured content (e.g., an empty inline header with 100pt declared width), `wanted_width` reported only the smaller measured extent. `shared_width` subsequently capped the container at that extent, causing the header to consume the full container and starving the body of width.
2. When `0.0` was passed as `inline_width`, `wanted_width` previously treated `0.0` as a valid declaration, whereas `regions.rs` treated `0.0` as "no declaration" (falling back to measured header width), creating a layout inconsistency.

#### Implementation & Architecture
`wanted_width` incorporates the finite positive inline header declaration rule matching `regions.rs`:
```rust
// crates/ui-contract/src/contract/body/size.rs
pub(super) fn wanted_width(&self, props: &ContainerProps) -> Option<f32> {
    self.measured_body.map(|body| {
        let (mut header, end) = (self.measured_header.unwrap_or(Vec2::ZERO), self.measured_end.unwrap_or(Vec2::ZERO));
        // A finite positive declared inline width is the header's reserved width, matching regions.rs.
        if let Some(declared) = props.header.filter(|header| header.inline).and_then(|header| header.inline_width).filter(|width| width.is_finite() && *width > 0.0) {
            header.x = declared;
        }
        let wanted = self.regions_size(props, body, header, self.measured_footer, end).x.max(self.least_width);
        overlay_of(props).map_or(wanted, |overlay| wanted.max(crate::contract::overlay::anchor_floor(overlay)))
    })
}
```

#### Verification & Tests
- Verified with `cargo test -p craft-ui-contract`: all 269 tests passing with zero regressions.

---

## 4. Verification & Quality Assurance Suite

All changes in the fork are validated using the MSVC x64 developer toolchain.

### Test Execution Commands
```powershell
# In D:\USERFILES\GitHub\craft-libs:
cmd.exe /c 'call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" && cargo test -p craft-font-manager'
cmd.exe /c 'call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" && cargo test -p craft-print'
cmd.exe /c 'call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" && cargo test -p craft-ui-contract'
cmd.exe /c 'call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" && cargo test -p craft-ui-shell'
cmd.exe /c 'call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" && cargo test -p craft-menu'
cmd.exe /c 'call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" && cargo test -p craft-pages --features screen-testing'
```

### Measured Test Results
- `craft-font-manager`: **54 passed; 0 failed**
- `craft-print`: **43 passed; 0 failed**
- `craft-ui-contract`: **269 passed; 0 failed** (plus 1 doc-test passed)
- `craft-ui-shell`: **94 passed; 0 failed** (plus 1 doc-test ignored)
- `craft-menu`: **3 passed; 0 failed** (plus 1 doc-test ignored)
- `craft-pages` (screen testing): **112 passed; 0 failed**
- Clippy across all workspace crates: **0 warnings, exit code 0**

---

## 5. Maintenance & Upstream Synchronization Protocol

To merge upstream updates while preserving all fork-specific architectural features:

0. **Verify Remotes**:
   ```bash
   git remote add upstream https://github.com/storytold/craft-libs.git   # skip if it already exists
   git remote add fork https://github.com/ussoewwin/craft-libs.git      # skip if it already exists
   git remote -v
   ```
   `upstream` must point to `storytold/craft-libs`, and `fork` must point to `ussoewwin/craft-libs`.
1. **Fetch, Switch & Backup**:
   ```bash
   git fetch upstream next
   git switch next
   git branch backup-next-$(date +%Y%m%d)
   ```
2. **Merge Upstream**:
   ```bash
   git merge upstream/next -m "Merge remote-tracking branch 'upstream/next' into next (preserve fork features)"
   ```
3. **Execute Verification Gate**:
   Execute all six test commands listed in Section 4. Ensure 0 failures and 0 warnings.
4. **Push to Fork Remote**:
   ```bash
   git push fork next
   ```
