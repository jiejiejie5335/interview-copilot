#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    platform: &'static str,
    mobile: bool,
    audio_capture: &'static str,
}

#[tauri::command]
pub fn get_runtime_info() -> RuntimeInfo {
    RuntimeInfo {
        platform: std::env::consts::OS,
        mobile: cfg!(mobile),
        audio_capture: if cfg!(target_os = "ios") {
            "ambient"
        } else {
            "dual"
        },
    }
}
