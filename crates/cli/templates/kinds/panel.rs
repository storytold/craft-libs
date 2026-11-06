
    /// Runs when the panel opens and when the user touches a widget. Returns the widget tree to show.
    /// The click count lives in the host's per-plugin store, so it survives a reload.
    fn open_panel(event: PanelEvent) -> Option<Widget> {
        let stored = state::get_as::<u64>("clicks").ok().flatten().unwrap_or(0);
        let clicks = if event.event == "action" && event.widget.as_deref() == Some("bump") {
            let next = stored + 1;
            if let Err(e) = state::put("clicks", &next) {
                host::error(&format!("cannot save the count: {e}"));
            }
            next
        } else {
            stored
        };
        Some(
            ui::column("root")
                .child(ui::label("count", &format!("Clicked {clicks} times")))
                .child(ui::button("bump", "Click me")),
        )
    }
