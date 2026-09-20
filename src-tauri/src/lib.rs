pub mod audio;
mod commands;
pub mod stt;

use commands::audio::{start_audio_capture, stop_audio_capture, AudioRuntime};
use commands::provider::stream_chat;
use commands::secrets::{delete_secret, get_secret_status, save_secret};
use commands::translation::translate_text;
use commands::window::{
    enforce_locked_position, restore_window_preferences, set_window_presentation, WindowRuntime,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AudioRuntime::default())
        .manage(WindowRuntime::default())
        .on_window_event(enforce_locked_position)
        .invoke_handler(tauri::generate_handler![
            start_audio_capture,
            stop_audio_capture,
            stream_chat,
            get_secret_status,
            save_secret,
            delete_secret,
            translate_text,
            restore_window_preferences,
            set_window_presentation
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Interview Copilot");
}
