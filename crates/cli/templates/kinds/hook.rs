
    /// Runs after every app command. An `after` hook cannot change the command, so the verdict is ignored.
    fn log_commands(hook: HookCall) -> HookVerdict {
        if let Some(call) = &hook.call {
            host::info(&format!("{} finished: {}", call.id, hook.outcome.as_deref().unwrap_or("unknown")));
        }
        HookVerdict::Pass
    }
