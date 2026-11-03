pub mod audio;
mod commands;
pub mod stt;

use commands::audio::{start_audio_capture, stop_audio_capture, AudioRuntime};
#[cfg(desktop)]
use commands::companion::{
    get_companion_server_info, start_companion_server, stop_companion_server, CompanionRuntime,
};
use commands::provider::stream_chat;
use commands::runtime::get_runtime_info;
use commands::secrets::{delete_secret, get_secret_status, save_secret};
use commands::translation::translate_text;
#[cfg(desktop)]
use commands::window::{
    enforce_locked_position, restore_window_preferences, set_window_presentation, WindowRuntime,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_ios_ambient_audio::init())
        .manage(AudioRuntime::default())
        .invoke_handler(tauri::generate_handler![
            start_audio_capture,
            stop_audio_capture,
            stream_chat,
            get_runtime_info,
            get_secret_status,
            save_secret,
            delete_secret,
            translate_text
        ]);

    #[cfg(desktop)]
    let builder = builder
        .manage(CompanionRuntime::default())
        .manage(WindowRuntime::default())
        .on_window_event(enforce_locked_position)
        .invoke_handler(tauri::generate_handler![
            start_audio_capture,
            stop_audio_capture,
            stream_chat,
            get_runtime_info,
            get_secret_status,
            save_secret,
            delete_secret,
            translate_text,
            start_companion_server,
            stop_companion_server,
            get_companion_server_info,
            restore_window_preferences,
            set_window_presentation
        ]);

    builder
        .run(tauri::generate_context!())
        .expect("failed to run Interview Copilot");
}
