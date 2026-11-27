# craft-plugin

`craft-plugin` is the add-in contract the Craft apps share. An add-in is a separate program in any
language, described by a `plugin.json` manifest; an app starts it and talks JSON-RPC 2.0 with it
over its standard input and output, one message per line. One add-in can serve several apps, so
the parts that must agree between apps live here:

- `Manifest` reads `plugin.json` for one app (`Host`: its id, name and right-click menus). `apps`
  says which apps an add-in is made for, and `uiFor.<app>` replaces `ui` for that app. Every list
  and label is bounded; a menu the app doesn't have is an error.
- `version` and `requires` hold versions, ranges, and `resolve`, which takes a snapshot of every
  installed add-in and the host and returns which are blocked and why, the start order and who
  needs whom. A range that doesn't parse blocks the add-in; it never fails the load.
- `github` reads the GitHub addresses add-ins are installed from and picks the release asset that
  suits a platform. It does not download or unpack anything.
- `Connection` starts the process (`Launch`) and sends requests. Reading and writing run on their
  own threads: a late answer, an exit, a line over 64 MiB or a stream error ends the request with
  a fatal `RequestError`, never a hung app. `request_with` answers the add-in's own requests
  while it works; `request_bounded` chooses whether those calls extend the deadline.
- `push` queues what add-ins say on their own (at most 20 000 messages, 64 MiB a line, and an
  add-in that sends over 1 000 messages a second for 5 seconds is cut off) for the app to take in
  its main loop.

The app keeps what only it knows: the registry of loaded add-ins, what a function or a command
does, its menus and ribbon, downloading and unpacking, and its documents.

On `wasm32` the types exist and `Connection::start` returns an error: a browser can't start
processes.
