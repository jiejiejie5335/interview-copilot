use std::sync::Mutex;

use serde::Serialize;
use tauri::State;

pub const GLOBAL_ANSWER_SHORTCUT: &str = "CommandOrControl+Shift+Enter";
const GLOBAL_ANSWER_SHORTCUT_LABEL: &str = "⌘ + Shift + Enter";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalShortcutStatus {
    accelerator: &'static str,
    label: &'static str,
    registered: bool,
    error: Option<String>,
}

impl Default for GlobalShortcutStatus {
    fn default() -> Self {
        Self {
            accelerator: GLOBAL_ANSWER_SHORTCUT,
            label: GLOBAL_ANSWER_SHORTCUT_LABEL,
            registered: false,
            error: None,
        }
    }
}

#[derive(Default)]
pub struct GlobalShortcutRuntime {
    status: Mutex<GlobalShortcutStatus>,
}

impl GlobalShortcutRuntime {
    pub fn set_registration_result(&self, result: Result<(), String>) {
        let mut status = self
            .status
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        status.registered = result.is_ok();
        status.error = result.err();
    }

    fn status(&self) -> GlobalShortcutStatus {
        self.status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

#[tauri::command]
pub fn get_global_shortcut_status(
    runtime: State<'_, GlobalShortcutRuntime>,
) -> GlobalShortcutStatus {
    runtime.status()
}

#[cfg(test)]
mod tests {
    use super::GlobalShortcutRuntime;

    #[test]
    fn records_registration_failure_without_panicking() {
        let runtime = GlobalShortcutRuntime::default();
        runtime.set_registration_result(Err("快捷键已被占用".to_string()));
        let status = runtime.status();
        assert!(!status.registered);
        assert_eq!(status.error.as_deref(), Some("快捷键已被占用"));
    }
}
