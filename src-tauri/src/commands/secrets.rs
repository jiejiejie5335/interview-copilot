use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

const KEYCHAIN_SERVICE: &str = "com.interviewcopilot.desktop";
const KEYCHAIN_ITEM_NOT_FOUND: i32 = -25_300;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SecretKind {
    DeepseekApiKey,
    TencentSecretKey,
}

static SECRET_CACHE: OnceLock<Mutex<HashMap<SecretKind, String>>> = OnceLock::new();

fn secret_cache() -> &'static Mutex<HashMap<SecretKind, String>> {
    SECRET_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_secret(kind: SecretKind) -> Result<Option<String>, String> {
    secret_cache()
        .lock()
        .map(|cache| cache.get(&kind).cloned())
        .map_err(|_| "密钥内存缓存状态异常".to_string())
}

fn cache_secret(kind: SecretKind, value: Option<&str>) -> Result<(), String> {
    let mut cache = secret_cache()
        .lock()
        .map_err(|_| "密钥内存缓存状态异常".to_string())?;
    if let Some(value) = value {
        cache.insert(kind, value.to_string());
    } else {
        cache.remove(&kind);
    }
    Ok(())
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

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) fn load_secret(kind: SecretKind) -> Result<Option<String>, String> {
    use security_framework::passwords::get_generic_password;

    if let Some(value) = cached_secret(kind)? {
        return Ok(Some(value));
    }

    match get_generic_password(KEYCHAIN_SERVICE, kind.account()) {
        Ok(value) => {
            let value =
                String::from_utf8(value).map_err(|_| "钥匙串中的密钥不是有效文本".to_string())?;
            cache_secret(kind, Some(&value))?;
            Ok(Some(value))
        }
        Err(error) if error.code() == KEYCHAIN_ITEM_NOT_FOUND => Ok(None),
        Err(error) => Err(format!("无法读取系统钥匙串：{error}")),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
pub(crate) fn load_secret(_kind: SecretKind) -> Result<Option<String>, String> {
    Err("当前平台暂不支持系统钥匙串".to_string())
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn store_secret(kind: SecretKind, value: &str) -> Result<(), String> {
    security_framework::passwords::set_generic_password(
        KEYCHAIN_SERVICE,
        kind.account(),
        value.as_bytes(),
    )
    .map_err(|error| format!("无法写入系统钥匙串：{error}"))?;
    cache_secret(kind, Some(value))
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn store_secret(_kind: SecretKind, _value: &str) -> Result<(), String> {
    Err("当前平台暂不支持系统钥匙串".to_string())
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn remove_secret(kind: SecretKind) -> Result<(), String> {
    let result = match security_framework::passwords::delete_generic_password(
        KEYCHAIN_SERVICE,
        kind.account(),
    ) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == KEYCHAIN_ITEM_NOT_FOUND => Ok(()),
        Err(error) => Err(format!("无法从系统钥匙串删除密钥：{error}")),
    };
    result?;
    cache_secret(kind, None)
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn remove_secret(_kind: SecretKind) -> Result<(), String> {
    Err("当前平台暂不支持系统钥匙串".to_string())
}

#[cfg(target_os = "macos")]
fn secret_exists(kind: SecretKind) -> Result<bool, String> {
    use security_framework::item::{ItemClass, ItemSearchOptions};

    let mut search = ItemSearchOptions::new();
    search
        .class(ItemClass::generic_password())
        .service(KEYCHAIN_SERVICE)
        .account(kind.account())
        .load_attributes(true)
        // Status checks must never show an authentication dialog. Secret data
        // is requested only when a provider actually needs it.
        .skip_authenticated_items(true);
    match search.search() {
        Ok(items) => Ok(!items.is_empty()),
        Err(error) if error.code() == KEYCHAIN_ITEM_NOT_FOUND => Ok(false),
        Err(error) => Err(format!("无法检查 macOS 钥匙串状态：{error}")),
    }
}

#[cfg(target_os = "ios")]
fn secret_exists(kind: SecretKind) -> Result<bool, String> {
    // iOS generic-password items created here do not use biometric access
    // control, so this lookup does not display an authentication prompt.
    load_secret(kind).map(|value| value.is_some())
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn secret_exists(_kind: SecretKind) -> Result<bool, String> {
    Err("当前平台暂不支持系统钥匙串".to_string())
}

#[tauri::command]
pub fn get_secret_status() -> Result<SecretStatus, String> {
    Ok(SecretStatus {
        deepseek_api_key: secret_exists(SecretKind::DeepseekApiKey)?,
        tencent_secret_key: secret_exists(SecretKind::TencentSecretKey)?,
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
