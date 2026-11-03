use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

#[cfg(desktop)]
use tauri::Manager;

#[cfg(desktop)]
use super::companion::CompanionRuntime;
use super::secrets::{load_secret, SecretKind};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamChatRequest {
    request_id: String,
    provider: ProviderConfig,
    messages: Vec<ChatMessage>,
    question: String,
    answer_language: String,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderConfig {
    #[allow(dead_code)]
    name: String,
    base_url: String,
    api_key: String,
    model: String,
    disable_thinking: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LlmStreamEvent {
    request_id: String,
    delta: String,
    done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn emit_stream(
    app: &AppHandle,
    request_id: &str,
    delta: impl Into<String>,
    done: bool,
    error: Option<String>,
) {
    let delta = delta.into();
    #[cfg(desktop)]
    if let Some(runtime) = app.try_state::<CompanionRuntime>() {
        runtime.apply_stream(request_id, &delta, done, error.as_deref());
    }
    let _ = app.emit(
        "llm-stream",
        LlmStreamEvent {
            request_id: request_id.to_owned(),
            delta,
            done,
            error,
        },
    );
}

fn event_separator(buffer: &[u8]) -> Option<(usize, usize)> {
    for index in 0..buffer.len().saturating_sub(1) {
        if buffer[index..].starts_with(b"\r\n\r\n") {
            return Some((index, 4));
        }
        if buffer[index..].starts_with(b"\n\n") {
            return Some((index, 2));
        }
    }
    None
}

fn data_payload(event: &[u8]) -> Result<Option<String>, String> {
    let text = std::str::from_utf8(event).map_err(|_| "模型返回了无效的 UTF-8 数据".to_string())?;
    let data = text
        .lines()
        .filter_map(|line| line.trim_end_matches('\r').strip_prefix("data:"))
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join("\n");

    if data.is_empty() {
        Ok(None)
    } else {
        Ok(Some(data))
    }
}

fn content_delta(data: &str) -> Result<Option<String>, String> {
    let value: Value =
        serde_json::from_str(data).map_err(|error| format!("无法解析模型流式响应：{error}"))?;

    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("模型服务返回错误");
        return Err(message.to_string());
    }

    Ok(value
        .pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
        .map(str::to_owned))
}

#[tauri::command]
pub async fn stream_chat(app: AppHandle, mut request: StreamChatRequest) -> Result<(), String> {
    let request_id = request.request_id.clone();
    if request.provider.api_key.trim().is_empty() {
        request.provider.api_key = load_secret(SecretKind::DeepseekApiKey)?
            .ok_or_else(|| "请先保存 DeepSeek API Key".to_string())?;
    }

    #[cfg(desktop)]
    if let Some(runtime) = app.try_state::<CompanionRuntime>() {
        runtime.begin_answer(
            &request_id,
            request.question.trim(),
            request.answer_language.trim(),
        );
    }

    let endpoint = format!(
        "{}/chat/completions",
        request.provider.base_url.trim_end_matches('/')
    );
    let mut payload = json!({
        "model": request.provider.model,
        "messages": request.messages,
        "temperature": request.temperature,
        "max_tokens": request.max_tokens,
        "stream": true
    });
    if request.provider.disable_thinking {
        payload["thinking"] = json!({ "type": "disabled" });
    }

    let response = match reqwest::Client::new()
        .post(endpoint)
        .bearer_auth(request.provider.api_key.trim())
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            let message = format!("无法连接模型服务：{error}");
            emit_stream(&app, &request_id, "", true, Some(message));
            return Ok(());
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let detail = response
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(1200)
            .collect::<String>();
        let message = if detail.is_empty() {
            format!("模型服务返回 HTTP {status}")
        } else {
            format!("模型服务返回 HTTP {status}：{detail}")
        };
        emit_stream(&app, &request_id, "", true, Some(message));
        return Ok(());
    }

    let mut stream = response.bytes_stream();
    let mut buffer = Vec::<u8>::new();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                emit_stream(
                    &app,
                    &request_id,
                    "",
                    true,
                    Some(format!("读取模型响应失败：{error}")),
                );
                return Ok(());
            }
        };
        buffer.extend_from_slice(&chunk);

        while let Some((index, separator_len)) = event_separator(&buffer) {
            let event = buffer[..index].to_vec();
            buffer.drain(..index + separator_len);
            let Some(data) = data_payload(&event)? else {
                continue;
            };
            if data == "[DONE]" {
                emit_stream(&app, &request_id, "", true, None);
                return Ok(());
            }
            match content_delta(&data) {
                Ok(Some(delta)) if !delta.is_empty() => {
                    emit_stream(&app, &request_id, delta, false, None);
                }
                Ok(_) => {}
                Err(error) => {
                    emit_stream(&app, &request_id, "", true, Some(error));
                    return Ok(());
                }
            }
        }
    }

    emit_stream(&app, &request_id, "", true, None);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{content_delta, data_payload, event_separator};

    #[test]
    fn parses_openai_sse_delta() {
        let event = br#"data: {"choices":[{"delta":{"content":"hello"}}]}"#;
        let payload = data_payload(event).unwrap().unwrap();
        assert_eq!(content_delta(&payload).unwrap().as_deref(), Some("hello"));
    }

    #[test]
    fn recognizes_lf_and_crlf_boundaries() {
        assert_eq!(event_separator(b"data: x\n\nnext"), Some((7, 2)));
        assert_eq!(event_separator(b"data: x\r\n\r\nnext"), Some((7, 4)));
    }
}
