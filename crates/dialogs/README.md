craft-dialogs
=============

Native file and message dialogs that never block the event loop. A request is queued, shown at the
end of the frame on the main thread, and answered on a later frame. No egui dependency: the parent
is any `raw-window-handle` 0.6 window, and the app passes a closure that wakes its event loop.

## Why blocking dialogs crash

On macOS a blocking `rfd::FileDialog` or `rfd::MessageDialog` runs AppKit's modal loop
(`-[NSSavePanel runModal]`) inside winit's event handler. Events that arrive while that loop runs
(a file dragged from Finder and held over the window, the panel being resized) re-enter the
handler. winit panics with "tried to handle event while another event is currently being
handled", the panic can't unwind out of the AppKit callback, and the app aborts.

## API

```rust
pub type Wake = Arc<dyn Fn() + Send + Sync>;
pub type Override = Box<dyn FnMut(&Request) -> Answer>;

pub enum Kind { Open { multiple: bool }, Folder, Save }
pub struct FileDialog { pub kind: Kind, pub title: Option<String>, pub filters: Vec<Filter>,
    pub directory: Option<PathBuf>, pub file_name: Option<String> }
pub struct Filter { pub name: String, pub extensions: Vec<String> }
pub struct Message { pub level: MessageLevel, pub title: String, pub description: String,
    pub buttons: MessageButtons }
pub enum Request { File(FileDialog), Message(Message) }
pub enum Answer { Cancelled, Paths(Vec<PathBuf>), Message(MessageResult), Unavailable(String) }

impl<T> Dialogs<T> {
    pub fn new(wake: Wake) -> Self;
    pub fn ask(&mut self, dialog: FileDialog, then: T);
    pub fn ask_message(&mut self, message: Message, then: T);
    pub fn show_pending<W: HasWindowHandle + HasDisplayHandle>(&mut self, parent: Option<&W>);
    pub fn take_answers(&mut self) -> Vec<(T, Answer)>;
    pub fn is_open(&self) -> bool;
    pub fn raise(&self);
    pub fn queued(&self) -> usize;
    pub fn set_override(&mut self, answer_with: Option<Override>);
}
```

`FileDialog` has builders: `FileDialog::open().title("Place").filter("Images", &["png", "jpg"])`,
and `open_many`, `folder`, `save`, `directory`, `file_name`.

In an eframe app:

```rust
// Once, at startup.
let ctx = cc.egui_ctx.clone();
let dialogs = Dialogs::new(Arc::new(move || ctx.request_repaint()));

// A command that needs a file.
self.dialogs.ask(FileDialog::open().filter("PDF", &["pdf"]), Then::Open);

// Every frame: answers first, then show what is queued, with the window as parent.
for (then, answer) in self.dialogs.take_answers() {
    match answer {
        Answer::Paths(paths) => self.run(then, paths),
        Answer::Unavailable(why) => self.report_error(why),
        Answer::Cancelled | Answer::Message(_) => {}
    }
}
self.dialogs.show_pending(Some(frame));
```

- `then` is the app's continuation: a command to re-run with the path, a panel field to fill. It
  stays on the main thread, so it needs no `Send`.
- One dialog shows at a time. A request made while one is open waits and shows when it closes, in
  the order asked.
- A dialog whose worker ends without answering (it panicked, or its thread couldn't start)
  answers `Unavailable`. A panic while showing one answers `Unavailable` too, instead of unwinding
  into winit's handler. Under `panic = "abort"` a panic while showing aborts the app; the crate
  checks what rfd would otherwise panic on (on macOS, a parent view that isn't in a window) before
  it calls rfd.
- `Unavailable` means no dialog could be shown; report it as an error, not as Cancel.
- `set_override` answers every request without a native dialog, for tests and automation. The
  answers still arrive through `take_answers` after `show_pending`.
- Filter extensions match in any letter case on every platform. On Linux and the BSDs, where rfd
  turns each extension into a case-sensitive `*.ext` glob, each one is also passed as `EXT` and
  `[eE][xX][tT]`.

## Presentation

| Dialog | macOS | Windows, Linux, BSD |
| --- | --- | --- |
| Open, Folder | free-standing panel that doesn't block the window | modal owned by the parent |
| Save | sheet on the parent window | modal owned by the parent |
| Message | sheet on the parent window | modal owned by the parent |

On macOS, Open and Folder follow native apps: `-[NSOpenPanel beginWithCompletionHandler:]`
shows a panel the user can move away from the window, and the window keeps working. A sheet stays
centred on its window when the window is resized, which feels wrong for Open. rfd 0.17.2 can't
show this panel: its async dialogs always begin a sheet, on the main or first window when no
parent is given (`backend/macos/modal_future.rs:71-102`). The crate shows it with objc2-app-kit's
safe bindings. A request made while the panel is open brings the panel to the front.

Save and message boxes are sheets through rfd's async dialogs. rfd falls back to the blocking
`runModal` when the app isn't running or has no window (`modal_future.rs:79-118`), so on macOS
they start only with a parent in a running app, and answer `Unavailable` otherwise. Dialogs needed
before the event loop starts or after it ends (startup and crash messages) stay blocking `rfd`
calls in the app.

## Follow-ups

- Linux portal Save (`org.freedesktop.portal.FileChooser.SaveFile`), so the file type the user
  picks sets the extension (PhotoCraft, LightCraft, WordCraft have it).
- Missing-portal probe: tell the user what to install when Linux has no desktop portal and no
  zenity, instead of an empty answer that looks like Cancel.
- A dialog that never answers (a hung portal) keeps `is_open()` true and the queue waiting. A
  timeout or a way to give up on it would free the app.
- wasm: the same API, answering with file contents, without `std::thread`.
- Android.
