pub mod audio;
mod commands;
pub mod stt;

use commands::audio::{start_audio_capture, stop_audio_capture, AudioRuntime};
#[cfg(desktop)]
use commands::companion::{
    get_companion_server_info, start_companion_server, stop_companion_server,
    update_companion_transcript, CompanionRuntime,
};
use commands::provider::stream_chat;
use commands::runtime::get_runtime_info;
use commands::secrets::{delete_secret, get_secret_status, save_secret};
#[cfg(desktop)]
use commands::shortcut::{
    get_global_shortcut_status, GlobalShortcutRuntime, GLOBAL_ANSWER_SHORTCUT,
};
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

    #[cfg(target_os = "macos")]
    let builder = builder.plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(|app, _shortcut, event| {
                use tauri::Emitter;
                use tauri_plugin_global_shortcut::ShortcutState;

                if event.state == ShortcutState::Pressed {
                    eprintln!("[shortcut] 已触发全局生成建议");
                    let _ = app.emit("global-answer-shortcut", ());
                }
            })
            .build(),
    );

    #[cfg(desktop)]
    let builder = builder
        .manage(CompanionRuntime::default())
        .manage(GlobalShortcutRuntime::default())
        .manage(WindowRuntime::default())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                use tauri::Manager;
                use tauri_plugin_global_shortcut::GlobalShortcutExt;

                let result = app
                    .global_shortcut()
                    .register(GLOBAL_ANSWER_SHORTCUT)
                    .map_err(|error| format!("无法注册全局快捷键：{error}"));
                match &result {
                    Ok(()) => eprintln!("[shortcut] 已注册 {GLOBAL_ANSWER_SHORTCUT}"),
                    Err(error) => eprintln!("[shortcut] {error}"),
                }
                app.state::<GlobalShortcutRuntime>()
                    .set_registration_result(result);
            }
            Ok(())
        })
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
            update_companion_transcript,
            get_global_shortcut_status,
            restore_window_preferences,
            set_window_presentation
        ]);

    builder
        .run(tauri::generate_context!())
        .expect("failed to run Interview Copilot");
}
