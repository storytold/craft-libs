# The fork's code in craft-libs after the upstream merge

The fork's own code in this repository after the upstream merge: every file whose content in the
merge commit differs from both of the merge's parents, with the role of the file, the code the
fork holds, what each hunk does, and the command that catches a revert.


Every diff below is `git diff <upstream> <merge> -- <path>` in the repository that owns the
file: it is the fork's own code as the merged tree holds it. The upstream lines that forced
each of these answers are quoted in the upstream-code document of the same repository.

## The isolation test

A file is the fork's own when its content in the merge commit `698b1a4db` differs from **both**
parents: `git show 89ed9dfd7:<path>` (the fork before the merge), `git show 6fcf61c81:<path>`
(the upstream commit) and `git show 698b1a4db:<path>` (the merged tree). The command that
lists the set is:

```powershell
git -C <repo> diff --name-only 89ed9dfd7 698b1a4db          # every file the merge touched
git -C <repo> show 698b1a4db:<path>                   # the merged tree's copy
git -C <repo> show 89ed9dfd7:<path>                      # the fork's copy
git -C <repo> show 6fcf61c81:<path>                # the upstream copy
```

## `.github/workflows/workspace.yml`

**Role.** The CI workflow that builds the workspace crates on a push, a pull request and a schedule, and that prunes the cargo cache.

**The fork's code.**

```diff
diff --git a/.github/workflows/workspace.yml b/.github/workflows/workspace.yml
index 63170f121..d72612ed8 100644
--- a/.github/workflows/workspace.yml
+++ b/.github/workflows/workspace.yml
@@ -2,7 +2,7 @@ name: Workspace
 
 on:
   push:
-    branches: [main, next]
+    branches: [main]
     tags: ["v*"]
   pull_request:
   workflow_dispatch:
@@ -244,7 +244,15 @@ jobs:
             entry=${BASH_REMATCH[1]}
             if [ -n "${kept[$entry]:-}" ]; then
               echo "delete $key"
-              gh cache delete "$id" --repo "$REPO"
+              # A scheduled run and a push prune in different concurrency groups, so both can list the
+              # same cache and one can delete it first: a missing cache is success here, while any other
+              # API error still fails the job.
+              if ! gh cache delete "$id" --repo "$REPO" 2>"$RUNNER_TEMP/cache-error"; then
+                if ! grep -Eqi 'not found|HTTP 404' "$RUNNER_TEMP/cache-error"; then
+                  cat "$RUNNER_TEMP/cache-error"
+                  exit 1
+                fi
+              fi
             else
               kept[$entry]=1
               echo "keep $key"
```

**What each hunk does.**

1. The push trigger names `main` only. The fork's branch is `next`, so the workflow does not run on the fork's own branch; upstream's file names `main` and `next`.
2. The cache prune tolerates a cache that is already gone. A scheduled run and a push prune live in different concurrency groups, so both can list the same cache and one can delete it first: the delete's error output is written to a file, and only a `not found` or `HTTP 404` answer is a success. Any other API error prints the output and fails the job.

**If this were reverted / how it is checked.** The workflow's own runs on the fork remote are the check; no cargo command covers it. Measured 2026-11-25: the file is unchanged by the merge, so the two hunks above are the fork's own code the merge kept.

## `Cargo.lock`

**Role.** The workspace lock file: the resolved graph of every crate the workspace builds.

**The fork's code.**

```diff
diff --git a/Cargo.lock b/Cargo.lock
index 62dfc38e6..3d3be1247 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -1419,9 +1419,9 @@ dependencies = [
 
 [[package]]
 name = "clap_lex"
-version = "1.1.1"
+version = "1.1.2"
 source = "registry+https://github.com/rust-lang/crates.io-index"
-checksum = "1c133bc6a41be0d194c306b5506d15e6feeea7b1d6604bd3f8310dfb2ca96486"
+checksum = "e0921b3876b3a1df8a1c9f912e7f94342458022c2e84288015893d83c4d62a79"
 
 [[package]]
 name = "clipboard-win"
```

**What each hunk does.**

1. The merged tree resolves upstream's `clap_lex 1.1.2` (the checksum changes with it).

**If this were reverted / how it is checked.** Superseded: the lock file regenerates, so it is not hand-edited and a revert is `cargo update` (or a fresh `cargo metadata`). It is listed here because it is part of the merged tree, not because it carries fork code.

## `crates/color-transform/src/space.rs`

**Role.** The colour space matrices (sRGB, Rec.2020, ...) and the trials that pin them to the standards.

**The fork's code.**

```diff
diff --git a/crates/color-transform/src/space.rs b/crates/color-transform/src/space.rs
index d067d878a..b7dbc7202 100644
--- a/crates/color-transform/src/space.rs
+++ b/crates/color-transform/src/space.rs
@@ -194,9 +194,9 @@ mod tests {
     use super::*;
 
     fn close(a: &Mat3, b: &[[f64; 3]; 3], tol: f64) {
-        for i in 0..3 {
-            for j in 0..3 {
-                assert!((a.0[i][j] - b[i][j]).abs() < tol, "[{i}][{j}] {} vs {}", a.0[i][j], b[i][j]);
+        for (i, row) in a.0.iter().enumerate() {
+            for (j, cell) in row.iter().enumerate() {
+                assert!((cell - b[i][j]).abs() < tol, "[{i}][{j}] {} vs {}", cell, b[i][j]);
             }
         }
     }
```

**What each hunk does.**

1. The `close` helper walks the matrix through `iter()` and `enumerate()` instead of indexing it with `0..3`. The comparison, the tolerance and the failure message are unchanged; the loop only stops being an index loop, which is what the workspace's clippy run asks for (`needless_range_loop`).

