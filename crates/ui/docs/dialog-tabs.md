# Dialog tabs that join their page

`Tab::show_dialog_at` adds the page-joining appearance proposed in
[WordCraft #4029](https://github.com/storytold/wordcraft/pull/4029), using the library's existing
accessible tab interaction. It suits settings and property dialogs across Craft apps.

Provide a `DialogTabStyle` with the application's colors, border, font and top corner radius.
Allocate rectangles using the application's layout and optionally `tabs::fit`; draw a baseline
before the tabs. Selected tabs use `style.page` and erase their bottom border so they join the
page. Use `style.border` for the baseline centered on the bottom edge; the selected cover extends
half its width below the interaction rectangle. Other sides and text retain their original clip.
The page should start at the tabs' bottom edge and use the same fill.

```rust
let response = craft_ui::tabs::Tab::new(stable_id, localized_label, selected)
    .show_dialog_at(ui, rect, &style);
if response.clicked() {
    // Update the application's selected page.
}
```

IDs must remain stable independently of translated labels. Selection, overflow, arrow-key
navigation and page content remain caller-owned. The underlying `Tab` supplies accessibility,
keyboard focus/activation, drag response and disabled behavior. Text clips to its rectangle;
invalid stroke widths become zero and invalid font sizes use 12 points. No app assets or theme
are bundled, and consumer migration is separate.
