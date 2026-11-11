# Dialogs

`craft_ui::dialog` holds the dialog shells, the prompts, and the host that runs an app's dialogs and keeps their answers.

| Type | Is |
| --- | --- |
| `Dialog` | A dialog on its own layer: modal or modeless, movable by its title band, with a focus trap and a footer of buttons. |
| `DialogFrame` | The chrome every shell shares: title row, close action, footer region. |
| `FullWindowDialog` | A dialog over the whole window. |
| `BlockingOverlay` | A modal layer with no title, for work the window must not interrupt. |
| `Prompt` | A question with answers on the modal preset of `Dialog`. |
| `Alert`, `StandardPrompt`, `UnsavedChangesDialog`, `RecoveryDialog` | Configurations of `Prompt`. |
| `DialogHost` | Runs the open dialogs of a window and keeps their results. |
| `DialogStack` | Which open modal dialog takes the input. |

Form dialogs (`ParamDialog`, image size, properties, new document and others) build on these in `craft_ui::patterns::form_dialogs`.

## Dialog

`Dialog` is a container: build it, then `show(ui, body)`.

```rust
let shown = Dialog::new("Layer Properties")
    .opened(Bind::bound(&mut state.properties_open))
    .remember(PROPERTIES_KEY)
    .close_on_enter(true)
    .buttons([FooterButton::new("OK", DialogRole::Accept), FooterButton::new("Cancel", DialogRole::Cancel)])
    .show(ui, |ui, cx| properties_body(ui, cx, &mut state.layer));
if shown.output.picked == Some(0) {
    state.apply_properties();
}
```

- Open state: `opened` binds it to the app's value; without it the dialog keeps a session value that starts open. The close action, Escape and `BodyCx::request_close` write `false`.
- Buttons: each `FooterButton` has a `DialogRole` (`Accept`, `Destructive`, `Cancel`, `Other`) that sets its look and its place in the row: the accepting button is last on macOS and Linux and first on Windows. A click closes the dialog unless the button `stays_open`. `FooterButton::letter` gives it an access letter.
- Enter: with `close_on_enter`, Enter activates the enabled `Accept` button, so Enter and a click take the same path. Without such a button, Enter closes the dialog with `CloseReason::Enter`.
- Output: `DialogOutput::closed` is the `CloseReason` in the pass the dialog closed, `picked` is the index of the button clicked or activated, and `result(answer)` turns them into a `DialogResult`.
- Hooks: `close_guard` may refuse the close of a picked button, `on_pick` acts on a pick before the close is decided, and `settle_buttons` sets the buttons' state each pass after the body ran.

Other settings:

| Setting | Effect |
| --- | --- |
| `Dialog::prompt` | A prompt: centred, content capped, fixed in place, not remembered. |
| `modeless` | A tool window: no modal layer and no backdrop. |
| `viewport` | Shows the dialog in its own OS window, with the layer as the fallback where windows are unavailable. |
| `resizable` | Edge grips resize it; the size is saved with `remember`. |
| `remember` | Saves placement and size under a `SaveKey`. |
| `initial_focus` | What takes focus when it opens (`runtime::focus::Initial`). |
| `backdrop` | What is drawn behind a modal dialog (`runtime::overlay::Backdrop`). |
| `outer_gesture` | One undo step over the body's edits from open to close: OK commits it, every other close takes the edits back. Modal dialogs only. |
| `footer` | Content in a footer bar above the buttons. |

A modal dialog blocks input below it in every window, and a press outside does nothing.

## DialogHost

An app usually leaves its dialogs to the window's host. The `AppShell` of each window keeps one `DialogHost` and runs it after the body in every pass. A body reaches it with `BodyCx::dialogs`, which returns a `WindowDialogs` handle:

```rust
if let Some(dialogs) = cx.dialogs() {
    dialogs.with(|host| {
        if delete_asked {
            host.open(StandardPrompt::confirm_delete([name], 1));
        }
        if let Some(DialogResult::Ok(answer)) = host.take::<StandardPrompt>() {
            if answer.choice == PromptChoice::Proceed {
                document.delete_layer();
            }
        }
    });
}
```

A dialog on the host is a `StackDialog`: the app builds the value, `open`s it, and reads `take::<D>()` on a later pass. `StackDialog::show` shows one pass and returns `Some(DialogResult)` on the pass it finished; the host then removes it and keeps the result. `DialogResult<T>` is `Ok(T)` or `Cancel`.

Rules of the host:

- One modal at a time: the newest open dialog takes the input. `open_child` opens a dialog over its parent; it closes with the parent.
- One open dialog per type and `StackDialog::instance`. Opening a second one cancels the first and drops its untaken result. `open_unless_open` refuses while one is open.
- A result stays until it is taken or a dialog of the same type and instance opens.
- `enqueue` queues a startup modal by `StartupStage` (recovery, first-launch offer, warning, update). Each opens when the stack is empty, one at a time.
- `cancel::<D>()` closes a dialog from outside as Escape would, and `StackDialog::cancelled` lets it release what it holds.
- `get`, `get_mut` and `confirm::<D>()` reach the open dialog's value, so a script or a test sets fields and answers OK as a user would.
- `take_keyed` returns the result with `StackDialog::key`, so a Cancel still names the command it answers.
- While the host runs its dialogs, or inside a `with` closure, `BodyCx::dialogs` returns `None`. A dialog that the host runs opens children through its own `DialogHost` value.

## DialogStack

`DialogStack` is the routing stack: `push(id, parent)`, `remove`, `remove_tree` (a parent and every dialog opened from it), `top`, `is_top`, `has_child`. `dialog_id(key)` gives a dialog its stable `DialogId`. The host keeps one; an app that shows a `Dialog` itself beside the host reads it with `DialogHost::stack`.

## Prompts

`Prompt::new(title, message)` takes `answers` (`FooterButton`s), and optionally `paragraph`, `lines`, `details` and `extras`. Escape and the close action give the escape answer: Cancel, else the first Other answer, else the default answer. Enter gives the Accept answer while that button is enabled. Both come back as `PromptOutput::picked`.

`StandardPrompt` is the set of confirmations every app asks the same way. Each one has the platform's button order and a letter on every answer, and Escape cancels:

| Constructor | Asks |
| --- | --- |
| `confirm_delete(names, count)` | Delete the items? |
| `changed_on_disk` | The document's file changed on disk: Replace or Cancel. |
| `replace_file` | A save would overwrite a file the save dialog did not ask about. |
| `trash_mask` | Trash a layer mask: Apply, Delete or Cancel. |
| `custom` | The app's own message and buttons. |

`dont_show_again` adds a check box. The answer is a `PromptAnswer`: its `choice` (`PromptChoice::Proceed`, `Alternate` or `Cancel`), `skip` (the box was ticked and the choice is Proceed; the app stores it) and the `tag` the app attached to say which question it answers.

`Alert` is a notice with one OK. `UnsavedChangesDialog` asks Save, Don't Save or Cancel for one document and answers an `UnsavedAnswer`. `RecoveryDialog` offers to recover documents an earlier run autosaved, and answers a `RecoveryAnswer`.
