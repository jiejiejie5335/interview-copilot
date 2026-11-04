use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[cfg(target_os = "ios")]
use std::sync::OnceLock;

use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;
use tokio::time::{self, Duration, Instant};

use crate::stt::{AudioChunk, RefinementModel, TencentAsrConfig, TencentStreamEnd};

use super::secrets::{load_secret, SecretKind};

#[cfg(target_os = "ios")]
use tauri_plugin_ios_ambient_audio::{CaptureRequest, IosAmbientAudioExt};

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

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioSourceStatusEvent {
    speaker: &'static str,
    state: &'static str,
    message: String,
    retry_delay_ms: Option<u64>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStarted {
    audio_capture: &'static str,
}

#[cfg(target_os = "ios")]
#[derive(Clone)]
struct IosAudioSink {
    app: AppHandle,
    sender: mpsc::Sender<AudioChunk>,
}

#[cfg(target_os = "ios")]
static IOS_AUDIO_SINK: OnceLock<Mutex<Option<IosAudioSink>>> = OnceLock::new();

#[cfg(target_os = "ios")]
fn ios_audio_sink() -> &'static Mutex<Option<IosAudioSink>> {
    IOS_AUDIO_SINK.get_or_init(|| Mutex::new(None))
}

#[cfg(target_os = "ios")]
fn set_ios_audio_sink(sink: Option<IosAudioSink>) -> Result<(), String> {
    *ios_audio_sink()
        .lock()
        .map_err(|_| "iPhone 音频状态锁异常".to_string())? = sink;
    Ok(())
}

/// Called by the Swift AVAudioEngine tap. Audio stays in memory and is copied
/// directly into the bounded Tencent ASR queue.
#[cfg(target_os = "ios")]
#[no_mangle]
pub unsafe extern "C" fn interview_audio_push_ffi(
    samples: *const f32,
    sample_count: i32,
    sample_rate: u32,
) {
    if samples.is_null() || sample_count <= 0 || sample_rate == 0 {
        return;
    }
    let Ok(sink) = ios_audio_sink().lock().map(|value| value.as_ref().cloned()) else {
        return;
    };
    let Some(sink) = sink else { return };
    let samples = unsafe { std::slice::from_raw_parts(samples, sample_count as usize) };
    let squares = samples.iter().map(|sample| sample * sample).sum::<f32>();
    let peak = samples
        .iter()
        .fold(0.0_f32, |current, sample| current.max(sample.abs()));
    let _ = sink.app.emit(
        "audio-level",
        AudioLevelEvent {
            rms: (squares / samples.len() as f32).sqrt(),
            peak,
            sample_rate,
            speaker: "ambient",
        },
    );
    let _ = sink.sender.try_send(AudioChunk {
        samples: samples.to_vec(),
        sample_rate,
    });
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
                let source = match speaker {
                    "interviewer" => "系统音频",
                    "candidate" => "麦克风",
                    _ => "环境音频",
                };
                let mut receiver = receiver;
                let mut failure_count = 0_u32;
                loop {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    let started_at = Instant::now();
                    match crate::stt::run_tencent_streaming_worker(
                        app.clone(),
                        &mut receiver,
                        &config.tencent,
                        &config.hotwords,
                        speaker,
                    )
                    .await
                    {
                        Ok(TencentStreamEnd::InputClosed) => return,
                        Ok(TencentStreamEnd::SessionComplete) => {
                            failure_count = 0;
                            emit_audio_source_status(
                                &app,
                                speaker,
                                "connecting",
                                format!("正在续接{source}腾讯云识别会话…"),
                                Some(Duration::from_millis(150)),
                            );
                            if !wait_for_retry(&stop, Duration::from_millis(150)).await {
                                return;
                            }
                        }
                        Err(error) => {
                            if started_at.elapsed() >= Duration::from_secs(20) {
                                failure_count = 0;
                            }
                            let delay = capture_retry_delay(failure_count);
                            failure_count = failure_count.saturating_add(1);
                            emit_audio_source_status(
                                &app,
                                speaker,
                                "recovering",
                                format!(
                                    "{source}识别连接中断，{:.1} 秒后自动重连；音频采集继续。{error}",
                                    delay.as_secs_f32()
                                ),
                                Some(delay),
                            );
                            if !wait_for_retry(&stop, delay).await {
                                return;
                            }
                        }
                    }
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

fn emit_audio_source_status(
    app: &AppHandle,
    speaker: &'static str,
    state: &'static str,
    message: impl Into<String>,
    retry_delay: Option<Duration>,
) {
    let message = message.into();
    if state == "recovering" {
        eprintln!("[audio:{speaker}] {message}");
    }
    let _ = app.emit(
        "audio-source-status",
        AudioSourceStatusEvent {
            speaker,
            state,
            message,
            retry_delay_ms: retry_delay.map(|delay| delay.as_millis() as u64),
        },
    );
}

fn capture_retry_delay(failure_count: u32) -> Duration {
    Duration::from_millis(500_u64.saturating_mul(1_u64 << failure_count.min(4)))
}

async fn wait_for_retry(stop: &AtomicBool, delay: Duration) -> bool {
    let deadline = Instant::now() + delay;
    while Instant::now() < deadline {
        if stop.load(Ordering::Acquire) {
            return false;
        }
        time::sleep(
            Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now())),
        )
        .await;
    }
    !stop.load(Ordering::Acquire)
}

