mod segmenter;
#[cfg(target_os = "macos")]
mod sherpa;
mod tencent;
mod text;

pub struct AudioChunk {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

#[derive(Clone, Copy, Debug)]
pub enum RefinementModel {
    SenseVoice,
    Qwen3,
}

pub use segmenter::SpeechSegmenter;
#[cfg(target_os = "macos")]
pub use sherpa::{
    refine_samples, refine_samples_with_model, run_streaming_worker, transcribe_samples,
    validate_installation, validate_local_installation,
};
pub use tencent::{run_tencent_streaming_worker, TencentAsrConfig, TencentStreamEnd};

#[cfg(not(target_os = "macos"))]
pub fn validate_local_installation(_model: RefinementModel) -> anyhow::Result<()> {
    Err(anyhow::anyhow!(
        "移动端首版不加载本地 STT 模型，请选择腾讯云实时识别"
    ))
}

#[cfg(not(target_os = "macos"))]
pub fn run_streaming_worker(
    _app: tauri::AppHandle,
    _receiver: tokio::sync::mpsc::Receiver<AudioChunk>,
    _hotwords: String,
    _model: RefinementModel,
    _speaker: &'static str,
) {
}
