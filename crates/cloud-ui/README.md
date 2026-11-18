# craft-cloud-ui

The share dialog, the cloud Open and Save pages, the account menu and the presence bar that every
Craft app shares. This is storytold main's cloud and share UI as main wrote it in craft-ui: raw egui,
outside the craft UI library's component contract, to be ported onto the framework's components. The
`craft-ui` facade re-exports `account`, `cloud`, `share` and `presence` from here behind the same
`share` and `cloud` features.

## Sharing

`share::ShareDialog` is a whole dialog. It holds a recipient picker with autocomplete and
removable chips. It offers a view-or-edit choice, a note and a notify-by-email box. Under those it
lists who already has access, with a withdrawal beside each entry. Every Craft app gets the same
dialog. An application's own part comes to three lines.

```rust
// Once, at startup. The host supplies the half that reaches the service.
app.share.connect(my_backend);

// Every frame, open or shut. This is also where answers are collected.
app.share.show(ctx);

// From the app's Share control, drawn only while `available()` is true.
if app.share.available() && share_button.clicked() {
    app.share.open(entry.id.clone(), entry.name.clone());
}
```

The dialog opens no socket, names no provider and holds no credential. It states each piece of
work as a `share::Request` and hands it to a `share::ShareBackend`. That backend answers through a
`share::Replier` whenever it is ready. Nothing in this crate implements that trait. A host writes
the half that opens a socket, and this crate never learns what it reaches. The interface half
therefore builds for the web, and every flow is testable with no server running.

Autocomplete waits `share::DEBOUNCE` after the last keystroke before it asks anything. An answer
to a query the user has already typed past is dropped before it reaches the list. A list that
flickers back to stale matches is worse than one that arrives late.

Partial success is rendered as partial success. A share aimed at five people with one wrong
address reaches four. The dialog names the fifth with the reason the service gave. It keeps that
one recipient in the picker. A correction then takes one edit. `outcome()` hands back both halves.

Capability is reported and never assumed. `available()` stays false until the service says the
provider supports sharing and an account is linked. `open` does nothing while it is false. An
application draws no Share control in that state. A hidden feature beats a button that fails on
use. Contacts and mail are reported separately. A server with no address book accepts typed
addresses only. A server that cannot send mail disables the notify box and says why.
`ShareLabels` holds every string the dialog shows. The defaults are English. A localized
application replaces them without touching this crate.

A contact's `avatar` is a URL. The dialog asks for the bytes through the same backend, as the row
that needs the picture is painted and only while that row is on screen. A search answering with
ten people costs no pictures until somebody looks at them. Each URL is asked about once. A refusal
is remembered, with initials painted in its place. The host decides whether a picture may be
fetched at all. A backend that refuses `share::Request::Avatar` gets initials painted instead, and
a host that would rather not have an application reach a provider's avatar URL directly can say
so by refusing it. A host that relays pictures answers it. Nothing else changes.

### The account

`account::AccountMenu` is the badge every suite puts in the top right. It shows who is signed in,
where they are signed in, and whether storage is linked. It holds no credential. It asks the
service who the caller is, through the application. It reports what the user asked for through
`take_action`. The application owns what signing in means.

Without linked storage, Open and Save have nowhere to go. The menu says so at rest. A failure at
the moment of saving arrives too late to be useful.