#[cfg(target_os = "macos")]
async fn supervise_macos_capture(
    app: AppHandle,
    sender: mpsc::Sender<AudioChunk>,
    speaker: &'static str,
    source: &'static str,
    microphone: bool,
    stop: Arc<AtomicBool>,
) {
    let mut failure_count = 0_u32;
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }

        let recovering = failure_count > 0;
        emit_audio_source_status(
            &app,
            speaker,
            "connecting",
            if recovering {
                format!("正在重新连接{source}…")
            } else {
                format!("正在连接{source}…")
            },
            None,
        );
        let started_at = Instant::now();
        let ready_app = app.clone();
        let on_ready = move |sample_rate| {
            emit_audio_source_status(
                &ready_app,
                speaker,
                "listening",
                if recovering {
                    format!("{source}已恢复监听（{sample_rate} Hz）")
                } else {
                    format!("{source}已连接（{sample_rate} Hz）")
                },
                None,
            );
        };
        let result = if microphone {
            crate::audio::macos::run_microphone_pcm_stream(
                stop.clone(),
                audio_handler(app.clone(), sender.clone(), speaker),
                on_ready,
            )
            .await
        } else {
            crate::audio::macos::run_pcm_stream(
                stop.clone(),
                audio_handler(app.clone(), sender.clone(), speaker),
                on_ready,
            )
            .await
        };

        if stop.load(Ordering::Acquire) {
            return;
        }
        if started_at.elapsed() >= Duration::from_secs(20) {
            failure_count = 0;
        }
        let delay = capture_retry_delay(failure_count);
        failure_count = failure_count.saturating_add(1);
        let detail = result
            .err()
            .map(|error| error.to_string())
            .unwrap_or_else(|| "音频流意外结束".to_string());
        emit_audio_source_status(
            &app,
            speaker,
            "recovering",
            format!(
                "{source}暂时中断，{:.1} 秒后自动重连；另一通道继续监听。{detail}",
                delay.as_secs_f32()
            ),
            Some(delay),
        );
        if !wait_for_retry(&stop, delay).await {
            return;
        }
    }
}

#[tauri::command]
pub async fn start_audio_capture(
    app: AppHandle,
    runtime: State<'_, AudioRuntime>,
    mut config: SttConfig,
) -> Result<CaptureStarted, String> {
    if matches!(config.mode, SttMode::TencentCloud) && config.tencent.secret_key.trim().is_empty() {
        config.tencent.secret_key = load_secret(SecretKind::TencentSecretKey)?
            .ok_or_else(|| "请先保存腾讯云 SecretKey".to_string())?;
    }
    match config.mode {
        SttMode::LocalFast => crate::stt::validate_local_installation(RefinementModel::SenseVoice),
        SttMode::LocalAccurate => crate::stt::validate_local_installation(RefinementModel::Qwen3),
        SttMode::TencentCloud => config.tencent.validate(),
    }
    .map_err(|error| error.to_string())?;

    #[cfg(target_os = "ios")]
    if !matches!(config.mode, SttMode::TencentCloud) {
        return Err("iPhone 环境监听首版仅支持腾讯云实时语音识别".to_string());
    }

    if runtime.running.swap(true, Ordering::AcqRel) {
        return Ok(CaptureStarted {
            audio_capture: if cfg!(target_os = "ios") {
                "ambient"
            } else {
                "dual"
            },
        });
    }

    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    {
        runtime.running.store(false, Ordering::Release);
        return Err("当前版本仅实现 macOS 双通道和 iPhone 环境音频捕获".to_string());
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

            let system_capture = supervise_macos_capture(
                app_handle.clone(),
                system_sender,
                "interviewer",
                "系统音频",
                false,
                stop.clone(),
            );
            let microphone_capture = supervise_macos_capture(
                app_handle,
                microphone_sender,
                "candidate",
                "麦克风",
                true,
                stop.clone(),
            );
            tokio::join!(system_capture, microphone_capture);
            running.store(false, Ordering::Release);
        });
        Ok(CaptureStarted {
            audio_capture: "dual",
        })
    }

    #[cfg(target_os = "ios")]
    {
        let stop = Arc::new(AtomicBool::new(false));
        *runtime.stop_signal.lock().map_err(|_| "音频状态锁异常")? = Some(stop.clone());
        let (sender, receiver) = mpsc::channel::<AudioChunk>(256);
        set_ios_audio_sink(Some(IosAudioSink {
            app: app.clone(),
            sender,
        }))?;
        spawn_stt_worker(app.clone(), receiver, config, "ambient", stop.clone());

        if let Err(error) = app.ios_ambient_audio().start_capture(CaptureRequest {
            preferred_sample_rate: Some(48_000.0),
        }) {
            stop.store(true, Ordering::Release);
            let _ = set_ios_audio_sink(None);
            runtime.running.store(false, Ordering::Release);
            return Err(error.to_string());
        }
        Ok(CaptureStarted {
            audio_capture: "ambient",
        })
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::capture_retry_delay;
    use std::time::Duration;

    #[test]
    fn capture_retry_uses_bounded_exponential_backoff() {
        assert_eq!(capture_retry_delay(0), Duration::from_millis(500));
        assert_eq!(capture_retry_delay(1), Duration::from_secs(1));
        assert_eq!(capture_retry_delay(4), Duration::from_secs(8));
        assert_eq!(capture_retry_delay(12), Duration::from_secs(8));
    }
}

#[tauri::command]
pub fn stop_audio_capture(_app: AppHandle, runtime: State<'_, AudioRuntime>) -> Result<(), String> {
    if let Some(stop) = runtime
        .stop_signal
        .lock()
        .map_err(|_| "音频状态锁异常")?
        .take()
    {
        stop.store(true, Ordering::Release);
    }
    #[cfg(target_os = "ios")]
    {
        let _ = _app.ios_ambient_audio().stop_capture();
        set_ios_audio_sink(None)?;
    }
    runtime.running.store(false, Ordering::Release);
    Ok(())
}
