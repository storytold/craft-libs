# Contextual keytip allocation

`craft_keymap::keytips::allocate` shares the collision-free letter allocation from
[WordCraft #3855](https://github.com/storytold/wordcraft/pull/3855). It is useful for contextual
ribbon tabs and menus in other Craft apps while leaving their command catalogs and Alt-key
handling in each host.

Pass labels in stable order and reserve every fixed tab/search/legacy-menu prefix. Multi-key
sequences reserve their first letter, so a generated single-letter tip cannot shadow them.
Allocation prefers an available letter from the label, then the first free A-Z letter.

Allocate over all possible contextual tabs once, then display the active subset. This keeps
letters stable as selections change. Stable source labels or localized labels are both possible
host policies; non-Latin labels use the A-Z fallback. No transliteration is attempted. Exhausted
letters return `None`, meaning the control stays usable without a generated keytip.

The result covers at most 4096 labels and reports omitted choices; at most 256 characters per
label are scanned. Reserved prefixes are all inspected. This adds no global state, bindings or
app-specific legacy key reservations, and it does not change existing `Keymap` shortcut dispatch.
