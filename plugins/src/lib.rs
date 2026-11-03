use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

pub use models::*;

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

mod commands;
mod error;
mod models;

pub use error::{Error, Result};

#[cfg(desktop)]
use desktop::IosAmbientAudio;
#[cfg(mobile)]
use mobile::IosAmbientAudio;

/// Extensions to [`tauri::App`], [`tauri::AppHandle`] and [`tauri::Window`] to access the ios-ambient-audio APIs.
pub trait IosAmbientAudioExt<R: Runtime> {
    fn ios_ambient_audio(&self) -> &IosAmbientAudio<R>;
}

impl<R: Runtime, T: Manager<R>> crate::IosAmbientAudioExt<R> for T {
    fn ios_ambient_audio(&self) -> &IosAmbientAudio<R> {
        self.state::<IosAmbientAudio<R>>().inner()
    }
}

/// Initializes the plugin.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("ios-ambient-audio")
        .invoke_handler(tauri::generate_handler![
            commands::start_capture,
            commands::stop_capture
        ])
        .setup(|app, api| {
            #[cfg(mobile)]
            let ios_ambient_audio = mobile::init(app, api)?;
            #[cfg(desktop)]
            let ios_ambient_audio = desktop::init(app, api)?;
            app.manage(ios_ambient_audio);
            Ok(())
        })
        .build()
}
