# Contextual tab entry and exit

`tabs::TabPresence` shares the contextual tab lifecycle from
[WordCraft #3131](https://github.com/storytold/wordcraft/pull/3131). Each app still decides which
tabs apply to its current selection. Supply stable IDs and strip positions to `update`, along
with monotonic seconds, duration (for example 0.35) and the host's motion setting.

The returned frames provide eased visibility, position and whether the tab is present. Initial
tabs appear immediately. New tabs enter from zero visibility; removed tabs remain as outgoing
paint-only ghosts until their fade completes. Reappearing tabs reverse from their current
visibility without duplicate copies. Invalid time/duration, rollback and motion-off settle
current tabs and discard ghosts immediately. Duplicate IDs use their first position.

Use visibility for opacity and optionally `(1.0 - visibility) * distance` for an entry offset.
Request repaint while any frame is moving. Keep label/rectangle data needed for outgoing
painting in the host. **Only present tabs may register input, commands or keytips**. Selection
fallback, overflow, localization, clipping and document semantics remain app-owned. This helper
does not make a disappearing selected tab applicable to the document.
