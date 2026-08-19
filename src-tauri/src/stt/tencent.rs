use std::{collections::BTreeMap, time::{Duration, SystemTime, UNIX_EPOCH}};

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::Sha1;
use tauri::{AppHandle, Emitter};
use tokio::{sync::mpsc, time::timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::form_urlencoded::byte_serialize;

use super::{sherpa::{hotword_terms, normalize_transcript}, AudioChunk};

const TARGET_SAMPLE_RATE: u32 = 16_000;
const PACKET_SAMPLES: usize = 640;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TencentAsrConfig {
    pub app_id: String,
    pub secret_id: String,
    pub secret_key: String,
}

impl TencentAsrConfig {
    pub fn validate(&self) -> Result<()> {
        if self.app_id.trim().is_empty()
            || self.secret_id.trim().is_empty()
            || self.secret_key.trim().is_empty()
        {
            Err(anyhow!("腾讯云模式需要填写 AppID、SecretId 和 SecretKey"))
        } else if !self.app_id.chars().all(|character| character.is_ascii_digit()) {
            Err(anyhow!("腾讯云 AppID 应为纯数字"))
        } else {
            Ok(())
        }
    }
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

#[derive(Default)]
struct LinearResampler {
    source_rate: u32,
    previous: Option<f32>,
    position: f64,
}

impl LinearResampler {
    fn process(&mut self, input: &[f32], source_rate: u32) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }
        if source_rate == TARGET_SAMPLE_RATE {
            return input.to_vec();
        }
        if self.source_rate != source_rate {
            self.source_rate = source_rate;
            self.previous = None;
            self.position = 0.0;
        }

        let mut source = Vec::with_capacity(input.len() + 1);
        source.push(self.previous.unwrap_or(input[0]));
        source.extend_from_slice(input);
        let step = source_rate as f64 / TARGET_SAMPLE_RATE as f64;
        let mut output = Vec::with_capacity((input.len() as f64 / step).ceil() as usize + 1);
        while self.position + 1.0 < source.len() as f64 {
            let index = self.position.floor() as usize;
            let fraction = (self.position - index as f64) as f32;
            output.push(source[index] + (source[index + 1] - source[index]) * fraction);
            self.position += step;
        }
        self.position -= (source.len() - 1) as f64;
        self.previous = input.last().copied();
        output
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

fn unix_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("系统时间早于 UNIX epoch")?
        .as_secs())
}

fn build_hotword_list(hotwords: &str) -> String {
    hotword_terms(hotwords)
        .into_iter()
        .filter_map(|term| {
            let term = term.replace(char::is_whitespace, "");
            (!term.is_empty() && term.chars().count() <= 30).then(|| format!("{term}|10"))
        })
        .take(128)
        .collect::<Vec<_>>()
        .join(",")
}

