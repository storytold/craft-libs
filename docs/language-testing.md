# Language testing per release

A manual checklist for each app release: input methods, keyboard layouts and screen readers on
real systems. Copy it into the release issue and tick each item; for a failure, note the system,
its version and what happened instead, and open an issue.

Run every input item in two places: a single-line text field (a rename or search field) and the
app's canvas text tool, if it has one. "Commits" means the text appears in the field as ordinary
text, once, at the caret, with the composition underline gone.

## What is automated

Leave these to the automated tests; repeat them by hand only to investigate a failure.

| Automated | Where |
|---|---|
| Linux input methods on this box: fcitx5 on Xvfb (XIM), fcitx5 on headless sway (text-input-v3) and IBus on Xvfb; Pinyin, Hangul, Telex and Anthy compositions, a click into a composing field, a pointer unplugged while composing | [IME harness](https://github.com/storytold/craft-libs/pull/212), `tools/ime-harness/` |
| Each app's key screens in Arabic, Japanese, Traditional Chinese, Korean, Thai and Vietnamese with no system fonts: no missing glyph and no widget past the screen; one scripted IME composition and one non-Latin shortcut per app | `craft_ui::testing` ([craft-ui README](../crates/ui/README.md#testing-screens-in-each-language)) |
| The shortcut rule for Russian, Arabic 101, Hebrew, Greek, Thai, AZERTY, Dvorak and QWERTZ presses as egui reports them | craft-keymap's tests |

The manual list covers what those can't reach: the input methods and layouts of Windows and
macOS, desktop Linux sessions, the browser on phones, and screen readers.

## Release

- App and version:
- Tester and date:
- Builds tested (Windows, macOS, Linux, Flatpak, FreeBSD, web):

## Windows

- [ ] **Microsoft Pinyin.** Type `nihao`, then Space. The candidate window opens at the caret;
      你好 commits. Type `nihao` and press Enter: `nihao` commits as Latin text.
- [ ] **Microsoft Japanese IME.** Type `nihongo`, then Space. にほんご converts to 日本語 with
      the converted segment underlined; Enter commits 日本語. Type `nihongo` again and press
      Escape twice: the composition is removed, nothing is inserted, and the dialog or panel stays
      open.
- [ ] **Microsoft Korean IME** (2-Set). Switch with the Han/Eng key or Right Alt. Type `gksrmf`:
      the syllables build in place and 한글 commits. While composing 한, Backspace leaves 하.
      Press Enter after a syllable: the syllable commits and the field does not also submit; a
      second Enter does.
- [ ] **Sogou Pinyin.** Type `zhongguo`, then Space: 中国 commits, with Sogou's candidate bar at
      the caret. In Chinese mode, `,` types the full-width `，`.
- [ ] **UniKey** (Telex). Type `Vieejt Nam`: Việt Nam commits with no doubled or missing letters.
      Type `tieesng` in the canvas text tool: tiếng.
- [ ] **Emoji panel.** Press Win+. and pick 😀: it is inserted once, at the caret. Pick a second
      emoji with the panel still open: it follows the first.
- [ ] **AltGr layouts.** On German, AltGr+Q types `@` and AltGr+E types `€`; on Polish
      (programmer's), AltGr+A types `ą`. None of these runs a Ctrl+Alt shortcut.
- [ ] **US-International dead keys.** Type `'` then `e`: é. Type `'` then Space: `'`.

## macOS

- [ ] **Pinyin – Simplified.** Type `nihao`, then Space: the candidate window opens at the caret
      and 你好 commits.
- [ ] **Japanese – Romaji (Kotoeri).** Type `nihongo`, then Space: 日本語 is shown with the
      active segment underlined more thickly; Return commits. Escape during a composition cancels
      it without closing the dialog.
- [ ] **2-Set Korean.** Type `gksrmf`: 한글 builds in place and commits. Return after a syllable
      commits it without submitting the field; a second Return submits.
- [ ] **Press and hold.** Hold `e`: the accent menu opens at the caret; press `2`: é replaces the
      e. The plain `e` is not left behind.
- [ ] **Dead keys** (U.S. layout). Option+E then `e`: é. Option+U then `u`: ü. Option+N then `n`:
      ñ. The pending accent shows as marked text until the second key.
- [ ] **Character Viewer.** Press Ctrl+Cmd+Space and pick 😀: it is inserted once, at the caret.
- [ ] **Dictation.** Start dictation (press Fn twice, or the microphone key) and say a short
      sentence: interim text appears at the caret; stopping dictation leaves the sentence once.

## Linux

- [ ] **GNOME with IBus** (Wayland session). With ibus-libpinyin, `nihao` Space: 你好. With
      ibus-hangul, `gksrmf`: 한글. With ibus-mozc or ibus-anthy, `nihongo` Space Enter: 日本語.
      The candidate popup opens at the caret.
- [ ] **KDE Plasma with Fcitx5** (Wayland session). The same three compositions through
      fcitx5-chinese-addons, fcitx5-hangul and fcitx5-mozc. The preedit shows in the field while
      composing (designcraft#1234).
- [ ] **Hyprland with Fcitx5.** The same three compositions; the candidate popup follows the
      caret when the caret moves to a second line.
- [ ] **X11 with Vietnamese Telex.** In an X11 session with fcitx5-unikey or ibus-unikey, type
      `tieesng Vieejt`: tiếng Việt, with no doubled letters.
- [ ] **One Flatpak build.** Install the app's Flatpak and repeat one composition (Pinyin or
      Hangul) through the host's input method. Switch the UI to Japanese and to Arabic: every label
      draws without boxes, with only the runtime's fonts.

## FreeBSD (once per release cycle)

- [ ] Build and start the app. Under X11 with fcitx5 or IBus, compose `nihao` → 你好 and
      `gksrmf` → 한글. Switch the UI to Arabic: labels join and run right to left.

## Web

- [ ] **Android, Chrome, Gboard.** Type `hello wrold` with suggestions on: the composing word is
      underlined, and the autocorrected `world` replaces it without duplicating text. Backspace
      deletes one character at a time. Switch Gboard to Korean and type 한글: syllables build in
      place.
- [ ] **iOS, Safari, Korean keyboard.** Type 한글: syllables build in place with no doubled jamo;
      tapping elsewhere in the text commits the syllable and moves the caret there.

## Keyboard shortcuts on other layouts

For each layout, run Copy, Paste, Undo, Select All and Save from the keyboard, and the shortcuts
named below. Each should run the same command it runs on US QWERTY, and the menus should show the
shortcut as listed. On macOS read Ctrl as Cmd.

- [ ] **Russian.** Ctrl on the C, V, Z, A and S keys (с, м, я, ф, ы) copies, pastes, undoes,
      selects all and saves. The menu shows Ctrl+[ as Ctrl+Х.
- [ ] **Arabic 101.** Ctrl on the C, V, Z, A and S keys (ؤ, ر, ئ, ش, س) runs the same commands.
- [ ] **Hebrew.** Ctrl on the C, V, Z, A and S keys (ב, ה, ז, ש, ד) runs the same commands. Ctrl on
      the Q key, which types `/`, runs the Ctrl+Q command, not Ctrl+/.
- [ ] **Greek.** Ctrl on the C, V, Z, A and S keys (ψ, ω, ζ, α, σ) runs the same commands. In a
      text field, `;` then `α` types ά.
- [ ] **Thai** (Kedmanee). Ctrl on the C, V, Z, A and S keys (แ, อ, ผ, ฟ, ห) runs the same
      commands. Ctrl on the 1 key, which types ๅ, runs Ctrl+1.
- [ ] **AZERTY** (French). Ctrl+A and Ctrl+Z are the keys labelled A and Z. Ctrl on the 1 key
      (`&`) runs Ctrl+1. Ctrl on the 6 key (`-`) zooms out.
- [ ] **Dvorak.** Ctrl+C, Ctrl+V and Ctrl+Z are the keys labelled C, V and Z. Ctrl on the QWERTY
      Q position, which types `'`, does not run Ctrl+Q.
- [ ] **QWERTZ** (German). Ctrl+Z undoes and Ctrl+Y redoes, on the keys labelled Z and Y. Ctrl+⇧7
      runs Ctrl+/. The menu shows Ctrl+[ as Ctrl+Ü.
- [ ] **Dvorak – QWERTY ⌘** (macOS). Typing gives Dvorak letters. With Cmd held, the keys follow
      QWERTY: Cmd on the QWERTY C position copies, and the menus show ⌘C.

## Screen readers

Run each with the UI in English, then in Arabic. In Arabic, use a voice that reads Arabic.

- [ ] **NVDA** (Windows). Tab through the main window and one dialog: each control is announced
      with its name and role, in the UI language, in the order it appears (right to left in
      Arabic). In a text field, typed characters are echoed and arrow keys read the character
      passed over. An Arabic file name in an English UI is read in Arabic.
- [ ] **VoiceOver** (macOS). Move with VO+Right through the toolbar and a dialog: names and roles
      in the UI language, in reading order. The rotor lists the form controls. Composing Hangul in
      a text field is announced once, when it commits.
- [ ] **Orca** (Linux, GNOME). Tab through the main window and one dialog: names and roles are
      read in the UI language. Focus moves visibly with each announcement.
