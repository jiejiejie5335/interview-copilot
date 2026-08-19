use serde::{Deserialize, Serialize};

const KEYCHAIN_SERVICE: &str = "com.interviewcopilot.desktop";
const KEYCHAIN_ITEM_NOT_FOUND: i32 = -25_300;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretKind {
    DeepseekApiKey,
    TencentSecretKey,
}

impl SecretKind {
    fn account(self) -> &'static str {
        match self {
            Self::DeepseekApiKey => "deepseek-api-key",
            Self::TencentSecretKey => "tencent-secret-key",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretStatus {
    deepseek_api_key: bool,
    tencent_secret_key: bool,
}

#[cfg(target_os = "macos")]
pub(crate) fn load_secret(kind: SecretKind) -> Result<Option<String>, String> {
    use security_framework::passwords::get_generic_password;

    match get_generic_password(KEYCHAIN_SERVICE, kind.account()) {
        Ok(value) => String::from_utf8(value)
            .map(Some)
            .map_err(|_| "钥匙串中的密钥不是有效文本".to_string()),
        Err(error) if error.code() == KEYCHAIN_ITEM_NOT_FOUND => Ok(None),
        Err(error) => Err(format!("无法读取 macOS 钥匙串：{error}")),
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn load_secret(_kind: SecretKind) -> Result<Option<String>, String> {
    Err("当前版本仅支持使用 macOS 钥匙串保存密钥".to_string())
}

#[cfg(target_os = "macos")]
fn store_secret(kind: SecretKind, value: &str) -> Result<(), String> {
    security_framework::passwords::set_generic_password(
        KEYCHAIN_SERVICE,
        kind.account(),
        value.as_bytes(),
    )
    .map_err(|error| format!("无法写入 macOS 钥匙串：{error}"))
}

#[cfg(not(target_os = "macos"))]
fn store_secret(_kind: SecretKind, _value: &str) -> Result<(), String> {
    Err("当前版本仅支持使用 macOS 钥匙串保存密钥".to_string())
}

#[cfg(target_os = "macos")]
fn remove_secret(kind: SecretKind) -> Result<(), String> {
    match security_framework::passwords::delete_generic_password(
        KEYCHAIN_SERVICE,
        kind.account(),
    ) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == KEYCHAIN_ITEM_NOT_FOUND => Ok(()),
        Err(error) => Err(format!("无法从 macOS 钥匙串删除密钥：{error}")),
    }
}

#[cfg(not(target_os = "macos"))]
fn remove_secret(_kind: SecretKind) -> Result<(), String> {
    Err("当前版本仅支持使用 macOS 钥匙串保存密钥".to_string())
}

#[tauri::command]
pub fn get_secret_status() -> Result<SecretStatus, String> {
    Ok(SecretStatus {
        deepseek_api_key: load_secret(SecretKind::DeepseekApiKey)?.is_some(),
        tencent_secret_key: load_secret(SecretKind::TencentSecretKey)?.is_some(),
    })
}

#[tauri::command]
pub fn save_secret(kind: SecretKind, value: String) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("不能保存空密钥".to_string());
    }
    store_secret(kind, value)
}

#[tauri::command]
pub fn delete_secret(kind: SecretKind) -> Result<(), String> {
    remove_secret(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keychain_accounts_are_stable_and_separate() {
        assert_eq!(SecretKind::DeepseekApiKey.account(), "deepseek-api-key");
        assert_eq!(SecretKind::TencentSecretKey.account(), "tencent-secret-key");
    }
}