fn build_query(parameters: &BTreeMap<String, String>) -> String {
    parameters
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn build_signed_url(
    config: &TencentAsrConfig,
    hotwords: &str,
    timestamp: u64,
    nonce: u64,
    voice_id: &str,
) -> Result<String> {
    let mut parameters = BTreeMap::from([
        ("convert_num_mode".to_string(), "1".to_string()),
        ("domain".to_string(), "1".to_string()),
        ("engine_model_type".to_string(), "16k_zh_en_2.0".to_string()),
        ("expired".to_string(), (timestamp + 86_400).to_string()),
        ("needvad".to_string(), "1".to_string()),
        ("nonce".to_string(), nonce.to_string()),
        ("result_mod".to_string(), "1".to_string()),
        ("secretid".to_string(), config.secret_id.trim().to_string()),
        ("sentence_strategy".to_string(), "0".to_string()),
        ("timestamp".to_string(), timestamp.to_string()),
        ("voice_format".to_string(), "1".to_string()),
        ("voice_id".to_string(), voice_id.to_string()),
    ]);
    let cloud_hotwords = build_hotword_list(hotwords);
    if !cloud_hotwords.is_empty() {
        parameters.insert("hotword_list".to_string(), cloud_hotwords);
    }

    let query = build_query(&parameters);
    let sign_source = format!(
        "asr.cloud.tencent.com/asr/v2/{}?{}",
        config.app_id.trim(),
        query
    );
    let mut signer = Hmac::<Sha1>::new_from_slice(config.secret_key.trim().as_bytes())
        .map_err(|_| anyhow!("无法初始化腾讯云签名"))?;
    signer.update(sign_source.as_bytes());
    let signature = BASE64.encode(signer.finalize().into_bytes());
    let encoded_signature = byte_serialize(signature.as_bytes()).collect::<String>();
    Ok(format!(
        "wss://asr.cloud.tencent.com/asr/v2/{}?{}&signature={}",
        config.app_id.trim(),
        query,
        encoded_signature,
    ))
}

fn message_error(value: &Value) -> Result<()> {
    let code = value.get("code").and_then(Value::as_i64).unwrap_or(-1);
    if code == 0 {
        Ok(())
    } else {
        let message = value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("未知错误");
        Err(anyhow!("腾讯云 ASR 错误 {code}：{message}"))
    }
}

fn sentence_values(value: &Value) -> Vec<&Value> {
    match value.get("sentences") {
        Some(Value::Array(sentences)) => sentences.iter().collect(),
        Some(Value::Object(sentences)) => match sentences.get("sentence_list") {
            Some(Value::Array(sentence_list)) => sentence_list.iter().collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn handle_response(app: &AppHandle, value: &Value, hotwords: &str, voice_id: &str) -> Result<bool> {
    message_error(value)?;
    for sentence in sentence_values(value) {
        let text = sentence
            .get("sentence")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if text.is_empty() {
            continue;
        }
        let sentence_id = sentence
            .get("sentence_id")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let is_final = sentence
            .get("sentence_type")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            == 1;
        let event_name = if is_final {
            "transcript-final"
        } else {
            "transcript-partial"
        };
        let _ = app.emit(
            event_name,
            TranscriptEvent {
                id: format!("tencent-{voice_id}-{sentence_id}"),
                text: normalize_transcript(text, hotwords),
                speaker: "interviewer",
                is_final,
            },
        );
    }
    Ok(value.get("final").and_then(Value::as_i64) == Some(1))
}

fn samples_to_pcm(samples: &[f32]) -> Vec<u8> {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        pcm.extend_from_slice(
            &((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes(),
        );
    }
    pcm
}

pub async fn run_tencent_streaming_worker(
    app: AppHandle,
    mut receiver: mpsc::Receiver<AudioChunk>,
    config: TencentAsrConfig,
    hotwords: String,
) -> Result<()> {
    config.validate()?;
    emit_status(&app, "loading", "正在连接腾讯云 ASR 大模型 2.0…");
    let timestamp = unix_seconds()?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .subsec_nanos() as u64;
    let voice_id = format!("interview-{timestamp}-{nonce}");
    let url = build_signed_url(&config, &hotwords, timestamp, nonce, &voice_id)?;
    let (mut websocket, _) = connect_async(url)
        .await
        .context("无法连接腾讯云 ASR WebSocket")?;

    let handshake = timeout(Duration::from_secs(10), websocket.next())
        .await
        .context("腾讯云 ASR 握手超时")?
        .ok_or_else(|| anyhow!("腾讯云 ASR 在握手前关闭连接"))??;
    if let Message::Text(text) = handshake {
        let value: Value = serde_json::from_str(text.as_ref()).context("腾讯云握手响应格式异常")?;
        message_error(&value)?;
    } else {
        return Err(anyhow!("腾讯云 ASR 未返回握手确认"));
    }
    emit_status(&app, "listening", "腾讯云 ASR 大模型 2.0 已连接，正在实时识别");

    let mut resampler = LinearResampler::default();
    let mut buffered_samples = Vec::<f32>::new();
    loop {
        tokio::select! {
            chunk = receiver.recv() => {
                let Some(chunk) = chunk else { break; };
                buffered_samples.extend(resampler.process(&chunk.samples, chunk.sample_rate));
                while buffered_samples.len() >= PACKET_SAMPLES {
                    let packet = buffered_samples.drain(..PACKET_SAMPLES).collect::<Vec<_>>();
                    websocket.send(Message::Binary(samples_to_pcm(&packet).into())).await?;
                }
            }
            message = websocket.next() => {
                let Some(message) = message else {
                    return Err(anyhow!("腾讯云 ASR 意外关闭连接"));
                };
                match message? {
                    Message::Text(text) => {
                        let value: Value = serde_json::from_str(text.as_ref())?;
                        if handle_response(&app, &value, &hotwords, &voice_id)? {
                            return Ok(());
                        }
                    }
                    Message::Ping(data) => websocket.send(Message::Pong(data)).await?,
                    Message::Close(_) => return Err(anyhow!("腾讯云 ASR 提前关闭连接")),
                    _ => {}
                }
            }
        }
    }

    if !buffered_samples.is_empty() {
        websocket
            .send(Message::Binary(samples_to_pcm(&buffered_samples).into()))
            .await?;
    }
    websocket
        .send(Message::Text(r#"{"type":"end"}"#.into()))
        .await?;
    loop {
        let message = timeout(Duration::from_secs(20), websocket.next())
            .await
            .context("等待腾讯云最终结果超时")?;
        let Some(message) = message else { break; };
        match message? {
            Message::Text(text) => {
                let value: Value = serde_json::from_str(text.as_ref())?;
                if handle_response(&app, &value, &hotwords, &voice_id)? {
                    break;
                }
            }
            Message::Ping(data) => websocket.send(Message::Pong(data)).await?,
            Message::Close(_) => break,
            _ => {}
        }
    }
    emit_status(&app, "listening", "腾讯云 ASR 已完成当前音频流");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_hotword_list, build_query, build_signed_url, samples_to_pcm, sentence_values,
        TencentAsrConfig,
    };
    use serde_json::json;
    use std::collections::BTreeMap;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    #[test]
    fn converts_float_samples_to_little_endian_pcm16() {
        assert_eq!(samples_to_pcm(&[0.0, 1.0, -1.0]), vec![0, 0, 255, 127, 1, 128]);
    }

    #[test]
    fn produces_tencent_hotword_syntax() {
        let hotwords = build_hotword_list("Golang=高粱|高亮\nGMP\nSpring Boot");
        assert!(hotwords.contains("Golang|10"));
        assert!(hotwords.contains("GMP|10"));
        assert!(hotwords.contains("SpringBoot|10"));
    }

    #[test]
    fn sorts_query_parameters_before_signing() {
        let parameters = BTreeMap::from([
            ("voice_id".to_string(), "a+b".to_string()),
            ("engine_model_type".to_string(), "16k_zh_en_2.0".to_string()),
        ]);
        assert_eq!(
            build_query(&parameters),
            "engine_model_type=16k_zh_en_2.0&voice_id=a+b"
        );
    }

    #[test]
    fn reads_v2_sentence_list_response_shape() {
        let value = json!({
            "sentences": {
                "sentence_list": [{"sentence": "你好", "sentence_id": 1, "sentence_type": 1}]
            }
        });
        assert_eq!(sentence_values(&value)[0]["sentence"], "你好");
    }

    #[test]
    fn signed_url_with_chinese_hotwords_is_a_valid_websocket_request() {
        let url = build_signed_url(
            &TencentAsrConfig {
                app_id: "123456".to_string(),
                secret_id: "secret-id".to_string(),
                secret_key: "secret-key".to_string(),
            },
            "Golang\n高并发",
            1_700_000_000,
            123,
            "voice-id",
        )
        .unwrap();
        assert!(url.into_client_request().is_ok());
    }
}
