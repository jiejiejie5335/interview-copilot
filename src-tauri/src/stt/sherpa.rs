use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{sync_channel, Receiver as RefinementReceiver, SyncSender, TrySendError},
    },
    thread,
    time::Instant,
};

use anyhow::{anyhow, Result};
use serde::Serialize;
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OnlineRecognizer, OnlineRecognizerConfig,
};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use super::{
    text::{hotword_terms, normalize_hotword_aliases, parse_hotwords, HotwordEntry},
    AudioChunk, RefinementModel,
};

static TRANSCRIPT_COUNTER: AtomicU64 = AtomicU64::new(1);
const ONLINE_MODEL_DIRECTORY: &str = "sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20";
const REFINEMENT_MODEL_DIRECTORY: &str = "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17";
const QWEN_MODEL_DIRECTORY: &str = "sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25";
impl RefinementModel {
    fn label(self) -> &'static str {
        match self {
            Self::SenseVoice => "SenseVoice",
            Self::Qwen3 => "Qwen3-ASR",
        }
    }
}

struct RefinementJob {
    id: String,
    samples: Vec<f32>,
    sample_rate: u32,
    fast_text: String,
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

fn model_directory(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../models")
        .join(name)
}

fn online_model_files() -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let directory = model_directory(ONLINE_MODEL_DIRECTORY);
    (
        directory.join("encoder.int8.onnx"),
        directory.join("decoder.onnx"),
        directory.join("joiner.int8.onnx"),
        directory.join("tokens.txt"),
        directory.join("bpe.vocab"),
    )
}

fn refinement_model_files() -> (PathBuf, PathBuf) {
    let directory = model_directory(REFINEMENT_MODEL_DIRECTORY);
    (
        directory.join("model.int8.onnx"),
        directory.join("tokens.txt"),
    )
}

fn qwen_model_files() -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let directory = model_directory(QWEN_MODEL_DIRECTORY);
    (
        directory.join("conv_frontend.onnx"),
        directory.join("encoder.int8.onnx"),
        directory.join("decoder.int8.onnx"),
        directory.join("tokenizer"),
    )
}

pub fn validate_installation() -> Result<()> {
    validate_local_installation(RefinementModel::SenseVoice)
}

pub fn validate_local_installation(refinement_model: RefinementModel) -> Result<()> {
    let (encoder, decoder, joiner, tokens, bpe_vocab) = online_model_files();
    let mut required = vec![encoder, decoder, joiner, tokens, bpe_vocab];
    match refinement_model {
        RefinementModel::SenseVoice => {
            let (model, tokens) = refinement_model_files();
            required.extend([model, tokens]);
        }
        RefinementModel::Qwen3 => {
            let (frontend, encoder, decoder, tokenizer) = qwen_model_files();
            required.extend([
                frontend,
                encoder,
                decoder,
                tokenizer.join("merges.txt"),
                tokenizer.join("tokenizer_config.json"),
                tokenizer.join("vocab.json"),
            ]);
        }
    }
    let missing = required
        .into_iter()
        .filter(|path| !path.exists())
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();

    if missing.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "缺少流式 STT 模型文件：{}。请先运行 npm run setup:stt",
            missing.join("、")
        ))
    }
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

fn select_final_text(fast: &str, refined: &str, entries: &[HotwordEntry]) -> String {
    let (fast, fast_hits) = normalize_hotword_aliases(fast, entries);
    let (refined, refined_hits) = normalize_hotword_aliases(refined, entries);
    if refined.trim().is_empty() || refined_hits < fast_hits {
        fast
    } else {
        refined
    }
}

fn create_recognizer(extra_hotwords: &str) -> Result<OnlineRecognizer> {
    let (encoder, decoder, joiner, tokens, bpe_vocab) = online_model_files();
    let mut config = OnlineRecognizerConfig::default();
    config.model_config.transducer.encoder = Some(encoder.display().to_string());
    config.model_config.transducer.decoder = Some(decoder.display().to_string());
    config.model_config.transducer.joiner = Some(joiner.display().to_string());
    config.model_config.tokens = Some(tokens.display().to_string());
    config.model_config.provider = Some("cpu".to_string());
    config.model_config.num_threads = 1;
    config.model_config.modeling_unit = Some("cjkchar+bpe".to_string());
    config.model_config.bpe_vocab = Some(bpe_vocab.display().to_string());
    config.decoding_method = Some("modified_beam_search".to_string());
    config.max_active_paths = 4;
    let hotwords = parse_hotwords(extra_hotwords)
        .into_iter()
        .map(|entry| entry.canonical)
        .collect::<Vec<_>>()
        .join("\n");
    config.hotwords_buf = Some(hotwords.into_bytes());
    config.hotwords_score = 2.0;
    config.enable_endpoint = true;

    // Show partial text while the interviewer is speaking, then commit quickly
    // after a real utterance. The first rule only handles prolonged pure silence.
    config.rule1_min_trailing_silence = 1.6;
    config.rule2_min_trailing_silence = 0.75;
    config.rule3_min_utterance_length = 30.0;

    OnlineRecognizer::create(&config).ok_or_else(|| anyhow!("无法加载 sherpa-onnx Zipformer 模型"))
}

