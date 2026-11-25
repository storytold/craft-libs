# The upstream code behind the fork's answers in craft-libs

For each adaptation the merge had to make: the upstream lines that forced
it, what they do, why they broke the fork, and the fork's answer.

Each entry names the upstream file and the lines the merge had to answer. The lines are
quoted from `6fcf61c81` as it was merged, with their line numbers.

## 1. The activation harness walks every activation key

**The upstream code.**

`crates/ui-contract/src/testing/suite/activation.rs`, lines 78-79:

```rust
   78 |     let keys = [(Key::Enter, subject.probe.keys.activate.enter()), (Key::Space, subject.probe.keys.activate.space())];
   79 |     for story in &subject.stories {
```

`crates/ui-contract/src/testing/suite/activation.rs`, lines 91-98:

```rust
   91 |             // Paths: 0 click, 1 AccessKit Click, 2 and 3 the activation keys.
   92 |             let paths = [
   93 |                 (0, "a click", true),
   94 |                 (1, "AccessKit Click", declares(Declared::Click)),
   95 |                 (2, "Enter", keys[0].1 && declares(Declared::Focus)),
   96 |                 (3, "Space", keys[1].1 && declares(Declared::Focus)),
   97 |             ];
   98 |             // The paths of a part share one fixture while each leaves the component as it found it ([`left_as_found`]).
```

**What it does.** Upstream's activation harness treats Space as an activation key: it reads the activation keys as a pair, and it checks a fourth path (a Space press) for every part that senses a click.

**Why it broke the fork.** The fork's rule is that a bare Space is the canvas's Hand key and only a control that asks for it takes Space. A Space path in the harness would ask every control in the application to answer Space, and the canvas's own Space would go to the controls.

**The fork's answer.** The fork keeps Enter alone in the harness's `keys` array and drops the Space path, so the path list has three entries. Upstream's new loop around it - the filter that drops a disabled part, the sampling of repeated parts and the fixture the paths share - is taken as it is.

## 2. The start-screen story names icons the catalog does not carry

**The upstream code.**

`crates/ui-shell/src/shell/start/stories.rs`, lines 212-219:

```rust
  212 | fn column_links() -> [StartLink; 3] {
  213 |     [
  214 |         StartLink::url("Join the community", Some(IconId::new("message-circle")), "https://example.com/chat").primary(),
  215 |         StartLink::url("Website", Some(IconId::new("globe")), "https://example.com"),
  216 |         StartLink::url("Source code", Some(IconId::new("git-branch")), "https://example.com/source"),
  217 |     ]
  218 | }
  219 | 
```

**What it does.** Upstream's column story gives the community link the icon `message-circle` and the source link the icon `git-branch`.

**Why it broke the fork.** The built catalog holds neither name, so the icon draws nothing, and the workspace's icon-id check fails on the merged tree. The check fails on upstream's own tree as well (measured on `6fcf61c81`, file byte-identical), so this is upstream's own gap and not a merge error.

**The fork's answer.** The story names `link` and `open`, which the catalog's built manifest holds. The names are read from the generated manifest, whose ids are underscored.

## 3. The components index is generated from the registrations

**The upstream code.**

`docs/components-index.md`, lines 67-67:

```text
   67 | | `Link` | Leaf | `craft_ui_widgets::buttons::link` | A link: drawn in the accent colour in a type role (body by default) and underlined, so colour is not its only cue |
```

**What it does.** Upstream's committed components index describes the `Link` component without the note that a link standing alone is not underlined when `LinkLook::Plain` says so.

**Why it broke the fork.** The index is generated from the registrations, and the merged registrations carry that note, so the committed copy was stale and its check failed.

**The fork's answer.** The file is regenerated with `cargo run -p craft-ui-gallery --bin components-index`; the committed copy is the generated one and the check passes.

## 4. The page picture's paper and its document dimensions break two design rules

**The upstream code.**

`crates/ui-widgets/src/tile_grid/tile.rs`, lines 308-308:

```rust
  308 | const PAPER: DataColor = DataColor::new(Color32::WHITE);
```

`crates/ui-widgets/src/tile_grid/tile.rs`, lines 557-560:

```rust
  557 |     pub fn page(self, width: f64, height: f64) -> Self {
  558 |         self.flat_thumbnail().thumb(ThumbAspect::new(2, 1), move |cx: &ContentPainterCx<'_>, picture: Rect| paint_page(cx, picture, width, height))
  559 |     }
  560 | 
```

**What it does.** Upstream's page picture uses a raw white for the paper and takes the document's width and height as plain numbers.

**Why it broke the fork.** The design lint reads every mapped crate: a raw colour constant must come from a semantic token, and a size in points must come from a token step, an aspect ratio or the content. The page's paper is document data and its width and height are the document's own dimensions.

**The fork's answer.** Both sites carry a `design: allow` marker, and the exceptions file of the crate registers both markers with the number of sites each covers.

## 5. The iconic trial compares measured rects with a half point

**The upstream code.**

`crates/ui-panels/src/docking/iconic/trials.rs`, lines 703-724:

```rust
  703 | fn icon_groups_apart_by_a_rule_show_one_level_rule_between_them() {
  704 |     let (gripped, gripped_id, _) = separated(IconGroupSeparator::Grip, RAIL);
  705 |     let gripped_first = gripped.part_rect(gripped_id, CELL.at(0)).expect("the first icon under grips");
  706 |     let (mut fixture, id, _) = separated(IconGroupSeparator::Rule, RAIL);
  707 |     let border = StyleCx::with_root(fixture.ctx(), |cx| cx.tokens.color.line.border);
  708 |     let shapes = fixture.frame_shapes();
  709 |     let level: Vec<(Pos2, Pos2, egui::Color32)> = shapes
  710 |         .iter()
  711 |         .filter_map(|shape| match shape {
  712 |             egui::Shape::LineSegment { points: [a, b], stroke } if (a.y - b.y).abs() < EPS => Some((*a, *b, stroke.color)),
  713 |             _ => None,
  714 |         })
  715 |         .collect();
  716 |     let cell = |at: u64| fixture.part_rect(id, CELL.at(at)).expect("the icon shows");
  717 |     let band = |group: u64| fixture.part_rect(id, GRIP.at(group)).expect("each group keeps a grip band");
  718 |     let (first, last, next) = (cell(0), cell(1), cell(2));
  719 |     let (top, lower) = (band(0), band(1));
  720 |     assert!(first.top() < gripped_first.top() - 0.5, "the first icon {first:?} stands higher than under a grip strip {gripped_first:?}");
  721 |     for (group, band) in [top, lower].into_iter().enumerate() {
  722 |         assert!(grip_ticks(&shapes, band).is_none(), "group {group}: the band shows no marks");
  723 |     }
  724 |     assert!(
```

**What it does.** Upstream's trial asserts with the literal 0.5 that one icon stands higher than the one under a grip strip, and that the rule between two groups lies in the middle of the room.

**Why it broke the fork.** The design lint names a literal size in a trial that shows a component; a rounding tolerance between two measured rects is not a size a token should carry.

**The fork's answer.** The two assertions carry a `design: allow[test/literal-size]` marker, and one entry in the exceptions file of the crate lists both sites.

## 6. The develop mixer indexes the band weights

**The upstream code.**

`crates/develop/src/colorops.rs`, lines 262-270:

```rust
  262 |             let w = band_weights(h);
  263 |             let (mut dh, mut ds, mut dl) = (0.0, 0.0, 0.0);
  264 |             for i in 0..8 {
  265 |                 dh += w[i] * self.hue[i];
  266 |                 ds += w[i] * self.sat[i];
  267 |                 dl += w[i] * self.lum[i];
  268 |             }
  269 |             let chroma_w = (c / 0.12).min(1.0);
  270 |             h += dh * chroma_w;
```

**What it does.** Upstream's colour mixer walks `0..8` and indexes the band weights.

**Why it broke the fork.** The workspace is built with `-D warnings`, and clippy rejects a loop that indexes the sequence it iterates.

**The fork's answer.** The loop iterates the weights and keeps the index for the hue, saturation and luminance lookups; the three accumulations and their order are unchanged.

## 7. The sensor's matrix interpolation indexes the result

**The upstream code.**

`crates/develop/src/sensor/color.rs`, lines 43-51:

```rust
   43 | fn lerp(a: &Mat3, b: &Mat3, g: f64) -> Mat3 {
   44 |     let mut r = [[0.0; 3]; 3];
   45 |     for i in 0..3 {
   46 |         for j in 0..3 {
   47 |             r[i][j] = g * a.0[i][j] + (1.0 - g) * b.0[i][j];
   48 |         }
   49 |     }
   50 |     Mat3(r)
   51 | }
```

**What it does.** Upstream's `lerp` fills the result matrix by indexing it.

**Why it broke the fork.** The same clippy rule; the workspace's clippy run is a gate.

**The fork's answer.** The result is filled through the matrix's own iterators; the interpolation and the layout are unchanged.

## 8. The colour-space test helper indexes the matrix

**The upstream code.**

`crates/color-transform/src/space.rs`, lines 196-202:

```rust
  196 |     fn close(a: &Mat3, b: &[[f64; 3]; 3], tol: f64) {
  197 |         for i in 0..3 {
  198 |             for j in 0..3 {
  199 |                 assert!((a.0[i][j] - b[i][j]).abs() < tol, "[{i}][{j}] {} vs {}", a.0[i][j], b[i][j]);
  200 |             }
  201 |         }
  202 |     }
```

**What it does.** Upstream's `close` helper compares the matrices by indexing them.

**Why it broke the fork.** The same clippy rule, in a crate of the workspace.

**The fork's answer.** The helper walks the rows and the cells; the comparison, the tolerance and the failure message are unchanged.

## 9. Space is an activation key for every stop that asks for one

**The upstream code.**

`crates/ui-contract/src/runtime/keys/route.rs`, lines 744-745:

```rust
  744 |         None if press.key == Key::Space && plain && stop.keys.activate.space() => PressRoute::Activate,
  745 |         None if press.key == Key::Tab && plain && stop.keys.keep == focus::KeepKeys::Tab => PressRoute::Intent(Intent::Tab),
```

**What it does.** Upstream routes a Space press on a stop whose activation keys include Space straight to the activation, and it has no record of a Space that is down.

**Why it broke the fork.** The fork's rule is that a bare Space is the canvas's Hand key and only a control that asks for it takes Space. Upstream's route cannot say that a control asks, and it activates on the press, where a click activates on the release.

**The fork's answer.** The fork routes Space through `PressRoute::Space`, keeps the stop whose Space is down in `space_held`, and asks the stop whether it wants Space (by name or among its activation keys). The controls that answer Space declare it, and the router accepts Space for either way of asking.

