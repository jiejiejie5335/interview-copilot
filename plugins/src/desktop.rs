use serde::de::DeserializeOwned;
use tauri::{plugin::PluginApi, AppHandle, Runtime};

use crate::models::*;

pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> crate::Result<IosAmbientAudio<R>> {
    Ok(IosAmbientAudio(app.clone()))
}

/// Access to the ios-ambient-audio APIs.
pub struct IosAmbientAudio<R: Runtime>(AppHandle<R>);

impl<R: Runtime> IosAmbientAudio<R> {
    pub fn start_capture(&self, _payload: CaptureRequest) -> crate::Result<CaptureResponse> {
        Err(crate::Error::UnsupportedPlatform)
    }

    pub fn stop_capture(&self) -> crate::Result<()> {
        Err(crate::Error::UnsupportedPlatform)
    }
}
