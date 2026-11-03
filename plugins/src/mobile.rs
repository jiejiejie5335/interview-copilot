use serde::de::DeserializeOwned;
use tauri::{
    plugin::{PluginApi, PluginHandle},
    AppHandle, Runtime,
};

use crate::models::*;

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_ios_ambient_audio);

// initializes the Kotlin or Swift plugin classes
pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> crate::Result<IosAmbientAudio<R>> {
    #[cfg(target_os = "android")]
    let handle = api.register_android_plugin("", "ExamplePlugin")?;
    #[cfg(target_os = "ios")]
    let handle = api.register_ios_plugin(init_plugin_ios_ambient_audio)?;
    Ok(IosAmbientAudio(handle))
}

/// Access to the ios-ambient-audio APIs.
pub struct IosAmbientAudio<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> IosAmbientAudio<R> {
    pub fn start_capture(&self, payload: CaptureRequest) -> crate::Result<CaptureResponse> {
        self.0
            .run_mobile_plugin("startCapture", payload)
            .map_err(Into::into)
    }

    pub fn stop_capture(&self) -> crate::Result<()> {
        self.0
            .run_mobile_plugin::<()>("stopCapture", ())
            .map_err(Into::into)
    }
}
