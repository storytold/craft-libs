# Craft Proof UI

`craft-proof-ui` is the spelling review for the Craft apps: a panel that steps through a document's
issues beside the page, and a section for the right-click menu on a misspelled word. The application
supplies its documents through `Host`; findings usually come from `craft-proof`, which this crate
does not depend on.

## Boundaries

- The review and its interface only. The app owns proofing, its text model, applying corrections and
  undo, where the panel docks, and how it opens.
- Dependencies: `egui`.
- Never crashes: no panics, no `unsafe`. A failed host call becomes a message in the panel.
- The document stays editable during a review, so nothing is trusted to still be true: a correction
  is checked against a fresh `Host::check`, and text that changed asks for a new pass.
- Suggestions are requested for the current issue only, never for the rest of a book's findings.

## Use

```rust,ignore
use craft_proof_ui::{Colors, Host, Review, context_section, panel};

// Keep a `Review` while the panel is open. `start` checks the document from the top.
let mut review = Review::default();
review.start(&mut host);
// Each frame, inside the app's dock:
panel(ui, &mut review, &mut host, &text, &colors);
// On a right-click the app finds the issue under the caret, once. Then, in its menu:
if let Some(chosen) = context_section(ui, &issue, &text) { /* act on it, close the menu */ }
```

`STRINGS` lists every string looked up through `Text::ui`, for the app's translation catalog.
