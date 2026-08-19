use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{Manager, PhysicalPosition, WebviewWindow, Window, WindowEvent};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WindowPosition {
    pub x: i32,
    pub y: i32,
}

impl From<PhysicalPosition<i32>> for WindowPosition {
    fn from(position: PhysicalPosition<i32>) -> Self {
        Self {
            x: position.x,
            y: position.y,
        }
    }
}

impl From<WindowPosition> for PhysicalPosition<i32> {
    fn from(position: WindowPosition) -> Self {
        PhysicalPosition::new(position.x, position.y)
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowPreferences {
    pub always_on_top: bool,
    pub locked: bool,
    pub position: Option<WindowPosition>,
}

#[derive(Default)]
pub struct WindowRuntime {
    locked_position: Mutex<Option<WindowPosition>>,
}

impl WindowRuntime {
    fn set_locked_position(&self, position: Option<WindowPosition>) -> Result<(), String> {
        *self
            .locked_position
            .lock()
            .map_err(|_| "窗口位置锁已损坏".to_string())? = position;
        Ok(())
    }

    fn locked_position(&self) -> Option<WindowPosition> {
        self.locked_position.lock().ok().and_then(|value| *value)
    }
}

#[tauri::command]
pub fn restore_window_preferences(
    window: WebviewWindow,
    runtime: tauri::State<'_, WindowRuntime>,
    preferences: WindowPreferences,
) -> Result<WindowPosition, String> {
    window
        .set_always_on_top(preferences.always_on_top)
        .map_err(|error| error.to_string())?;

    if let Some(position) = preferences.position {
        if is_position_visible(&window, position)? {
            window
                .set_position(PhysicalPosition::<i32>::from(position))
                .map_err(|error| error.to_string())?;
        }
    }

    let current = WindowPosition::from(
        window
            .outer_position()
            .map_err(|error| error.to_string())?,
    );
    set_window_movable(&window, !preferences.locked)?;
    runtime.set_locked_position(preferences.locked.then_some(current))?;
    Ok(current)
}

#[tauri::command]
pub fn set_window_presentation(
    window: WebviewWindow,
    runtime: tauri::State<'_, WindowRuntime>,
    always_on_top: bool,
    locked: bool,
) -> Result<WindowPosition, String> {
    let current = WindowPosition::from(
        window
            .outer_position()
            .map_err(|error| error.to_string())?,
    );
    window
        .set_always_on_top(always_on_top)
        .map_err(|error| error.to_string())?;
    set_window_movable(&window, !locked)?;
    runtime.set_locked_position(locked.then_some(current))?;
    Ok(current)
}

#[cfg(target_os = "macos")]
fn set_window_movable(window: &WebviewWindow, movable: bool) -> Result<(), String> {
    let native_window = window.ns_window().map_err(|error| error.to_string())? as usize;
    window
        .run_on_main_thread(move || unsafe {
            let native_window = &*(native_window as *const objc2_app_kit::NSWindow);
            native_window.setMovable(movable);
        })
        .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "macos"))]
fn set_window_movable(_window: &WebviewWindow, _movable: bool) -> Result<(), String> {
    Ok(())
}

pub fn enforce_locked_position(window: &Window, event: &WindowEvent) {
    let WindowEvent::Moved(current) = event else {
        return;
    };
    let Some(locked) = window.state::<WindowRuntime>().locked_position() else {
        return;
    };
    if current.x != locked.x || current.y != locked.y {
        let _ = window.set_position(PhysicalPosition::<i32>::from(locked));
    }
}

fn is_position_visible(window: &WebviewWindow, position: WindowPosition) -> Result<bool, String> {
    let monitors = window
        .available_monitors()
        .map_err(|error| error.to_string())?;
    Ok(monitors.iter().any(|monitor| {
        let area = monitor.work_area();
        let right = area.position.x.saturating_add_unsigned(area.size.width);
        let bottom = area.position.y.saturating_add_unsigned(area.size.height);
        position.x >= area.position.x
            && position.x < right
            && position.y >= area.position.y
            && position.y < bottom
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_position_round_trips() {
        let saved = WindowPosition { x: -120, y: 64 };
        let physical = PhysicalPosition::<i32>::from(saved);
        assert_eq!(WindowPosition::from(physical), saved);
    }
}