fn canonical_hotwords(extra_hotwords: &str) -> String {
    let mut prioritized = extra_hotwords
        .lines()
        .filter_map(|line| {
            line.trim().split_once('=').map_or_else(
                || (!line.trim().is_empty()).then(|| line.trim().to_string()),
                |(canonical, _)| {
                    (!canonical.trim().is_empty()).then(|| canonical.trim().to_string())
                },
            )
        })
        .collect::<Vec<_>>();
    for term in hotword_terms(extra_hotwords) {
        if !prioritized
            .iter()
            .any(|item| item.eq_ignore_ascii_case(&term))
        {
            prioritized.push(term);
        }
    }
    prioritized
        .into_iter()
        .take(16)
        .collect::<Vec<_>>()
        .join(",")
}

fn create_refinement_recognizer(
    refinement_model: RefinementModel,
    extra_hotwords: &str,
) -> Result<OfflineRecognizer> {
    let mut config = OfflineRecognizerConfig::default();
    match refinement_model {
        RefinementModel::SenseVoice => {
            let (model, tokens) = refinement_model_files();
            config.model_config.sense_voice.model = Some(model.display().to_string());
            config.model_config.sense_voice.language = Some("auto".to_string());
            config.model_config.sense_voice.use_itn = true;
            config.model_config.tokens = Some(tokens.display().to_string());
            config.model_config.num_threads = 2;
        }
        RefinementModel::Qwen3 => {
            let (frontend, encoder, decoder, tokenizer) = qwen_model_files();
            config.model_config.qwen3_asr.conv_frontend = Some(frontend.display().to_string());
            config.model_config.qwen3_asr.encoder = Some(encoder.display().to_string());
            config.model_config.qwen3_asr.decoder = Some(decoder.display().to_string());
            config.model_config.qwen3_asr.tokenizer = Some(tokenizer.display().to_string());
            config.model_config.qwen3_asr.max_total_len = 512;
            config.model_config.qwen3_asr.max_new_tokens = 256;
            config.model_config.qwen3_asr.temperature = 1e-6;
            config.model_config.qwen3_asr.hotwords = Some(canonical_hotwords(extra_hotwords));
            config.model_config.num_threads = 3;
        }
    }
    config.model_config.provider = Some("cpu".to_string());
    OfflineRecognizer::create(&config)
        .ok_or_else(|| anyhow!("无法加载 sherpa-onnx {} 模型", refinement_model.label()))
}

fn refine_with_recognizer(
    recognizer: &OfflineRecognizer,
    samples: &[f32],
    sample_rate: i32,
) -> String {
    let stream = recognizer.create_stream();
    stream.accept_waveform(sample_rate, samples);
    recognizer.decode(&stream);
    stream
        .get_result()
        .map(|result| result.text.trim().to_string())
        .unwrap_or_default()
}

pub fn transcribe_samples(samples: &[f32], sample_rate: i32) -> Result<String> {
    let recognizer = create_recognizer("")?;
    let stream = recognizer.create_stream();
    let chunk_size = (sample_rate / 10).max(320) as usize;
    for chunk in samples.chunks(chunk_size) {
        stream.accept_waveform(sample_rate, chunk);
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }
    }
    stream.accept_waveform(
        sample_rate,
        &vec![0.0_f32; (sample_rate as f32 * 0.66) as usize],
    );
    while recognizer.is_ready(&stream) {
        recognizer.decode(&stream);
    }
    let text = recognizer
        .get_result(&stream)
        .map(|result| result.text.trim().to_string())
        .unwrap_or_default();
    Ok(normalize_hotword_aliases(&text, &parse_hotwords("")).0)
}

pub fn refine_samples(samples: &[f32], sample_rate: i32, fast_text: &str) -> Result<String> {
    refine_samples_with_model(samples, sample_rate, fast_text, RefinementModel::SenseVoice)
}

