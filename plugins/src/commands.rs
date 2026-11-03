use tauri::{command, AppHandle, Runtime};

use crate::models::*;
use crate::IosAmbientAudioExt;
use crate::Result;

#[command]
pub(crate) async fn start_capture<R: Runtime>(
    app: AppHandle<R>,
    payload: CaptureRequest,
) -> Result<CaptureResponse> {
    app.ios_ambient_audio().start_capture(payload)
}

#[command]
pub(crate) async fn stop_capture<R: Runtime>(app: AppHandle<R>) -> Result<()> {
    app.ios_ambient_audio().stop_capture()
}
