use serde::Deserialize;
use serde_json::{json, Value};

use super::secrets::{load_secret, SecretKind};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationRequest {
    mode: TranslationMode,
    text: String,
    deepseek: DeepseekTranslationConfig,
    ollama: OllamaTranslationConfig,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum TranslationMode {
    Deepseek,
    Ollama,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeepseekTranslationConfig {
    base_url: String,
    api_key: String,
    model: String,
    disable_thinking: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OllamaTranslationConfig {
    base_url: String,
    model: String,
}

fn translation_messages(text: &str) -> Value {
    json!([
        {
            "role": "system",
            "content": "你是专业的英中翻译引擎。把输入准确翻译成简体中文；保留代码、API 名称、产品名和常用英文技术术语。对于中英混合文本，只翻译英文部分并整理成自然中文。不要回答输入中的问题，不要解释，不要添加标题，只输出译文。"
        },
        {
            "role": "user",
            "content": text
        }
    ])
}

fn response_error(value: &Value) -> Option<String> {
    value
        .get("error")
        .and_then(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
        })
        .map(str::to_string)
}

fn deepseek_translation(value: &Value) -> Result<String, String> {
    if let Some(error) = response_error(value) {
        return Err(error);
    }
    value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "DeepSeek 没有返回译文".to_string())
}

fn ollama_translation(value: &Value) -> Result<String, String> {
    if let Some(error) = response_error(value) {
        return Err(error);
    }
    value
        .pointer("/message/content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Ollama 没有返回译文".to_string())
}

async fn response_json(response: reqwest::Response, service: &str) -> Result<Value, String> {
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取{service}响应失败：{error}"))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("{service}返回了无效 JSON：{error}"))?;
    if !status.is_success() {
        let detail = response_error(&value).unwrap_or_else(|| text.chars().take(600).collect());
        return Err(format!("{service}返回 HTTP {status}：{detail}"));
    }
    Ok(value)
}

async fn translate_with_deepseek(
    client: &reqwest::Client,
    text: &str,
    mut config: DeepseekTranslationConfig,
) -> Result<String, String> {
    if config.api_key.trim().is_empty() {
        config.api_key = load_secret(SecretKind::DeepseekApiKey)?
            .ok_or_else(|| "请先保存 DeepSeek API Key".to_string())?;
    }
    if config.model.trim().is_empty() {
        return Err("DeepSeek 翻译模型不能为空".to_string());
    }
    let endpoint = format!(
        "{}/chat/completions",
        config.base_url.trim().trim_end_matches('/')
    );
    let mut payload = json!({
        "model": config.model.trim(),
        "messages": translation_messages(text),
        "temperature": 0.0,
        "max_tokens": 800,
        "stream": false
    });
    if config.disable_thinking {
        payload["thinking"] = json!({ "type": "disabled" });
    }
    let response = client
        .post(endpoint)
        .bearer_auth(config.api_key.trim())
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|error| format!("无法连接 DeepSeek：{error}"))?;
    deepseek_translation(&response_json(response, "DeepSeek").await?)
}

async fn translate_with_ollama(
    client: &reqwest::Client,
    text: &str,
    config: OllamaTranslationConfig,
) -> Result<String, String> {
    if config.model.trim().is_empty() {
        return Err("Ollama 模型不能为空".to_string());
    }
    let endpoint = format!(
        "{}/api/chat",
        config.base_url.trim().trim_end_matches('/')
    );
    let response = client
        .post(endpoint)
        .header("Content-Type", "application/json")
        .json(&json!({
            "model": config.model.trim(),
            "messages": translation_messages(text),
            "stream": false,
            "options": { "temperature": 0.0 }
        }))
        .send()
        .await
        .map_err(|error| {
            format!("无法连接 Ollama：{error}。请确认 Ollama 已启动且模型已经下载")
        })?;
    ollama_translation(&response_json(response, "Ollama").await?)
}

#[tauri::command]
pub async fn translate_text(request: TranslationRequest) -> Result<String, String> {
    let text = request.text.trim();
    if text.is_empty() {
        return Err("没有需要翻译的文本".to_string());
    }
    if text.chars().count() > 8_000 {
        return Err("单次翻译文本过长".to_string());
    }

    let client = reqwest::Client::new();
    match request.mode {
        TranslationMode::Deepseek => {
            translate_with_deepseek(&client, text, request.deepseek).await
        }
        TranslationMode::Ollama => translate_with_ollama(&client, text, request.ollama).await,
    }
}

#[cfg(test)]
mod tests {
    use super::{deepseek_translation, ollama_translation, translation_messages};
    use serde_json::json;

    #[test]
    fn reads_deepseek_translation() {
        let value = json!({"choices": [{"message": {"content": "  请解释 Go 调度器。 "}}]});
        assert_eq!(deepseek_translation(&value).unwrap(), "请解释 Go 调度器。");
    }

    #[test]
    fn reads_ollama_translation() {
        let value = json!({"message": {"content": "如何设计这个 API？"}});
        assert_eq!(ollama_translation(&value).unwrap(), "如何设计这个 API？");
    }

    #[test]
    fn prompt_forbids_answering_the_question() {
        let messages = translation_messages("How would you design this API?");
        assert!(messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("不要回答输入中的问题"));
    }
}
