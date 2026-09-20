import { invoke } from "@tauri-apps/api/core";
import { normalizeOpenAIBaseUrl } from "./provider";
import type { ProviderConfig, TranslationConfig } from "./types";

export function normalizeOllamaBaseUrl(value: string): string {
  const normalized = value.trim().replace(/\/+$/, "");
  if (!normalized) throw new Error("Ollama 地址不能为空");
  return normalized;
}

export async function translateToChinese(
  text: string,
  config: TranslationConfig,
  provider: ProviderConfig,
): Promise<string> {
  if (config.mode === "off") throw new Error("实时翻译未开启");
  if (!("__TAURI_INTERNALS__" in window)) {
    throw new Error("翻译只能在 Tauri 桌面应用中调用");
  }

  return invoke<string>("translate_text", {
    request: {
      mode: config.mode,
      text,
      deepseek: {
        baseUrl: normalizeOpenAIBaseUrl(provider.baseUrl),
        apiKey: provider.apiKey,
        model: provider.model,
        disableThinking: provider.disableThinking,
      },
      ollama: {
        baseUrl: normalizeOllamaBaseUrl(config.ollamaBaseUrl),
        model: config.ollamaModel,
      },
    },
  });
}
