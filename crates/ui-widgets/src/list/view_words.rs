//! Built-in words of `FindingsList`, `EventList` and `UndoList`.

craft_ui::words! {
    /// The words of the findings summary.
    pub struct FindingsWords {
        no_errors: "findings.noErrors" = "No errors";
        info: "findings.info" = "Info";
        warning: "findings.warning" = "Warning";
        error: "findings.error" = "Error";
        counts: template "findings.counts" = "{errors}, {warnings}";
        errors: count "findings.errors" = "{n} error" | "{n} errors";
        warnings: count "findings.warnings" = "{n} warning" | "{n} warnings";
    }
}

craft_ui::words! {
    /// The words of the event list: level names, row text and the count beside each level.
    pub struct EventWords {
        info: "eventLog.info" = "Info";
        warning: "eventLog.warning" = "Warning";
        error: "eventLog.error" = "Error";
        /// An event that happened several times in a row: `{message}`, `{count}`.
        repeated: template "eventLog.repeated" = "{message}  (\u{d7}{count})";
        /// A row's accessible name, which starts with the message the row shows: `{message}`, `{level}` and `{age}`.
        row_name: template "eventLog.rowName" = "{message}, {level}, {age}";
        /// The details line over the selected event's message: `{level}`, `{source}` and `{time}`.
        details: template "eventLog.details" = "{level} \u{b7} {source} \u{b7} {time}";
        none: "eventLog.none" = "No events.";
        error_count: count "eventLog.errorCount" = "{n} error" | "{n} errors";
        warning_count: count "eventLog.warningCount" = "{n} warning" | "{n} warnings";
        message_count: count "eventLog.messageCount" = "{n} message" | "{n} messages";
    }
}

craft_ui::words! {
    /// The words of the undo list.
    pub struct UndoWords {
        /// The name of the list of actions.
        list: "undoList.list" = "Undo History";
        cancel: "undoList.cancel" = "Cancel";
        actions: count "undoList.actions" = "Undo {n} Action" | "Undo {n} Actions";
    }
}