pub fn refine_samples_with_model(
    samples: &[f32],
    sample_rate: i32,
    fast_text: &str,
    refinement_model: RefinementModel,
) -> Result<String> {
    let recognizer = create_refinement_recognizer(refinement_model, "")?;
    let refined = refine_with_recognizer(&recognizer, samples, sample_rate);
    let entries = parse_hotwords("");
    Ok(if is_likely_hallucination(&refined) {
        normalize_hotword_aliases(fast_text, &entries).0
    } else {
        select_final_text(fast_text, &refined, &entries)
    })
}

fn normalized_key(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn is_likely_hallucination(text: &str) -> bool {
    matches!(
        normalized_key(text).as_str(),
        "you" | "yeah" | "uh" | "um" | "hmm" | "oh" | "谢谢" | "谢谢观看" | "字幕"
    )
}

fn emit_transcript(
    app: &AppHandle,
    event_name: &str,
    id: &str,
    text: String,
    speaker: &'static str,
    is_final: bool,
) {
    let _ = app.emit(
        event_name,
        TranscriptEvent {
            id: id.to_string(),
            text,
            speaker,
            is_final,
        },
    );
}

fn next_transcript_id(speaker: &str) -> String {
    format!(
        "stt-{speaker}-{}",
        TRANSCRIPT_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn run_refinement_worker(
    app: AppHandle,
    receiver: RefinementReceiver<RefinementJob>,
    hotword_entries: Vec<HotwordEntry>,
    extra_hotwords: String,
    refinement_model: RefinementModel,
    speaker: &'static str,
) {
    let speaker_label = if speaker == "interviewer" {
        "面试官"
    } else {
        "我的回答"
    };
    let model_label = refinement_model.label();
    emit_status(
        &app,
        "loading",
        format!("正在为{speaker_label}加载 {model_label} 最终识别模型…"),
    );
    let recognizer = match create_refinement_recognizer(refinement_model, &extra_hotwords) {
        Ok(recognizer) => recognizer,
        Err(error) => {
            emit_status(&app, "error", error.to_string());
            while let Ok(job) = receiver.recv() {
                let fallback = normalize_hotword_aliases(&job.fast_text, &hotword_entries).0;
                emit_transcript(&app, "transcript-final", &job.id, fallback, speaker, true);
            }
            return;
        }
    };
    emit_status(
        &app,
        "listening",
        format!("{speaker_label}实时识别与 {model_label} 最终校正已就绪"),
    );

    while let Ok(job) = receiver.recv() {
        let started = Instant::now();
        let refined = refine_with_recognizer(&recognizer, &job.samples, job.sample_rate as i32);
        let final_text = if is_likely_hallucination(&refined) {
            normalize_hotword_aliases(&job.fast_text, &hotword_entries).0
        } else {
            select_final_text(&job.fast_text, &refined, &hotword_entries)
        };
        emit_transcript(&app, "transcript-final", &job.id, final_text, speaker, true);
        emit_status(
            &app,
            "listening",
            format!(
                "{model_label} 最终转写完成（{} ms），继续监听",
                started.elapsed().as_millis()
            ),
        );
    }
}

fn submit_refinement(
    app: &AppHandle,
    sender: &SyncSender<RefinementJob>,
    hotword_entries: &[HotwordEntry],
    id: String,
    fast_text: String,
    samples: Vec<f32>,
    sample_rate: u32,
    speaker: &'static str,
) {
    let fallback_id = id.clone();
    let fallback_text = fast_text.clone();
    let job = RefinementJob {
        id,
        samples,
        sample_rate,
        fast_text,
    };
    match sender.try_send(job) {
        Ok(()) => emit_status(app, "transcribing", "高精度模型正在生成最终转写…"),
        Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
            let fallback = normalize_hotword_aliases(&fallback_text, hotword_entries).0;
            emit_transcript(
                app,
                "transcript-final",
                &fallback_id,
                fallback,
                speaker,
                true,
            );
            emit_status(app, "listening", "最终校正队列繁忙，已保留实时转写");
        }
    }
}

/// Runs on a blocking thread: the recognizer stays loaded for the complete
/// capture session and receives the same small PCM chunks produced by Core Audio.
pub fn run_streaming_worker(
    app: AppHandle,
    mut receiver: mpsc::Receiver<AudioChunk>,
    extra_hotwords: String,
    refinement_model: RefinementModel,
    speaker: &'static str,
) {
    let speaker_label = if speaker == "interviewer" {
        "面试官"
    } else {
        "我的回答"
    };
    let hotword_entries = parse_hotwords(&extra_hotwords);
    emit_status(
        &app,
        "loading",
        format!("正在为{speaker_label}加载中英双语流式识别模型…"),
    );
    let recognizer = match create_recognizer(&extra_hotwords) {
        Ok(recognizer) => recognizer,
        Err(error) => {
            emit_status(&app, "error", error.to_string());
            let _ = app.emit("audio-error", error.to_string());
            return;
        }
    };

    let (refinement_sender, refinement_receiver) = sync_channel::<RefinementJob>(4);
    let refinement_app = app.clone();
    let refinement_hotwords = hotword_entries.clone();
    let refinement_extra_hotwords = extra_hotwords.clone();
    let refinement_thread = thread::spawn(move || {
        run_refinement_worker(
            refinement_app,
            refinement_receiver,
            refinement_hotwords,
            refinement_extra_hotwords,
            refinement_model,
            speaker,
        )
    });

    let mut stream = recognizer.create_stream();
    let mut transcript_id = next_transcript_id(speaker);
    let mut last_partial = String::new();
    let mut last_sample_rate = 16_000_u32;
    let mut utterance_audio = Vec::<f32>::new();
    emit_status(&app, "listening", format!("{speaker_label}流式识别已就绪"));

    while let Some(chunk) = receiver.blocking_recv() {
        last_sample_rate = chunk.sample_rate;
        utterance_audio.extend_from_slice(&chunk.samples);
        stream.accept_waveform(chunk.sample_rate as i32, &chunk.samples);
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }

        let raw_text = recognizer
            .get_result(&stream)
            .map(|result| result.text.trim().to_string())
            .unwrap_or_default();
        let text = normalize_hotword_aliases(&raw_text, &hotword_entries).0;

        if !text.is_empty() && text != last_partial && !is_likely_hallucination(&text) {
            emit_transcript(
                &app,
                "transcript-partial",
                &transcript_id,
                text.clone(),
                speaker,
                false,
            );
            last_partial = text.clone();
        }

        if recognizer.is_endpoint(&stream) {
            if !text.is_empty() && !is_likely_hallucination(&text) {
                submit_refinement(
                    &app,
                    &refinement_sender,
                    &hotword_entries,
                    transcript_id.clone(),
                    text,
                    std::mem::take(&mut utterance_audio),
                    last_sample_rate,
                    speaker,
                );
            } else {
                utterance_audio.clear();
            }
            recognizer.reset(&stream);
            stream = recognizer.create_stream();
            transcript_id = next_transcript_id(speaker);
            last_partial.clear();
        }
    }

    // Stopping capture can cut off the model's right context. Padding here
    // flushes the final hypothesis without changing the live endpoint latency.
    stream.accept_waveform(
        last_sample_rate as i32,
        &vec![0.0_f32; (last_sample_rate as f32 * 0.66) as usize],
    );
    while recognizer.is_ready(&stream) {
        recognizer.decode(&stream);
    }
    if let Some(result) = recognizer.get_result(&stream) {
        let text = result.text.trim().to_string();
        if !text.is_empty() && !is_likely_hallucination(&text) {
            submit_refinement(
                &app,
                &refinement_sender,
                &hotword_entries,
                transcript_id,
                normalize_hotword_aliases(&text, &hotword_entries).0,
                utterance_audio,
                last_sample_rate,
                speaker,
            );
        }
    }
    drop(refinement_sender);
    let _ = refinement_thread.join();
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_hotwords, is_likely_hallucination, normalized_key, parse_hotwords,
        select_final_text,
    };

    #[test]
    fn creates_language_independent_deduplication_keys() {
        assert_eq!(normalized_key("Hello, World!"), "helloworld");
        assert_eq!(normalized_key("请介绍一下。"), "请介绍一下");
    }

    #[test]
    fn filters_short_tail_hallucinations() {
        assert!(is_likely_hallucination(" you. "));
        assert!(is_likely_hallucination("谢谢观看！"));
        assert!(!is_likely_hallucination("How would you design this API?"));
    }

    #[test]
    fn keeps_fast_candidate_when_it_preserves_more_hotwords() {
        let entries = parse_hotwords("");
        assert_eq!(
            select_final_text(
                "请介绍一下高粱的鸡一模型",
                "请介绍一下高亮的GE模型。",
                &entries,
            ),
            "请介绍一下Golang的GMP模型"
        );
    }

    #[test]
    fn limits_qwen_prompt_and_prioritizes_user_hotwords() {
        let hotwords = canonical_hotwords("用户自定义词\n另一个词=错误词")
            .split(',')
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(hotwords[0], "用户自定义词");
        assert_eq!(hotwords[1], "另一个词");
        assert!(hotwords.len() <= 16);
    }
}
