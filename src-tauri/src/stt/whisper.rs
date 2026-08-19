use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::{process::Command, sync::mpsc};

static TRANSCRIPT_COUNTER: AtomicU64 = AtomicU64::new(1);

pub struct AudioSegment {
    pub samples: Vec<f32>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptEvent {
    id: String,
    text: String,
    speaker: &'static str,
    is_final: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SttStatusEvent {
    state: &'static str,
    message: String,
}

fn model_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../models/ggml-small.bin")
}

fn cli_path() -> PathBuf {
    [
        PathBuf::from("/opt/homebrew/bin/whisper-cli"),
        PathBuf::from("/usr/local/bin/whisper-cli"),
    ]
    .into_iter()
    .find(|path| path.exists())
    .unwrap_or_else(|| PathBuf::from("whisper-cli"))
}

pub fn validate_installation() -> Result<()> {
    let model = model_path();
    if !model.exists() {
        return Err(anyhow!(
            "未找到 STT 模型：{}。请运行 npm run setup:stt",
            model.display()
        ));
    }
    let cli = cli_path();
    if cli.components().count() > 1 && !cli.exists() {
        return Err(anyhow!("未找到 whisper-cli，请运行 npm run setup:stt"));
    }
    Ok(())
}

fn emit_status(app: &AppHandle, state: &'static str, message: impl Into<String>) {
    let _ = app.emit(
        "stt-status",
        SttStatusEvent {
            state,
            message: message.into(),
        },
    );
}

fn write_wav(path: &Path, samples: &[f32]) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).context("创建 STT 临时 WAV 失败")?;
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer.write_sample(value).context("写入 STT 临时 WAV 失败")?;
    }
    writer.finalize().context("完成 STT 临时 WAV 失败")
}

fn normalize_transcript(output: &str) -> String {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('['))
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches('"')
        .trim()
        .to_string()
}

fn transcript_key(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn is_likely_hallucination(text: &str) -> bool {
    let key = transcript_key(text);
    matches!(
        key.as_str(),
        "you"
            | "yeah"
            | "uh"
            | "um"
            | "hmm"
            | "oh"
            | "thankyou"
            | "thanksforwatching"
            | "theend"
            | "谢谢"
            | "谢谢观看"
            | "感谢观看"
            | "字幕"
    )
}

pub async fn transcribe_samples(samples: Vec<f32>) -> Result<String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("interview-copilot-{timestamp}.wav"));
    write_wav(&path, &samples)?;

    let output = Command::new(cli_path())
        .args([
            "-ng", "-l", "auto", "-np", "-nt", "-sns", "-nf", "-nth", "0.45", "-lpt",
            "-0.8", "-et", "2.2", "-t", "6", "-m",
        ])
        .arg(model_path())
        .arg("-f")
        .arg(&path)
        .output()
        .await
        .context("启动 whisper.cpp 失败")?;
    let _ = tokio::fs::remove_file(&path).await;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("whisper.cpp 识别失败：{}", detail.trim()));
    }
    Ok(normalize_transcript(&String::from_utf8_lossy(&output.stdout)))
}

pub async fn run_worker(app: AppHandle, mut receiver: mpsc::Receiver<AudioSegment>) {
    let mut last_emitted: Option<(String, Instant)> = None;
    while let Some(segment) = receiver.recv().await {
        emit_status(&app, "transcribing", "正在识别刚才的语音…");
        match transcribe_samples(segment.samples).await {
            Ok(text) if !text.is_empty() && !is_likely_hallucination(&text) => {
                let key = transcript_key(&text);
                let is_recent_duplicate = last_emitted.as_ref().is_some_and(|(previous, at)| {
                    previous == &key && at.elapsed() < Duration::from_secs(20)
                });
                if is_recent_duplicate {
                    emit_status(&app, "listening", "已忽略重复的短音频，继续监听");
                    continue;
                }
                let id = TRANSCRIPT_COUNTER.fetch_add(1, Ordering::Relaxed);
                let _ = app.emit(
                    "transcript-final",
                    TranscriptEvent {
                        id: format!("stt-{id}"),
                        text,
                        speaker: "interviewer",
                        is_final: true,
                    },
                );
                last_emitted = Some((key, Instant::now()));
                emit_status(&app, "listening", "继续监听系统音频");
            }
            Ok(_) => emit_status(&app, "listening", "已忽略静音、余音或无效短语，继续监听"),
            Err(error) => emit_status(&app, "error", error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_likely_hallucination, normalize_transcript, transcript_key};

    #[test]
    fn removes_blank_and_whitespace() {
        assert_eq!(normalize_transcript("\n [BLANK_AUDIO]\n 你好，世界。 \n"), "你好，世界。");
    }

    #[test]
    fn filters_common_short_hallucinations() {
        assert!(is_likely_hallucination(" you. "));
        assert!(is_likely_hallucination("谢谢观看！"));
        assert!(!is_likely_hallucination("How would you design this API?"));
    }

    #[test]
    fn creates_stable_deduplication_keys() {
        assert_eq!(transcript_key("Hello, World!"), "helloworld");
        assert_eq!(transcript_key("请介绍一下。"), "请介绍一下");
    }
}
