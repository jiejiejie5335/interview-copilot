use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;

use crate::stt::{AudioChunk, RefinementModel, TencentAsrConfig};

use super::secrets::{load_secret, SecretKind};

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SttConfig {
    mode: SttMode,
    hotwords: String,
    tencent: TencentAsrConfig,
}

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum SttMode {
    LocalFast,
    LocalAccurate,
    TencentCloud,
}

pub struct AudioRuntime {
    running: Arc<AtomicBool>,
    stop_signal: Mutex<Option<Arc<AtomicBool>>>,
}

impl Default for AudioRuntime {
    fn default() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            stop_signal: Mutex::new(None),
        }
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioLevelEvent {
    rms: f32,
    peak: f32,
    sample_rate: u32,
    speaker: &'static str,
}

fn spawn_stt_worker(
    app: AppHandle,
    receiver: mpsc::Receiver<AudioChunk>,
    config: SttConfig,
    speaker: &'static str,
    stop: Arc<AtomicBool>,
) {
    match config.mode {
        SttMode::LocalFast | SttMode::LocalAccurate => {
            let refinement_model = match config.mode {
                SttMode::LocalFast => RefinementModel::SenseVoice,
                SttMode::LocalAccurate => RefinementModel::Qwen3,
                SttMode::TencentCloud => unreachable!(),
            };
            tauri::async_runtime::spawn_blocking(move || {
                crate::stt::run_streaming_worker(
                    app,
                    receiver,
                    config.hotwords,
                    refinement_model,
                    speaker,
                )
            });
        }
        SttMode::TencentCloud => {
            tauri::async_runtime::spawn(async move {
                if let Err(error) = crate::stt::run_tencent_streaming_worker(
                    app.clone(),
                    receiver,
                    config.tencent,
                    config.hotwords,
                    speaker,
                )
                .await
                {
                    let source = if speaker == "interviewer" { "系统音频" } else { "麦克风" };
                    let _ = app.emit("audio-error", format!("{source}识别失败：{error}"));
                    stop.store(true, Ordering::Release);
                }
            });
        }
    }
}

fn audio_handler(
    app: AppHandle,
    sender: mpsc::Sender<AudioChunk>,
    speaker: &'static str,
) -> impl FnMut(&[f32], u32) + Send + 'static {
    let mut meter_squares = 0.0_f32;
    let mut meter_peak = 0.0_f32;
    let mut meter_samples = 0_usize;

    move |samples, sample_rate| {
        meter_squares += samples.iter().map(|sample| sample * sample).sum::<f32>();
        meter_peak = samples
            .iter()
            .fold(meter_peak, |peak, sample| peak.max(sample.abs()));
        meter_samples += samples.len();
        if meter_samples >= (sample_rate / 20).max(256) as usize {
            let rms = (meter_squares / meter_samples as f32).sqrt();
            let _ = app.emit(
                "audio-level",
                AudioLevelEvent {
                    rms,
                    peak: meter_peak,
                    sample_rate,
                    speaker,
                },
            );
            meter_squares = 0.0;
            meter_peak = 0.0;
            meter_samples = 0;
        }

        let _ = sender.try_send(AudioChunk {
            samples: samples.to_vec(),
            sample_rate,
        });
    }
}

#[tauri::command]
pub async fn start_audio_capture(
    app: AppHandle,
    runtime: State<'_, AudioRuntime>,
    mut config: SttConfig,
) -> Result<(), String> {
    if matches!(config.mode, SttMode::TencentCloud)
        && config.tencent.secret_key.trim().is_empty()
    {
        config.tencent.secret_key = load_secret(SecretKind::TencentSecretKey)?
            .ok_or_else(|| "请先保存腾讯云 SecretKey".to_string())?;
    }
    match config.mode {
        SttMode::LocalFast => {
            crate::stt::validate_local_installation(RefinementModel::SenseVoice)
        }
        SttMode::LocalAccurate => {
            crate::stt::validate_local_installation(RefinementModel::Qwen3)
        }
        SttMode::TencentCloud => config.tencent.validate(),
    }
    .map_err(|error| error.to_string())?;

    if runtime.running.swap(true, Ordering::AcqRel) {
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    {
        runtime.running.store(false, Ordering::Release);
        return Err("当前版本仅实现 macOS 系统音频捕获".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let stop = Arc::new(AtomicBool::new(false));
        *runtime.stop_signal.lock().map_err(|_| "音频状态锁异常")? = Some(stop.clone());
        let app_handle = app.clone();
        let running = runtime.running.clone();

        tauri::async_runtime::spawn(async move {
            // Each source has an independent queue and recognizer so speaker
            // attribution remains reliable even when both sides briefly overlap.
            let (system_sender, system_receiver) = mpsc::channel::<AudioChunk>(256);
            let (microphone_sender, microphone_receiver) = mpsc::channel::<AudioChunk>(256);
            spawn_stt_worker(
                app_handle.clone(),
                system_receiver,
                config.clone(),
                "interviewer",
                stop.clone(),
            );
            spawn_stt_worker(
                app_handle.clone(),
                microphone_receiver,
                config,
                "candidate",
                stop.clone(),
            );

            let system_capture = crate::audio::macos::run_pcm_stream(
                stop.clone(),
                audio_handler(app_handle.clone(), system_sender, "interviewer"),
            );
            let microphone_capture = crate::audio::macos::run_microphone_pcm_stream(
                stop.clone(),
                audio_handler(app_handle.clone(), microphone_sender, "candidate"),
            );
            tokio::pin!(system_capture, microphone_capture);

            let (first_source, first_result, second_source, second_result) = tokio::select! {
                result = &mut system_capture => {
                    stop.store(true, Ordering::Release);
                    let microphone_result = (&mut microphone_capture).await;
                    ("系统音频", result, "麦克风", microphone_result)
                }
                result = &mut microphone_capture => {
                    stop.store(true, Ordering::Release);
                    let system_result = (&mut system_capture).await;
                    ("麦克风", result, "系统音频", system_result)
                }
            };

            for (source, result) in [
                (first_source, first_result),
                (second_source, second_result),
            ] {
                if let Err(error) = result {
                    let _ = app.emit("audio-error", format!("{source}采集失败：{error}"));
                }
            }
            running.store(false, Ordering::Release);
        });
        Ok(())
    }
}

#[tauri::command]
pub fn stop_audio_capture(runtime: State<'_, AudioRuntime>) -> Result<(), String> {
    if let Some(stop) = runtime
        .stop_signal
        .lock()
        .map_err(|_| "音频状态锁异常")?
        .take()
    {
        stop.store(true, Ordering::Release);
    }
    runtime.running.store(false, Ordering::Release);
    Ok(())
}