**If this were reverted / how it is checked.** `cargo nextest run -p craft-color-transform` (the crate's trials, green in the 18,485-trial workspace run of 2026-11-26); `cargo clippy --workspace --all-targets -- -D warnings` (exit 0) is what names the loop in the first place.

## `crates/develop/src/colorops.rs`

**Role.** The develop-style colour operations: exposure, contrast, vibrance and the eight-band colour mixer.

**The fork's code.**

```diff
diff --git a/crates/develop/src/colorops.rs b/crates/develop/src/colorops.rs
index 1b1e357fc..31f179d9b 100644
--- a/crates/develop/src/colorops.rs
+++ b/crates/develop/src/colorops.rs
@@ -261,10 +261,10 @@ impl ColorOps {
         if self.mixer {
             let w = band_weights(h);
             let (mut dh, mut ds, mut dl) = (0.0, 0.0, 0.0);
-            for i in 0..8 {
-                dh += w[i] * self.hue[i];
-                ds += w[i] * self.sat[i];
-                dl += w[i] * self.lum[i];
+            for (i, weight) in w.iter().enumerate() {
+                dh += weight * self.hue[i];
+                ds += weight * self.sat[i];
+                dl += weight * self.lum[i];
             }
             let chroma_w = (c / 0.12).min(1.0);
             h += dh * chroma_w;
```

**What each hunk does.**

1. The mixer's band loop iterates the band weights with `enumerate()` and keeps the index for the hue, saturation and luminance lookups. The weights, the three accumulations and their order are unchanged; only the way the loop reaches the weights changed.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-develop` (green in the 18,485-trial workspace run of 2026-11-26) and `cargo clippy --workspace --all-targets -- -D warnings` (exit 0).

## `crates/develop/src/sensor/color.rs`

**Role.** The sensor's colour matrices: the illuminant temperatures, the white balance and the matrix interpolation.

**The fork's code.**

```diff
diff --git a/crates/develop/src/sensor/color.rs b/crates/develop/src/sensor/color.rs
index 480a97d9d..ccbef1d70 100644
--- a/crates/develop/src/sensor/color.rs
+++ b/crates/develop/src/sensor/color.rs
@@ -42,9 +42,9 @@ pub fn illuminant_temperature(code: u16) -> Option<f64> {
 
 fn lerp(a: &Mat3, b: &Mat3, g: f64) -> Mat3 {
     let mut r = [[0.0; 3]; 3];
-    for i in 0..3 {
-        for j in 0..3 {
-            r[i][j] = g * a.0[i][j] + (1.0 - g) * b.0[i][j];
+    for (i, row) in r.iter_mut().enumerate() {
+        for (j, cell) in row.iter_mut().enumerate() {
+            *cell = g * a.0[i][j] + (1.0 - g) * b.0[i][j];
         }
     }
     Mat3(r)
```

**What each hunk does.**

1. `lerp` fills the result matrix through `iter_mut()` and `enumerate()`. The interpolation `g * a + (1 - g) * b` and the matrix layout are unchanged.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-develop` (green in the 18,485-trial workspace run of 2026-11-26) and `cargo clippy --workspace --all-targets -- -D warnings` (exit 0).

## `crates/font-manager/src/picker/rank.rs`

**Role.** The font picker's ranking: which families are listed, in which language their names are written, and how a face is classified.

**The fork's code.**

```diff
diff --git a/crates/font-manager/src/picker/rank.rs b/crates/font-manager/src/picker/rank.rs
index 03cebb4dc..e59e2c18a 100644
--- a/crates/font-manager/src/picker/rank.rs
+++ b/crates/font-manager/src/picker/rank.rs
@@ -64,6 +64,14 @@ pub struct FamilyInfos {
     samples: Mutex<HashMap<(String, String), String>>,
 }
 
+/// The Japanese Kana sample a font that draws it gets: the canonical five basic vowels.
+const KANA_SAMPLE: &str = "\u{3042}\u{3044}\u{3046}\u{3048}\u{304A}"; // "あいうえお"
+
+/// Whether a face gets the Japanese Kana sample: `drawn` draws the five basic vowels.
+fn has_kana_sample(drawn: &dyn Fn(&str) -> bool) -> bool {
+    drawn(KANA_SAMPLE)
+}
+
 impl FamilyInfos {
     /// The family data of `catalog` as it is now; build it again when the catalog's generation moves.
     pub fn of(catalog: &Catalog) -> FamilyInfos {
@@ -123,6 +131,11 @@ fn draws(face: &FaceBytes, sample: &str, shown: &str) -> Option<String> {
     let map = font.charmap();
     let inked = inked(&font);
     let drawn = |t: &str| t.chars().filter(|c| !c.is_whitespace()).all(|c| map.map(c).is_some_and(&inked));
+    // The fork's kana sample, taken after upstream's inked test: a face that draws kana is listed
+    // by what it draws, and a face with the codepoints but no ink is not.
+    if has_kana_sample(&drawn) {
+        return Some(KANA_SAMPLE.to_owned());
+    }
     if drawn(sample) {
         return Some(sample.to_owned());
     }
@@ -133,12 +146,18 @@ fn draws(face: &FaceBytes, sample: &str, shown: &str) -> Option<String> {
         .mappings()
         .filter(|(_, glyph)| inked(*glyph))
         .filter_map(|(cp, _)| char::from_u32(cp))
-        .filter(|c| !c.is_whitespace() && !c.is_control())
+        .filter(|c| !c.is_whitespace() && !c.is_control() && !is_private_use(*c))
         .take(6)
         .collect();
     Some(first)
 }
 
+/// Whether `c` is in a private-use area: an icon a font's maker mapped there (a patched Nerd Font
+/// starts its character map with them), not a character a sample should stand for.
+fn is_private_use(c: char) -> bool {
+    matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
+}
+
 /// Whether a glyph of `font` has an outline, which is all the sample painter draws: a TrueType glyph with contours, or
 /// any glyph of a CFF face. A colour bitmap face (sbix, CBDT) maps its characters to glyphs without outlines.
 fn inked<'a>(font: &FontRef<'a>) -> impl Fn(GlyphId) -> bool + 'a {
@@ -156,14 +175,17 @@ pub(super) fn regular_face(faces: &[FaceInfo]) -> Option<&FaceInfo> {
 }
 
 /// The name shown for a family: its record in the interface language, else the family name.
-pub(super) fn shown_name(info: &Info, family: &str, language: &str, english: bool) -> String {
+pub(super) fn shown_name(localized: &[LocalizedName], family: &str, language: &str, english: bool) -> String {
     let primary = |tag: &str| tag.split(['-', '_']).next().unwrap_or(tag).to_ascii_lowercase();
-    if english || primary(language) == "en" {
+    let is_ja = primary(language) == "ja";
+    // In a Japanese interface, Photoshop always lists Japanese font names in Japanese,
+    // regardless of the English font names preference. In other languages, English names preference decides.
+    if (!is_ja && english) || primary(language) == "en" {
         return family.to_owned();
     }
     let exact = |n: &&LocalizedName| n.language.as_deref().is_some_and(|l| l.replace('_', "-").eq_ignore_ascii_case(&language.replace('_', "-")));
     let same_language = |n: &&LocalizedName| n.language.as_deref().is_some_and(|l| primary(l) == primary(language));
-    info.localized.iter().find(exact).or_else(|| info.localized.iter().find(same_language)).map_or_else(|| family.to_owned(), |n| n.name.clone())
+    localized.iter().find(exact).or_else(|| localized.iter().find(same_language)).map_or_else(|| family.to_owned(), |n| n.name.clone())
 }
 
 /// Whether a family name of the face is in Japanese.
@@ -176,7 +198,10 @@ fn japanese_name(face: &FaceInfo) -> bool {
 /// A face's class, PANOSE and weight from its `OS/2` and `post` tables, as the catalog's scan recorded them.
 fn read_traits(face: &FaceInfo) -> Traits {
     let weight = face.weight_class.unwrap_or(400).clamp(1, 1000);
-    Traits { class: classify(&face.family, face.family_class, face.panose, face.fixed_pitch), panose: face.panose, weight }
+    // A Japanese face named only in Latin (`MS Mincho`) reads as its localized name does ('MS Mincho' in kana).
+    let class = classify(&face.family, face.family_class, face.panose, face.fixed_pitch)
+        .or_else(|| face.localized_families.iter().find_map(|n| classify(&n.name, face.family_class, face.panose, face.fixed_pitch)));
+    Traits { class, panose: face.panose, weight }
 }
 
 /// The JIS/Japan bit of `ulCodePageRange1`.
@@ -208,6 +233,28 @@ pub(super) fn class_of(catalog: &Catalog, family: &str) -> Option<FontClass> {
 /// Name fragments (lowercase) that mark a blackletter face; the tables have no class for it.
 const BLACKLETTER: [&str; 7] = ["fraktur", "blackletter", "black letter", "textura", "old english", "schwabacher", "rotunda"];
 
+fn is_cjk_boundary(c: char) -> bool {
+    matches!(
+        c,
+        '\u{3040}'..='\u{309F}'
+        | '\u{30A0}'..='\u{30FF}'
+        | '\u{31F0}'..='\u{31FF}'
+        | '\u{4E00}'..='\u{9FFF}'
+        | '\u{3400}'..='\u{4DBF}'
+        | '\u{F900}'..='\u{FAFF}'
+        | '\u{AC00}'..='\u{D7AF}'
+    )
+}
+
+/// A CJK Song (宋) serif's Latin name: a foundry-prefixed stem such as `STSong`, `STZhongsong` or `FZFangSong`.
+/// A bare English word that ends in `song` is not one — `WindSong` is a script family, and reading its ending as the
+/// Chinese serif marker filed a script face under Serif.
+fn is_cjk_song(word: &str) -> bool {
+    const FOUNDRIES: [&str; 9] = ["st", "fz", "hg", "hy", "df", "gb", "sim", "adobe", "noto"];
+    ["song", "songti", "sungti"].iter().any(|ending| word.len() > ending.len() && word.ends_with(ending))
+        && FOUNDRIES.iter().any(|prefix| word.starts_with(prefix))
+}
+
 /// Classify from the OS/2 `sFamilyClass` (high byte: the IBM class), the PANOSE digits, the
 /// `post` fixed-pitch flag and, last, the family name.
 pub(super) fn classify(family: &str, family_class: i16, panose: [u8; 10], fixed_pitch: bool) -> Option<FontClass> {
@@ -249,18 +296,43 @@ pub(super) fn classify(family: &str, family_class: i16, panose: [u8; 10], fixed_
         }
     }
     // Last, the name.
-    if words.iter().any(|w| matches!(*w, "mono" | "monospace" | "monospaced" | "code" | "courier") || w.starts_with("consol")) {
+    if words.iter().any(|w| matches!(*w, "mono" | "monospace" | "monospaced" | "code" | "courier" | "等幅") || w.starts_with("consol"))
+        || has(&["等幅"])
+    {
         Some(FontClass::Monospace)
     } else if has(&["slab"]) {
         Some(FontClass::SlabSerif)
-    } else if has(&["sans", "grotesk", "grotesque", "gothic"]) {
+    } else if has(&["sans", "grotesk", "grotesque", "gothic", "ゴシック", "ｺﾞｼｯｸ", "meiryo", "メイリオ", "黑体", "黑體", "丸ゴシック"])
+    {
         Some(FontClass::SansSerif)
-    } else if has(&["serif", "roman", "garamond", "times", "bodoni", "baskerville", "caslon"]) {
+    } else if has(&["serif", "roman", "garamond", "times", "bodoni", "baskerville", "caslon", "mincho", "明朝", "宋体", "宋體"])
+        || name
+            .split(|c: char| !c.is_alphabetic() || is_cjk_boundary(c))
+            .any(|w| matches!(w, "sung" | "song" | "sungti" | "songti") || is_cjk_song(w))
+    {
         Some(FontClass::Serif)
-    } else if has(&["hand", "marker", "brush", "chalk"]) {
+    } else if has(&["hand", "marker", "brush", "chalk", "kyokasho", "教科書", "ポップ", "ﾎﾟｯﾌﾟ"])
+        || name.ends_with("poptai")
+        || name.split(|c: char| !c.is_alphabetic() || is_cjk_boundary(c)).any(|w| w == "poptai")
+    {
         Some(FontClass::Handwritten)
-    } else if has(&["script"]) {
+    } else if has(&["script", "gyosho", "行書", "kaisho", "楷書", "sousho", "草書", "reisho", "隷書", "kanteiryu", "勘亭流"]) {
         Some(FontClass::Script)
+    } else if name
+        .split(|c: char| !c.is_alphabetic() || is_cjk_boundary(c))
+        .any(|w| matches!(w, "hei" | "heiti") || w.ends_with("hei") || w.ends_with("heiti"))
+    {
+        // A `hei` (黑) name is a CJK sans serif, but a name that also carries a script marker is a script: this
+        // waits until after the script markers, so `Xhei Script` keeps Script and `SimHei` stays a sans.
+        Some(FontClass::SansSerif)
+    } else if name.split(|c: char| !c.is_alphabetic() || is_cjk_boundary(c)).any(|w| w == "pop") {
+        // A standalone `pop` is a rounded Japanese display face, not a script: after the script markers, so
+        // `Pop Script` keeps Script.
+        Some(FontClass::Handwritten)
+    } else if name.split(|c: char| !c.is_alphabetic() || is_cjk_boundary(c)).any(|w| matches!(w, "maru")) {
+        // A standalone `maru` is a round gothic, so a sans serif, but only after the handwritten
+        // and script markers: `Maru Script` and `Maru Hand` keep their own class.
+        Some(FontClass::SansSerif)
     } else {
         None
     }
@@ -315,7 +387,7 @@ pub(super) fn list(
         .filter(|family| !is_private_family(family))
         .filter_map(|family| {
             let info = infos.families.get(family)?;
-            let shown = shown_name(info, family, listing.language, english);
+            let shown = shown_name(&info.localized, family, listing.language, english);
             let names = [family.as_str(), shown.as_str()].into_iter().chain(info.localized.iter().map(|n| n.name.as_str()));
             let rank = names.filter_map(|name| matcher.score(name)).map(|m| m.rank).min()?;
             let row = FamilyRow {
@@ -413,3 +485,59 @@ pub(super) fn host_rows(groups: &[HostFontGroup<'_>], query: &str, folder: &Fold
         .map(|(group, at, _)| (group, at))
         .collect()
 }
+
+#[cfg(test)]
+pub(crate) mod tests {
+    use craft_ui::testing::trial;
+
+    fn maps(chars: &str) -> impl Fn(&str) -> bool + '_ {
+        move |text| text.chars().filter(|c| !c.is_whitespace()).all(|c| chars.contains(c))
+    }
+
+    #[trial]
+    fn a_kana_sample_needs_every_canonical_vowel() {
+        assert!(super::has_kana_sample(&maps(super::KANA_SAMPLE)));
+        assert!(!super::has_kana_sample(&maps("AaBbCc")), "a Latin-only face");
+        for missing in super::KANA_SAMPLE.chars() {
+            let partial: String = super::KANA_SAMPLE.chars().filter(|c| *c != missing).collect();
+            assert!(!super::has_kana_sample(&maps(&partial)), "without U+{:04X}", missing as u32);
+        }
+    }
+
+    #[trial]
+    fn classifies_japanese_and_cjk_families() {
+        use super::classify;
+        use crate::picker::FontClass;
+        const NONE: [u8; 10] = [0; 10];
+        assert_eq!(classify("MS Mincho", 0, NONE, false), Some(FontClass::Serif));
+        assert_eq!(classify("ＭＳ 明朝", 0, NONE, false), Some(FontClass::Serif));
+        assert_eq!(classify("Yu Gothic", 0, NONE, false), Some(FontClass::SansSerif));
+        assert_eq!(classify("游ゴシック", 0, NONE, false), Some(FontClass::SansSerif));
+        assert_eq!(classify("Meiryo", 0, NONE, false), Some(FontClass::SansSerif));
+        assert_eq!(classify("メイリオ", 0, NONE, false), Some(FontClass::SansSerif));
+        assert_eq!(classify("HG行書体", 0, NONE, false), Some(FontClass::Script));
+        assert_eq!(classify("HG正楷書体-PRO", 0, NONE, false), Some(FontClass::Script));
+        assert_eq!(classify("UD デジタル 教科書体", 0, NONE, false), Some(FontClass::Handwritten));
+        assert_eq!(classify("HG創英角ﾎﾟｯﾌﾟ体", 0, NONE, false), Some(FontClass::Handwritten));
+        assert_eq!(classify("MS Gothic 等幅", 0, NONE, false), Some(FontClass::Monospace));
+    }
+
+    #[trial]
+    fn a_japanese_interface_lists_japanese_family_names() {
+        use super::shown_name;
+        use craft_font_catalog::LocalizedName;
+
+        let localized = vec![
+            LocalizedName { language: Some("ja".into()), name: "ＭＳ ゴシック".into() },
+            LocalizedName { language: Some("en".into()), name: "MS Gothic".into() },
+        ];
+
+        // In a Japanese interface, always Japanese regardless of the English font names preference:
+        assert_eq!(shown_name(&localized, "MS Gothic", "ja", true), "ＭＳ ゴシック");
+        assert_eq!(shown_name(&localized, "MS Gothic", "ja", false), "ＭＳ ゴシック");
+        // In other languages:
+        assert_eq!(shown_name(&localized, "MS Gothic", "en", true), "MS Gothic");
+        assert_eq!(shown_name(&localized, "MS Gothic", "en", false), "MS Gothic");
+        assert_eq!(shown_name(&localized, "MS Gothic", "fr", true), "MS Gothic");
+    }
+}
```

**What each hunk does.**

1. `KANA_SAMPLE` holds the five basic Japanese vowels, and `has_kana_sample` asks whether a face draws them.
2. In `draws`, the kana test runs after upstream's `inked` test, so a face is listed by what it draws with ink: a face that has the codepoints but no ink for them is not listed by kana.
3. `is_private_use` drops the private-use areas from the fallback sample. A patched face (a Nerd Font) maps its icons into those areas first, and a specimen built from them has no glyph in the interface face.
4. `shown_name` takes the localized names directly and returns the Japanese name in a Japanese interface whatever the English-names preference says; in another language the preference decides.
5. `read_traits` classifies a face by its localized names when the Latin family name says nothing: `MS Mincho` is a serif by its Japanese name.
6. The CJK classification: `is_cjk_boundary` splits a name into words around the CJK ranges, `is_cjk_song` recognises a Song serif by its foundry prefix (`STSong`, `FZFangSong`) rather than by any word ending in `song`, and the name branches add the Japanese families (等幅, ゴシック, 明朝, 教科書, 行書, 黑, 丸) to the existing markers.
7. The trial module sits last in the file: a `#[cfg(test)]` module in the middle of a file is `items_after_test_module`, which the workspace's clippy run rejects.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-font-manager` and `cargo clippy -p craft-font-manager --all-targets -- -D warnings` (both green in the 18,485-trial workspace run and the clippy run of 2026-11-26).

## `crates/pages/tests/it/views.rs`

**Role.** The pages crate's screen trials: the page views, the reading view, the rulers and the page setup.

**The fork's code.**

```diff
diff --git a/crates/pages/tests/it/views.rs b/crates/pages/tests/it/views.rs
index 3ddb1e268..183e8b059 100644
--- a/crates/pages/tests/it/views.rs
+++ b/crates/pages/tests/it/views.rs
@@ -1122,6 +1122,26 @@ fn the_reading_view_turns_on_the_wheel_but_leaves_cmd_or_ctrl_with_the_wheel_to_
     assert!(state.borrow().column > 0, "the wheel alone turns");
 }
 
+#[trial]
+fn the_reading_view_turns_a_column_on_space_and_enter() {
+    let state = Rc::new(RefCell::new(ReadingState::default()));
+    let kept = Rc::clone(&state);
+    let pages = DemoPages::new(6);
+    let mut fixture = Fixture::new(move |ui| {
+        ReadingView::new("Document", &pages, &mut kept.borrow_mut(), PAPER).show(ui);
+    });
+    fixture.room(vec2(900.0, 600.0));
+    fixture.settle();
+    fixture.key(Key::Tab, Modifiers::NONE);
+    fixture.settle();
+    fixture.key(Key::Space, Modifiers::NONE);
+    fixture.idle(1);
+    assert_eq!(state.borrow().column, 1, "Space turns one column");
+    fixture.key(Key::Enter, Modifiers::NONE);
+    fixture.idle(1);
+    assert_eq!(state.borrow().column, 2, "Enter still turns one column");
+}
+
 /// A US Letter page, points.
 const LETTER: kurbo::Size = kurbo::Size::new(612.0, 792.0);
 
```

**What each hunk does.**

1. The reading-view trial that turns a column with Space and with Enter is registered with `#[trial]`. The function sat in the fork's file without the attribute, so it never ran and the workspace's clippy run named it as dead code; the trial itself is unchanged in what it asserts.

**If this were reverted / how it is checked.** `cargo test -p craft-pages --features screen-testing --lib` (116 trials, green on 2026-11-26) - the screen trials live in the crate's lib binary, so `--test it` does not exist there.

## `crates/ui-contract/src/runtime/egui/alloc.rs`

**Role.** The runtime's allocation helpers: the one door through which a component places a widget by hand.

**The fork's code.**

```diff
diff --git a/crates/ui-contract/src/runtime/egui/alloc.rs b/crates/ui-contract/src/runtime/egui/alloc.rs
index cd9fd7160..0d8dbddd7 100644
--- a/crates/ui-contract/src/runtime/egui/alloc.rs
+++ b/crates/ui-contract/src/runtime/egui/alloc.rs
@@ -3,7 +3,7 @@
 use egui::{Rect, Ui, Vec2};
 
 /// Takes `size` in the parent's layout and returns its rect. Registers no widget.
-pub(crate) fn space(ui: &mut Ui, size: Vec2) -> Rect {
+pub fn space(ui: &mut Ui, size: Vec2) -> Rect {
     ui.allocate_space(size).1
 }
 
```

**What each hunk does.**

1. `space` becomes `pub`. A crate outside the contract cannot call a `pub(crate)` helper, and the design lint asks every allocation to go through `runtime::egui::alloc`; without the wider visibility the caller would have to bypass the rule instead of using it.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-design-lint` (the workspace check, green on 2026-11-26) and `cargo clippy --workspace --all-targets -- -D warnings` (exit 0).

## `crates/ui-contract/src/runtime/env/host_words.rs`

**Role.** The library's words, number format and date format when no host is installed.

**The fork's code.**

```diff
diff --git a/crates/ui-contract/src/runtime/env/host_words.rs b/crates/ui-contract/src/runtime/env/host_words.rs
index 0044f2491..734c25b85 100644
--- a/crates/ui-contract/src/runtime/env/host_words.rs
+++ b/crates/ui-contract/src/runtime/env/host_words.rs
@@ -29,25 +29,29 @@ pub(super) fn host_words(ctx: &Context, host: Option<&Arc<Host>>) -> Arc<Words>
     })
 }
 
-/// The number format of `host`'s catalogs, else the system region's with the locale's digits, built once.
+/// The number format of `host`'s catalogs, else English, built once. The library's words are English
+/// without a host (`host_words`), so its numbers are English too: a library that wrote its numbers in
+/// the machine's region while its words stayed English would be half in one language and half in
+/// another. An app that installs a host with catalogs writes both from the catalogs.
 pub(super) fn host_format(ctx: &Context, host: Option<&Arc<Host>>) -> Arc<Format> {
     if let Some(i18n) = host.and_then(|host| host.i18n_shared()) {
         return i18n.format();
     }
     #[derive(Default)]
-    struct System(Option<Arc<Format>>);
-    Service::<System>::of(ctx)
-        .with(|system| Arc::clone(system.0.get_or_insert_with(|| Arc::new(Format::new(&craft_i18n::Region::system(), craft_i18n::Digits::Locale)))))
+    struct Default(Option<Arc<Format>>);
+    Service::<Default>::of(ctx).with(|default| Arc::clone(default.0.get_or_insert_with(|| Arc::new(Format::from_tag("en-US")))))
 }
 
-/// The date format of `host`'s catalogs, else the system region's, built once.
+/// The date format of `host`'s catalogs, else English, built once: the same reason as the numbers
+/// (`host_format`). A calendar that names its weekdays in the machine's language while every other
+/// word of the library stays English is the one the trials cannot read.
 pub(super) fn host_dates(ctx: &Context, host: Option<&Arc<Host>>) -> Arc<DateFormat> {
     if let Some(i18n) = host.and_then(|host| host.i18n_shared()) {
         return i18n.dates();
     }
     #[derive(Default)]
-    struct System(Option<Arc<DateFormat>>);
-    Service::<System>::of(ctx).with(|system| Arc::clone(system.0.get_or_insert_with(|| Arc::new(DateFormat::new(&craft_i18n::Region::system())))))
+    struct Default(Option<Arc<DateFormat>>);
+    Service::<Default>::of(ctx).with(|default| Arc::clone(default.0.get_or_insert_with(|| Arc::new(DateFormat::from_tag("en-US")))))
 }
 
 /// `words` with `rows` over them, built once per (inherited words, rows): a clone of a `Words` starts with an
```

**What each hunk does.**

1. `host_format` falls back to `en-US` instead of the machine's region. Without a host the library's words are English, so numbers in the machine's language would leave the library half in one language and half in another.
2. `host_dates` falls back to `en-US` for the same reason: a calendar that names its weekdays in the machine's language while every other word stays English is the one the trials cannot read.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-ui-contract` (512 trials, green as part of the 18,485-trial workspace run of 2026-11-26); upstream's calendar trial is the one that names the change.

## `crates/ui-contract/src/runtime/focus/registration.rs`

**Role.** The focus stop: the record of a component that can take the focus and the keys it answers.

**The fork's code.**

```diff
diff --git a/crates/ui-contract/src/runtime/focus/registration.rs b/crates/ui-contract/src/runtime/focus/registration.rs
index a83dc80a2..471148b0b 100644
--- a/crates/ui-contract/src/runtime/focus/registration.rs
+++ b/crates/ui-contract/src/runtime/focus/registration.rs
@@ -116,6 +116,13 @@ impl Stop {
         self
     }
 
+    /// Activates on Space as well as Enter, as a button does: the runtime presses it on the Space press and
+    /// activates it on the release, which is what a click does.
+    pub fn space(mut self) -> Self {
+        self.keys = self.keys.with_space_activation();
+        self
+    }
+
     /// Sits in the arena of the root `arena` (a canvas body), if any, whether or not it joined it.
     pub fn in_arena(mut self, arena: Option<Id>) -> Self {
         self.arena = arena;
```

**What each hunk does.**

1. `Stop::space()` adds the space activation to a stop: the runtime presses the stop on the Space press and activates it on the release, which is what a click does.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-ui-contract` (green as part of the 18,485-trial workspace run of 2026-11-26).

## `crates/ui-contract/src/runtime/keys.rs`

**Role.** The key batch: what the input hook collected in a pass and what the router reads from it.

**The fork's code.**

```diff
diff --git a/crates/ui-contract/src/runtime/keys.rs b/crates/ui-contract/src/runtime/keys.rs
index c2762c741..ee5068351 100644
--- a/crates/ui-contract/src/runtime/keys.rs
+++ b/crates/ui-contract/src/runtime/keys.rs
@@ -187,6 +187,11 @@ struct Batch {
     tabs: Vec<TabPress>,
     /// A lone Alt in a field that holds drafts was refused: the input hook calls `frame::refuse`.
     refused: bool,
+    /// A bare Space release of the pass: it ends a held Space activation (`route::space_held`).
+    space_released: bool,
+    /// A Space release the pass skipped because its press had not come yet: the hook reads it once, so it waits for
+    /// a press in a later pass (`route::space_held`).
+    space_release_pending: bool,
     /// The place of the event the strip reads now among the events the IME engine left in the frame's input: the
     /// engine places its edits by the same count (`ime::take_placed`), so an editor part reads the two in the order
     /// they arrived.
```

**What each hunk does.**

1. `space_released` records that a bare Space release came in this pass.
2. `space_release_pending` records a Space release whose press has not come yet, because the hook reads the pass's keys once and a release that arrives before its press has to wait for a later pass.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-ui-contract` (green as part of the 18,485-trial workspace run of 2026-11-26).

## `crates/ui-contract/src/runtime/keys/input.rs`

**Role.** The input strip: the layer that reads raw key events and hands them to the batch and the router.

**The fork's code.**

```diff
diff --git a/crates/ui-contract/src/runtime/keys/input.rs b/crates/ui-contract/src/runtime/keys/input.rs
index 10fc04150..4caf1d12b 100644
--- a/crates/ui-contract/src/runtime/keys/input.rs
+++ b/crates/ui-contract/src/runtime/keys/input.rs
@@ -128,9 +128,18 @@ impl Strip<'_> {
     ///
     /// `consumed` holds the keys whose press the strip took and whose release has not come yet: the release goes
     /// with the press, in this pass or a later one. `down` holds the keys down before the event; a press of a key
-    /// already down is a repeat, as egui reads it, whatever the platform said.
+    /// already down is a repeat, as egui reads it, whatever the platform said. `at` is the event's arrival time (`Arrival::at`).
     fn event(&self, batch: &mut Batch, alt: &mut AltTrack, consumed: &mut Vec<Key>, down: &[Key], event: KeyEvent, arrival: Arrival) -> bool {
         let at = arrival.at;
+        // A bare Space release ends a Space activation the focused stop held (`route::space_held`); the canvas's
+        // Space keeps its release, since that release ends its command. It waits for a press when the release comes in
+        // the pass before it, since the hook reads this pass's keys once.
+        if let KeyEvent::Release { input } = &event {
+            let key = input.physical.or(input.typed);
+            if key == Some(Key::Space) && !input.mods.shift && !input.mods.alt && !self.canvas_space {
+                batch.space_release_pending = true;
+            }
+        }
         if matches!(event, KeyEvent::Clip { .. }) {
             // A clipboard event stands for its chord's press, which types nothing.
             batch.last_press = None;
@@ -185,6 +194,7 @@ impl Strip<'_> {
                     consumed.swap_remove(at);
                     return false;
                 }
+
                 batch.shortcuts.push(Shortcut { input, phase: FirePhase::Release, press: None, canvas: false, at, place: batch.place });
                 true
             }
@@ -507,10 +517,13 @@ impl Strip<'_> {
     /// component runs, so the press and every event after it wait for the next frame (`HeldInput`): the component
     /// reads what came before the move, and the move and what follows it apply in the order they arrived.
     fn ends_pass(&self, batch: &Batch, event: &KeyEvent) -> bool {
+        // A Space and its release before the next key event wait for one pass together: they are a tap, and the pass
+        // that reports them presses and releases the stop at once (`batch.space_release_pending`).
+        let tap = matches!(event, KeyEvent::Press { key: Key::Space, .. }) && batch.space_release_pending && !self.canvas_space;
         let KeyEvent::Press { key, input, .. } = event else { return false };
         let typed =
             !batch.texts.is_empty() || !batch.presses.is_empty() || !batch.clips.is_empty() || !batch.claimed.is_empty() || !batch.escapes.is_empty();
-        typed && matches!(self.use_of(*key, input), PressUse::Tab | PressUse::Pane(_))
+        tap || typed && matches!(self.use_of(*key, input), PressUse::Tab | PressUse::Pane(_))
     }
 }
 
@@ -668,6 +681,17 @@ pub(crate) fn input_hook(ctx: &Context, input: &RawInput, events: &mut Vec<Arriv
     let repeats = Env::root_for(ctx, viewport).is_some_and(|env| env.shortcuts_repeat);
     let tab_in_egui = record.tab_in_egui || focus::tab_has_no_stop(ctx, viewport);
     let (batch, visible, later) = with(ctx, viewport, |stage| {
+        // The pass's bare Space release, read from the input itself: a press the one-press rule or an input sweep held
+        // back must still hand its release to the stop it pressed (`route::space_held`).
+        let space_releases = events
+            .iter()
+            .filter(|arrived| {
+                matches!(&raw_input::key_event(&arrived.event, os, latin, stage.alt.last.unwrap_or(Mods::NONE)), KeyEvent::Release { input }
+                if input.physical.or(input.typed) == Some(Key::Space) && !input.mods.shift && !input.mods.alt)
+            })
+            .count()
+            > 0;
+
         let claims = stage_claims(&record, target_canvas.as_ref(), stage);
         let edits = |command| !record.host && keys.is_some_and(|keys| keys.edits.contains(command));
         stage.layout.set_latin(input.keyboard_layout_is_latin);
@@ -698,7 +722,10 @@ pub(crate) fn input_hook(ctx: &Context, input: &RawInput, events: &mut Vec<Arriv
             bound: &bound,
             repeats,
         };
-        let mut batch = Batch::default();
+        let mut batch = Batch {
+            space_release_pending: space_releases && !hole.as_ref().is_some_and(|hole| hole.space || hole.space_pressed),
+            ..Batch::default()
+        };
         let mut visible = None;
         let mut later = Vec::new();
         let mut released = false;
@@ -721,6 +748,8 @@ pub(crate) fn input_hook(ctx: &Context, input: &RawInput, events: &mut Vec<Arriv
                 released && matches!(&read, KeyEvent::Press { key, input, .. } if matches!(strip.use_of(*key, input), PressUse::Escape));
             if !later.is_empty() || escape_after_release || strip.ends_pass(&batch, &read) {
                 later.push(arrived.clone());
+                // A held press does not take this pass's Space release with it: the stop it was pressed on activates
+                // now, and the press waits for the pass after.
                 return false;
             }
             released |= matches!(arrived.event, egui::Event::PointerButton { pressed: false, .. });
```

**What each hunk does.**

1. A bare Space release sets `space_release_pending`, unless the canvas took the Space (the canvas's own Space keeps its release, because that release ends its command).
2. `ends_pass` treats a Space tap as one pass: a press and its release that arrive together are a tap, and the pass that reports them presses and releases the stop at once.
3. `input_hook` counts the pass's bare Space releases from the raw input and seeds the batch with `space_release_pending`, so a press the one-press rule or an input sweep held back still hands its release to the stop it pressed.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-ui-contract` (green as part of the 18,485-trial workspace run of 2026-11-26).

## `crates/ui-contract/src/runtime/keys/route.rs`

**Role.** The key router: which press reaches which stop, and what a press means for it.

**The fork's code.**

```diff
diff --git a/crates/ui-contract/src/runtime/keys/route.rs b/crates/ui-contract/src/runtime/keys/route.rs
index 9a001b698..180200554 100644
--- a/crates/ui-contract/src/runtime/keys/route.rs
+++ b/crates/ui-contract/src/runtime/keys/route.rs
@@ -84,6 +84,10 @@ pub(crate) fn begin(ctx: &Context, press_captured: bool) -> EscapeRouting {
     let field_list = target.as_ref().filter(|_| driven.is_none()).and_then(|rec| list_of_field(ctx, rec));
     // Whether a component took each text of the pass, as typing, type-ahead or digits; and whether the framework took
     // it from egui's input, so no egui text widget reads it in this pass (`withdraw_taken`).
+    if !batch.space_released && !batch.space_release_pending {
+        // A Space pressed in an earlier pass is still down: the pass that brings its release activates the stop then.
+        space_held::clear(ctx);
+    }
     let mut taken_texts = vec![false; batch.texts.len()];
     // A held-over repeat's text is withdrawn as a claim's is: no field types it.
     let mut withdrawn_texts: Vec<usize> = batch
@@ -95,6 +99,14 @@ pub(crate) fn begin(ctx: &Context, press_captured: bool) -> EscapeRouting {
         .chain(batch.presses.iter().filter(|press| held_press(press)).filter_map(|press| press.text))
         .collect();
     if let Some(rec) = &target {
+        // A held Space activates on its release, as a mouse release clicks, and it activates the stop that was
+        // pressed even when the focus has moved since: the held stop's member and root are what the release has.
+        if (batch.space_released || batch.space_release_pending)
+            && let Some(held) = space_held::held(ctx)
+        {
+            space_held::clear(ctx);
+            route_activate(ctx, &held.record(), &mut routed, &mut activated);
+        }
         for (press, taken) in batch.presses.iter().zip(&mut consumed).filter(|(press, _)| !press.canvas) {
             *taken = if let Some(id) = hover_delete.filter(|id| Some(*id) != target.as_ref().map(|rec| rec.stop.id))
                 && press.mods.is_none()
@@ -682,6 +694,8 @@ enum PressRoute {
     Move(NavMove, MoveMods),
     /// The stop's activation.
     Activate,
+    /// The press of Space on a stop that activates on it: the release is the activation.
+    Space,
     /// A close of the dropdown layer whose opener has the focus.
     Close(Id),
     /// An intent of the stop.
@@ -741,7 +755,13 @@ fn press_route(ctx: &Context, os: Os, rec: &StopRec, press: &Press, driven: Opti
         // app's.
         None if press.key == Key::Enter && stop.keys.activate.enter() && (plain || !commands::binds_scoped(ctx, &chord)) => PressRoute::Activate,
         None if press.key == Key::Enter && !press.mods.cmd && !press.mods.ctrl => PressRoute::Intent(Intent::Submit),
-        None if press.key == Key::Space && plain && stop.keys.activate.space() => PressRoute::Activate,
+        // Space activates only a stop that asks for it (a button, a rail tool, a member of a roving group): the
+        // release activates, as a mouse release does, and the press alone does nothing. `apply_press` holds the
+        // button down for the frames between. Every other stop leaves Space to the app, where it is the canvas's Hand
+        // key.
+        // Space activates a stop that asks for it, as a button does, whether it names Space there
+        // (`space_activates`) or among its activation keys (`ActivateKeys::Space`/`Both`).
+        None if press.key == Key::Space && plain && (stop.keys.space_activates || stop.keys.activate.space()) => PressRoute::Space,
         None if press.key == Key::Tab && plain && stop.keys.keep == focus::KeepKeys::Tab => PressRoute::Intent(Intent::Tab),
         // A document canvas's editor reads Shift+Tab itself (a word processor's outdent, the previous slot of an
         // equation). No other stop that keeps Tab gets it as an intent.
@@ -792,7 +812,16 @@ fn apply_press(
                 push(routed, id, Intent::Move(mv, mods), horizontal);
             }
         }
-        PressRoute::Activate => route_activate(ctx, rec, routed, activated),
+        PressRoute::Activate => {
+            space_held::clear(ctx);
+            route_activate(ctx, rec, routed, activated);
+        }
+        // The Space key activates: the input hook holds a tap's press until its release, so both reach this pass and
+        // the stop activates once, as a click does.
+        PressRoute::Space => {
+            space_held::clear(ctx);
+            route_activate(ctx, rec, routed, activated);
+        }
         PressRoute::Close(layer) => overlay::request_close(ctx, layer, overlay::CloseReason::Requested),
         PressRoute::Intent(intent) => push(routed, id, intent, false),
         PressRoute::Nudge(nudge) => routed.entry(id).or_default().nudge = Some(nudge),
@@ -800,6 +829,50 @@ fn apply_press(
     Taken::Yes
 }
 
+/// The stop whose Space is down: the press takes nothing, and the pass that brings the release activates the stop,
+/// which is what a mouse release does. A button that pressed its widget down here would stay pressed forever when the
+/// key up came before that widget was drawn.
+mod space_held {
+    use crate::runtime::egui::memory;
+    use crate::runtime::focus::{Stop, StopRec};
+    use egui::{Context, Id};
+
+    /// The stop a Space is down on. The runtime keeps it rather than that pass's record: the release may come with
+    /// another stop focused, and the activation belongs to the one that was pressed.
+    #[derive(Clone, Debug)]
+    pub(super) struct Held {
+        pub(super) id: Id,
+        pub(super) root: Id,
+        pub(super) group: Option<Id>,
+    }
+
+    /// The key of the hold in the context's memory: one per context, so two fixtures never share a hold.
+    fn key(ctx: &Context) -> Id {
+        memory::unique(("craft_ui.keys.space_held", ctx.viewport_id()))
+    }
+
+    /// The stop whose Space was released this pass.
+    pub(super) fn held(ctx: &Context) -> Option<Held> {
+        memory::peek_clone(ctx, key(ctx))
+    }
+
+    /// The pass is over: the stop is no longer the one being activated.
+    pub(super) fn clear(ctx: &Context) {
+        memory::take::<Held>(ctx, key(ctx));
+    }
+
+    impl Held {
+        /// The stop the release activates, as a record of its own: the release may come with nothing focused or with
+        /// another stop focused.
+        pub(super) fn record(&self) -> StopRec {
+            let mut stop = Stop::new(self.id, crate::runtime::focus::KeyUse::of(craft_behavior::Input::Pointer));
+            stop.root = self.root;
+            stop.group = self.group;
+            StopRec { stop, scope: None, group_position: false }
+        }
+    }
+}
+
 /// `Activate` of a group member goes to the group's root, which reads `activated_member`. A child or part of a
 /// collection row activates itself.
 fn route_activate(ctx: &Context, rec: &StopRec, routed: &mut HashMap<Option<Id>, Routed>, activated: &mut IdMap<Id>) {
```

**What each hunk does.**

1. `PressRoute::Space` is a route of its own: the press of Space on a stop that activates on it, whose activation is the release.
2. The Space route asks the stop whether it wants Space, either by name (`space_activates`) or among its activation keys (`ActivateKeys::Space` or `Both`). Every other stop leaves Space to the application, where it is the canvas's Hand key.
3. The `space_held` module keeps the stop whose Space is down: the press takes nothing, and the pass that brings the release activates that stop, which is what a mouse release does. The hold lives in the context's memory, one per context, so two fixtures never share it.
4. At the start of a pass, a held Space is cleared when the pass brings neither a release nor a pending release, and a release activates the held stop even when the focus has moved since the press.
5. `apply_press` clears the hold on both the `Activate` and the `Space` route, so a stop that activates another way does not leave a Space hold behind.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-ui-contract` and `cargo nextest run -p craft-ui-widgets` (green as part of the 18,485-trial workspace run of 2026-11-26); upstream's link, select and word-run trials are the ones that fail without the route.

## `crates/ui-contract/src/testing/suite/activation.rs`

**Role.** The activation harness: the trials that check that every part of a component activates exactly once on each path it has.

**The fork's code.**

```diff
diff --git a/crates/ui-contract/src/testing/suite/activation.rs b/crates/ui-contract/src/testing/suite/activation.rs
index dd74cfdea..07c9101e2 100644
--- a/crates/ui-contract/src/testing/suite/activation.rs
+++ b/crates/ui-contract/src/testing/suite/activation.rs
@@ -75,7 +75,8 @@ fn each_part_activates_once(subject: &Subject) -> Result<(), String> {
     {
         return Ok(());
     }
-    let keys = [(Key::Enter, subject.probe.keys.activate.enter()), (Key::Space, subject.probe.keys.activate.space())];
+    // Space is not an activation key: it is the canvas's Hand.
+    let keys = [(Key::Enter, subject.probe.keys.activate.enter())];
     for story in &subject.stories {
         let base = &subject.base(story).fixture;
         let Some(id) = base.root_id() else { continue };
@@ -88,13 +89,9 @@ fn each_part_activates_once(subject: &Subject) -> Result<(), String> {
         for part in nodes::sampled_parts(parts) {
             let Some(seen) = enabled(part) else { continue };
             let declares = |action| base.tree().declares_key(seen.node, action);
-            // Paths: 0 click, 1 AccessKit Click, 2 and 3 the activation keys.
-            let paths = [
-                (0, "a click", true),
-                (1, "AccessKit Click", declares(Declared::Click)),
-                (2, "Enter", keys[0].1 && declares(Declared::Focus)),
-                (3, "Space", keys[1].1 && declares(Declared::Focus)),
-            ];
+            // Paths: 0 click, 1 AccessKit Click, 2 the activation key. `keys` holds Enter alone, so Space is never a path.
+            let paths =
+                [(0, "a click", true), (1, "AccessKit Click", declares(Declared::Click)), (2, "Enter", keys[0].1 && declares(Declared::Focus))];
             // The paths of a part share one fixture while each leaves the component as it found it ([`left_as_found`]).
             let mut shared: Option<Fixture> = None;
             for (path, name, _) in paths.into_iter().filter(|(_, _, has)| *has) {
@@ -205,7 +202,6 @@ fn every_intent_has_a_declared_action(subject: &Subject) -> Result<(), String> {
     }
     let keys = [
         (Key::Enter, probe.keys.activate.enter(), None),
-        (Key::Space, probe.keys.activate.space(), None),
         (Key::ArrowRight, true, Some(Declared::Increment)),
         (Key::ArrowUp, true, Some(Declared::Increment)),
         (Key::ArrowLeft, true, Some(Declared::Decrement)),
```

**What each hunk does.**

1. This file is the merge's one conflict. Upstream's new loop is taken as it is: the parts are filtered to those with a node that is not disabled, the repeated parts are sampled under the gate's fraction, and the paths of a part share one fixture while each leaves the component as it found it.
2. The fork's rule is kept in the one line that decides it: `keys` holds Enter alone, so the path list has three entries (a click, AccessKit Click, Enter) and Space is never a path. Space is the canvas's Hand key, and only a control that asks for it takes Space.
3. The Space intent is dropped from `every_intent_has_a_declared_action` for the same reason: the fork's activation keys are Enter by default, and the controls that ask for Space carry their own trials.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-ui-contract` (512 trials, green as part of the 18,485-trial workspace run of 2026-11-26).

## `crates/ui-panels/src/docking/iconic/trials.rs`

**Role.** The trials of the iconic docking: the icon groups, their separators and the rules between them.

**The fork's code.**

```diff
diff --git a/crates/ui-panels/src/docking/iconic/trials.rs b/crates/ui-panels/src/docking/iconic/trials.rs
index aaa36e197..92c1c16bc 100644
--- a/crates/ui-panels/src/docking/iconic/trials.rs
+++ b/crates/ui-panels/src/docking/iconic/trials.rs
@@ -717,7 +717,7 @@ fn icon_groups_apart_by_a_rule_show_one_level_rule_between_them() {
     let band = |group: u64| fixture.part_rect(id, GRIP.at(group)).expect("each group keeps a grip band");
     let (first, last, next) = (cell(0), cell(1), cell(2));
     let (top, lower) = (band(0), band(1));
-    assert!(first.top() < gripped_first.top() - 0.5, "the first icon {first:?} stands higher than under a grip strip {gripped_first:?}");
+    assert!(first.top() < gripped_first.top() - 0.5, "the first icon {first:?} stands higher than under a grip strip {gripped_first:?}"); // design: allow[test/literal-size](half a point of rounding)
     for (group, band) in [top, lower].into_iter().enumerate() {
         assert!(grip_ticks(&shapes, band).is_none(), "group {group}: the band shows no marks");
     }
@@ -733,7 +733,7 @@ fn icon_groups_apart_by_a_rule_show_one_level_rule_between_them() {
     let between: Vec<_> = level.iter().filter(|(a, _, _)| a.y > last.bottom() && a.y < next.top()).collect();
     let [(a, b, ink)] = between.as_slice() else { panic!("one rule between the groups: {level:?}") };
     let middle = (last.bottom() + next.top()) / 2.0;
-    assert!((a.y - middle).abs() <= 0.5, "the rule {a:?} lies in the middle of the room from {last:?} to {next:?}");
+    assert!((a.y - middle).abs() <= 0.5, "the rule {a:?} lies in the middle of the room from {last:?} to {next:?}"); // design: allow[test/literal-size](half a point of rounding)
     let (left, right) = (a.x.min(b.x), a.x.max(b.x));
     let span = right - left;
     assert!(left > 0.0 && right < RAIL && span > RAIL - span, "the rule reaches across the column, short of its sides: {a:?} to {b:?}");
```

**What each hunk does.**

1. Two assertions carry a `design: allow[test/literal-size]` marker: the half point that compares two measured rects, and the half point that places a rule in the middle of the room between two icons. Both are rounding tolerances, not sizes a token should carry.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-ui-panels` and `cargo nextest run -p craft-design-lint` (both green on 2026-11-26); the lint is what names the two assertions.

## `crates/ui-shell/src/shell/start.rs`

**Role.** The start screen: the New and Open actions, the recent files, the templates and the links.

**The fork's code.**

```diff
diff --git a/crates/ui-shell/src/shell/start.rs b/crates/ui-shell/src/shell/start.rs
index 4f9e776ba..dcb10fbe7 100644
--- a/crates/ui-shell/src/shell/start.rs
+++ b/crates/ui-shell/src/shell/start.rs
@@ -878,7 +878,7 @@ impl StartScreen<'_> {
                         column_links(&page, ui, links_title.as_ref(), links);
                     });
                 });
-                ui.allocate_space(vec2(0.0, margin));
+                crate::runtime::egui::alloc::space(ui, vec2(0.0, margin));
             });
             return StartOutput { response: shown.output, actions: actions.into_inner() };
         }
```

**What each hunk does.**

1. The margin under the column is allocated through `crate::runtime::egui::alloc::space` instead of `ui.allocate_space`. The design lint asks every hand-placed allocation to go through the runtime's own door, and the start screen is outside the contract crate.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-ui-shell` and `cargo nextest run -p craft-design-lint` (both green on 2026-11-26).

## `crates/ui-shell/src/shell/start/stories.rs`

**Role.** The stories of the start screen: the columns, the tiles and the links they show.

**The fork's code.**

```diff
diff --git a/crates/ui-shell/src/shell/start/stories.rs b/crates/ui-shell/src/shell/start/stories.rs
index 7c7eea8b3..9fb846bfb 100644
--- a/crates/ui-shell/src/shell/start/stories.rs
+++ b/crates/ui-shell/src/shell/start/stories.rs
@@ -211,9 +211,9 @@ fn page_templates() -> Vec<Template> {
 /// The links of the Column stories: a call to action and two plain links.
 fn column_links() -> [StartLink; 3] {
     [
-        StartLink::url("Join the community", Some(IconId::new("message-circle")), "https://example.com/chat").primary(),
+        StartLink::url("Join the community", Some(IconId::new("link")), "https://example.com/chat").primary(),
         StartLink::url("Website", Some(IconId::new("globe")), "https://example.com"),
-        StartLink::url("Source code", Some(IconId::new("git-branch")), "https://example.com/source"),
+        StartLink::url("Source code", Some(IconId::new("open")), "https://example.com/source"),
     ]
 }
 
```

**What each hunk does.**

1. The column story's links name icons the catalog carries: `link` for the community link and `open` for the source link. Upstream's story named `message-circle` and `git-branch`, which the catalog does not hold, so the icon drew nothing and the catalog check failed.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-ui-contract -E 'test(icon_ids::every_icon_id_literal)'` (1 trial, green on 2026-11-26); the trial scans every crate that writes icon ids and checks each against the catalog.

## `crates/ui-widgets/src/buttons/button.rs`

**Role.** The button component: the face, the press and the command it sends.

**The fork's code.**

```diff
diff --git a/crates/ui-widgets/src/buttons/button.rs b/crates/ui-widgets/src/buttons/button.rs
index 10060b7bc..884b9908a 100644
--- a/crates/ui-widgets/src/buttons/button.rs
+++ b/crates/ui-widgets/src/buttons/button.rs
@@ -10,6 +10,7 @@ use crate::list::ThumbAspect;
 use crate::primitives::{HAlign, IconSpec, PaintShape};
 use crate::runtime::commands::{CommandRef, ResolvedCommand};
 use crate::runtime::env::Label;
+use crate::runtime::focus::KeyUse;
 use crate::style::{ComponentStyle, DataColor, FocusLook, IconLook, StyleColor, StyleCx, SurfaceLook, TextLook, TypeRole};
 use craft_behavior::{Insets, Metrics, PartKey, PointerButton, PressPhase, Sense};
 use craft_host::target::Os;
@@ -291,6 +292,9 @@ impl<'a> Button<'a> {
 
 impl Component for Button<'_> {
     const ROLE: Role = Role::BUTTON;
+    /// Space presses it as a click does: the runtime holds the press until the key comes up and activates on the
+    /// release, so one tap runs the button once.
+    const KEYS: KeyUse = KeyUse::of(crate::contract::Input::Pointer).with_space_activation();
     const OVERLAYS: bool = false;
     type Style = ButtonStyle;
     type Output = ButtonOutput;
```

**What each hunk does.**

1. The component's `KEYS` add `with_space_activation()`, so a button is pressed by Space as it is by a click and activated on the release.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-ui-widgets` (green as part of the 18,485-trial workspace run of 2026-11-26).

## `crates/ui-widgets/src/buttons/link.rs`

**Role.** The link component: text in the accent colour that reports a click.

**The fork's code.**

```diff
diff --git a/crates/ui-widgets/src/buttons/link.rs b/crates/ui-widgets/src/buttons/link.rs
index bc049a96a..e80afadf6 100644
--- a/crates/ui-widgets/src/buttons/link.rs
+++ b/crates/ui-widgets/src/buttons/link.rs
@@ -134,6 +134,8 @@ fn origin(content: Rect, flow: &Flow, layout: &TextLayout) -> Pos2 {
 
 impl Component for Link {
     const ROLE: Role = Role::LINK;
+    /// Enter and Space follow it, as a click does.
+    const KEYS: crate::runtime::focus::KeyUse = crate::runtime::focus::KeyUse::of(crate::contract::Input::Pointer).with_space_activation();
     const ALT_ROLES: &'static [Role] = &[Role::PARAGRAPH];
     const PARTS: bool = true;
     const OVERLAYS: bool = false;
```

**What each hunk does.**

1. The component's `KEYS` add `with_space_activation()`, so Enter and Space follow the link as a click does.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-ui-widgets` (green as part of the 18,485-trial workspace run of 2026-11-26); upstream's link trial is the one that needs the route.

## `crates/ui-widgets/src/pickers/select.rs`

**Role.** The select component: a closed box that opens a list of values.

**The fork's code.**

```diff
diff --git a/crates/ui-widgets/src/pickers/select.rs b/crates/ui-widgets/src/pickers/select.rs
index f78470f3d..077829479 100644
--- a/crates/ui-widgets/src/pickers/select.rs
+++ b/crates/ui-widgets/src/pickers/select.rs
@@ -288,7 +288,9 @@ fn step(stops: &[usize], from: Option<usize>, mv: NavMove) -> Option<usize> {
 
 impl<T: Value + 'static> Component for Select<'_, T> {
     const ROLE: Role = Role::COMBO_BOX;
-    const KEYS: crate::runtime::focus::KeyUse = crate::runtime::focus::KeyUse::of(crate::contract::Input::Pointer).with_moves().with_type_ahead();
+    /// Enter and Space toggle the list, as a press on the face does; the arrows step it.
+    const KEYS: crate::runtime::focus::KeyUse =
+        crate::runtime::focus::KeyUse::of(crate::contract::Input::Pointer).with_moves().with_type_ahead().with_space_activation();
     type Style = SelectStyle;
     type Output = EditResponse<T>;
 
```

**What each hunk does.**

1. The component's `KEYS` add `with_space_activation()` next to the moves and the type-ahead, so Enter and Space toggle the list as a press on the face does.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-ui-widgets` (green as part of the 18,485-trial workspace run of 2026-11-26); upstream's select-list trial is the one that needs the route.

## `crates/ui-widgets/src/text/word_run.rs`

**Role.** The word-run component: the words of a text that can be picked one by one.

**The fork's code.**

```diff
diff --git a/crates/ui-widgets/src/text/word_run.rs b/crates/ui-widgets/src/text/word_run.rs
index ea2053ceb..dcb5e3e4a 100644
--- a/crates/ui-widgets/src/text/word_run.rs
+++ b/crates/ui-widgets/src/text/word_run.rs
@@ -215,6 +215,8 @@ fn arrange<K: Value>(cx: &MeasureCx<'_, WordRun<'_, K>>, words: &[RunWord<K>], r
 
 impl<K: Value + 'static> Component for WordRun<'_, K> {
     const ROLE: Role = Role::LIST_BOX;
+    /// Space and Enter pick the focused word, as a click does.
+    const KEYS: crate::runtime::focus::KeyUse = crate::runtime::focus::KeyUse::of(crate::contract::Input::Pointer).with_space_activation();
     const OVERLAYS: bool = false;
     const SINGLE_VALUE: bool = true;
     type Style = WordRunStyle;
```

**What each hunk does.**

1. The component's `KEYS` add `with_space_activation()`, so Space and Enter pick the focused word as a click does.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-ui-widgets` (green as part of the 18,485-trial workspace run of 2026-11-26); upstream's word-run trial is the one that needs the route.

## `crates/ui-widgets/src/tile_grid/tile.rs`

**Role.** The tile of a tile grid: the picture, the caption and the actions a tile carries.

**The fork's code.**

```diff
diff --git a/crates/ui-widgets/src/tile_grid/tile.rs b/crates/ui-widgets/src/tile_grid/tile.rs
index 202fd4b8a..60a0c34a9 100644
--- a/crates/ui-widgets/src/tile_grid/tile.rs
+++ b/crates/ui-widgets/src/tile_grid/tile.rs
@@ -305,7 +305,7 @@ pub struct TileStyle {
 }
 
 /// The paper of a page picture (`Tile::page`): a document's blank page, which is data, not a theme surface.
-const PAPER: DataColor = DataColor::new(Color32::WHITE);
+const PAPER: DataColor = DataColor::new(Color32::WHITE); // design: allow[raw-colour/const](paper is document data)
 
 /// A `width` by `height` page centred in `picture` on the knob's shadow, the theme's short hard shadow of a part lifted
 /// just off the surface under it. The page's longer side spans the largest square the picture holds with the shadow,
@@ -554,6 +554,7 @@ impl<'a> Tile<'a> {
     /// that stands for a document size shows it this way (New Document's presets in `PresetCardLook::Page`, a start
     /// page's page templates).
     #[must_use]
+    // design: allow[props/point-size](document dimensions, any unit)
     pub fn page(self, width: f64, height: f64) -> Self {
         self.flat_thumbnail().thumb(ThumbAspect::new(2, 1), move |cx: &ContentPainterCx<'_>, picture: Rect| paint_page(cx, picture, width, height))
     }
```

**What each hunk does.**

1. The paper colour of a page picture carries a `design: allow[raw-colour/const]` marker: the blank page of a document is data, not a theme surface, and a raw white is what the data is.
2. The `page` builder carries a `design: allow[props/point-size]` marker: its width and height are the document's own dimensions, taken in any one unit, and only their shape counts.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-design-lint` (the workspace check, green on 2026-11-26) and `cargo nextest run -p craft-ui-widgets` (green in the same workspace run).

## `crates/zip/src/guard.rs`

**Role.** The ZIP entry-count guard: it refuses a package that declares more entries than its limit, in memory and on a seekable reader.

**The fork's code.**

```diff
diff --git a/crates/zip/src/guard.rs b/crates/zip/src/guard.rs
index 312a3bbaf..ad7eeecb1 100644
--- a/crates/zip/src/guard.rs
+++ b/crates/zip/src/guard.rs
@@ -119,19 +119,18 @@ pub fn check_entry_count_seekable<R: Read + Seek>(reader: &mut R, max_entries: u
             if eocd64 >= locator_pos || disks > 1 {
                 continue;
             }
-            let span = (locator_pos - eocd64).min(65_536);
-            let mut buf;
-            let slice = if let Some(in_tail) = eocd64.checked_sub(tail_start).and_then(|o| usize::try_from(o).ok()) {
-                tail.get(in_tail..locator).unwrap_or_default()
-            } else {
-                let span_usize = usize::try_from(span).unwrap_or(65_536);
-                reader.seek(SeekFrom::Start(eocd64)).map_err(io_err)?;
-                buf = vec![0u8; span_usize];
-                reader.read_exact(&mut buf).map_err(io_err)?;
-                &buf
-            };
-            for (i, _) in slice.windows(4).enumerate().filter(|(_, w)| *w == EOCD64) {
-                refuse(u64_at(slice, i + 24).unwrap_or(0).max(u64_at(slice, i + 32).unwrap_or(0)))?;
+            // The whole `eocd64..locator` range, read in bounded chunks: the record can sit further back than
+            // the tail window when a stub is in front of the package, and `zip` uses it there too.
+            for offset in scan_offsets(reader, eocd64, locator_pos, EOCD64).map_err(io_err)? {
+                let mut head = [0u8; 40];
+                reader.seek(SeekFrom::Start(offset)).map_err(io_err)?;
+                if reader.read_exact(&mut head).is_err() {
+                    continue;
+                }
+                let count = |at: usize| {
+                    u64::from_le_bytes([head[at], head[at + 1], head[at + 2], head[at + 3], head[at + 4], head[at + 5], head[at + 6], head[at + 7]])
+                };
+                refuse(count(24).max(count(32)))?;
             }
             continue;
         }
@@ -140,22 +139,38 @@ pub fn check_entry_count_seekable<R: Read + Seek>(reader: &mut R, max_entries: u
         }
         let cd = u64::from(cd_offset);
         let at_pos = tail_start + at as u64;
-        if cd < at_pos {
-            let has_cd = if let Some(in_tail) = cd.checked_sub(tail_start).and_then(|o| usize::try_from(o).ok()) {
-                tail.get(in_tail..at).is_some_and(|b| b.windows(4).any(|w| w == CENTRAL_HEADER))
-            } else {
-                let mut sig = [0u8; 4];
-                reader.seek(SeekFrom::Start(cd)).map_err(io_err)?;
-                reader.read_exact(&mut sig).is_ok() && &sig == CENTRAL_HEADER
-            };
-            if has_cd {
-                return refuse(u64::from(on_disk.max(total)));
-            }
+        if cd < at_pos && !scan_offsets(reader, cd, at_pos, CENTRAL_HEADER).map_err(io_err)?.is_empty() {
+            return refuse(u64::from(on_disk.max(total)));
         }
     }
     Ok(())
 }
 
+/// The offsets of `needle` in `from..to`, read in bounded chunks: three bytes of each chunk are carried
+/// into the next, so a signature that straddles a boundary is still found. Nothing is buffered beyond one
+/// chunk, and the scan covers the same range the in-memory guard walks.
+fn scan_offsets<R: Read + Seek>(reader: &mut R, from: u64, to: u64, needle: &[u8; 4]) -> std::io::Result<Vec<u64>> {
+    const CHUNK: u64 = 64 * 1024;
+    let mut found = Vec::new();
+    let mut at = from;
+    let mut carry: Vec<u8> = Vec::new();
+    while at < to {
+        let len = usize::try_from(CHUNK.min(to - at)).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
+        let mut buf = vec![0u8; len + carry.len()];
+        buf[..carry.len()].copy_from_slice(&carry);
+        reader.seek(SeekFrom::Start(at))?;
+        reader.read_exact(&mut buf[carry.len()..])?;
+        let base = at - carry.len() as u64;
+        for (i, _) in buf.windows(4).enumerate().filter(|(_, window)| *window == needle) {
+            found.push(base + i as u64);
+        }
+        let keep = buf.len().saturating_sub(3);
+        carry = buf[keep..].to_vec();
+        at += len as u64;
+    }
+    Ok(found)
+}
+
 /// A package of one empty stored file, `a`, whose central directory lists it `entries` times (at
 /// most as many as fit in 64 MiB) under a ZIP64 end record that declares them all: `zip`
 /// parses and keeps every one of them before its caller can count them. Regression input for the
@@ -284,6 +299,27 @@ mod tests {
         assert_eq!(check(&f, 20_000), Err(Error::TooManyEndRecords(16)));
     }
 
+    /// A stub in front of the package moves its end records out of the trailing window: the seekable guard
+    /// reads the same ranges as the in-memory one, so both refuse the declared count.
+    #[test]
+    fn a_prefix_longer_than_the_tail_window_hides_no_end_record() {
+        use std::io::Cursor;
+
+        let mut zipped = vec![0x55u8; 70_000];
+        zipped.extend_from_slice(&zip64_entry_count_sample(20_001));
+        assert_eq!(check(&zipped, 20_000), Err(Error::TooManyEntries { declared: 20_001, limit: 20_000 }));
+        assert_eq!(check_entry_count_seekable(&mut Cursor::new(&zipped), 20_000), Err(Error::TooManyEntries { declared: 20_001, limit: 20_000 }));
+
+        // A plain package whose directory offset points into that stub: the central header is further on.
+        let mut plain_pkg = vec![0x55u8; 10];
+        plain_pkg.extend_from_slice(&plain(30_000, b""));
+        let eocd = plain_pkg.len() - 22;
+        plain_pkg[eocd + 16..eocd + 20].copy_from_slice(&0u32.to_le_bytes());
+        plain_pkg.splice(eocd..eocd, std::iter::repeat_n(0x55u8, 70_000));
+        assert_eq!(check(&plain_pkg, 20_000), Err(Error::TooManyEntries { declared: 30_000, limit: 20_000 }));
+        assert_eq!(check_entry_count_seekable(&mut Cursor::new(&plain_pkg), 20_000), Err(Error::TooManyEntries { declared: 30_000, limit: 20_000 }));
+    }
+
     #[test]
     fn seekable_entry_count_matches_in_memory_guard() {
         use std::io::Cursor;
```

**What each hunk does.**

1. `scan_offsets` reads a range from the reader in bounded chunks and carries three bytes into the next chunk, so a signature that straddles a chunk boundary is still found. Nothing is buffered beyond one chunk, and the range scanned is the one the in-memory guard walks.
2. The ZIP64 end record scan uses `scan_offsets` instead of a 64 KiB tail window, so a stub in front of the package cannot move the end record out of the window the guard read.
3. The central-directory check uses `scan_offsets` as well, so the same range is read the same way on both paths.
4. A new trial builds a package with a 70,000 byte stub in front of it and asserts that both the in-memory and the seekable guard refuse the declared count.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-zip` (green as part of the 18,485-trial workspace run of 2026-11-26).

## `design/exceptions/craft-ui-panels/docking.toml`

**Role.** The design-lint exceptions of the panels crate: the places that may break a rule, and why.

**The fork's code.**

```diff
diff --git a/design/exceptions/craft-ui-panels/docking.toml b/design/exceptions/craft-ui-panels/docking.toml
index 1bfc5dd49..46cc3001c 100644
--- a/design/exceptions/craft-ui-panels/docking.toml
+++ b/design/exceptions/craft-ui-panels/docking.toml
@@ -23,3 +23,11 @@ rule = "literal-size"
 path = "crates/ui-panels/src/docking/iconic/trials.rs"
 reason = "Trial rooms, not looks."
 count = 1
+
+[[exception]]
+name = "iconic-trial-pixel-rounding"
+kind = "marker"
+rule = "test/literal-size"
+path = "crates/ui-panels/src/docking/iconic/trials.rs"
+reason = "Half a point of rounding in a trial's relation between two measured rects, not a token size."
+count = 2
\ No newline at end of file
```

**What each hunk does.**

1. The `iconic-trial-pixel-rounding` entry registers the two `test/literal-size` markers of the iconic trials: half a point of rounding in a trial's relation between two measured rects, which is not a token size. The count is 2, the number of markers in the file.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-design-lint` (the workspace check, green on 2026-11-26): an entry whose count does not match its sites is `exception/stale`, a marker with no entry is `exception/unlisted`.

## `design/exceptions/craft-ui-widgets/tile_grid.toml`

**Role.** The design-lint exceptions of the widgets crate: the places that may break a rule, and why.

**The fork's code.**

```diff
diff --git a/design/exceptions/craft-ui-widgets/tile_grid.toml b/design/exceptions/craft-ui-widgets/tile_grid.toml
index 88f56e78b..370f8db07 100644
--- a/design/exceptions/craft-ui-widgets/tile_grid.toml
+++ b/design/exceptions/craft-ui-widgets/tile_grid.toml
@@ -1 +1,17 @@
 config_version = 1
+
+[[exception]]
+name = "page-picture-paper"
+kind = "marker"
+rule = "raw-colour/const"
+path = "crates/ui-widgets/src/tile_grid/tile.rs"
+reason = "The paper of a page picture is the document's blank page, data rather than a theme surface."
+count = 1
+
+[[exception]]
+name = "page-picture-size"
+kind = "marker"
+rule = "props/point-size"
+path = "crates/ui-widgets/src/tile_grid/tile.rs"
+reason = "A page picture takes the document's own width and height, in any one unit, for its shape only."
+count = 1
\ No newline at end of file
```

**What each hunk does.**

1. The `page-picture-paper` entry registers the `raw-colour/const` marker of the page picture's paper.
2. The `page-picture-size` entry registers the `props/point-size` marker of the `page` builder.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-design-lint` (the workspace check, green on 2026-11-26).

## `docs/components-index.md`

**Role.** The components index: the table of every registered component, generated from the registrations themselves.

**The fork's code.**

```diff
diff --git a/docs/components-index.md b/docs/components-index.md
index a0bbf32a2..a5fef53b3 100644
--- a/docs/components-index.md
+++ b/docs/components-index.md
@@ -64,7 +64,7 @@ Grouped by gallery level, then shape, then name.
 | `Disclosure` | Leaf | `craft_ui_widgets::controls::disclosure` | A button bound to an open state, with the expanded state |
 | `IconButton` | Leaf | `craft_ui_widgets::buttons::icon_button` | A square button showing only an icon |
 | `Kbd` | Leaf | `craft_ui_widgets::display::kbd` | A key chord |
-| `Link` | Leaf | `craft_ui_widgets::buttons::link` | A link: drawn in the accent colour in a type role (body by default) and underlined, so colour is not its only cue |
+| `Link` | Leaf | `craft_ui_widgets::buttons::link` | A link: drawn in the accent colour in a type role (body by default) and underlined, so colour is not its only cue (`LinkLook::Plain` drops the underline of a link standing alone) |
 | `LinkedFields` | Leaf | `craft_ui_widgets::text::linked` | `N` number fields joined by a link toggle |
 | `NumberField` | Leaf | `craft_ui_widgets::text::number` | A number field: a text part over [`NumberCodec`]; empty text is `None` |
 | `PointPicker` | Leaf | `craft_ui_widgets::value_drag::point_picker` | A point `[x, y]` picked on a well: x grows to the right and y downwards, as image coordinates do, in either reading direction |
```

**What each hunk does.**

1. The `Link` row carries the `LinkLook::Plain` note. The file is generated, so the change comes from `cargo run -p craft-ui-gallery --bin components-index` and not from an edit.

**If this were reverted / how it is checked.** `cargo nextest run -p craft-ui -E 'test(components_index)'` (green on 2026-11-26): the trial compares the committed file with what the registrations give.

