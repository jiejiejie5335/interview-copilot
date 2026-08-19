import { invoke } from "@tauri-apps/api/core";
import type { StreamChatRequest } from "./types";

export function normalizeOpenAIBaseUrl(value: string): string {
  const trimmed = value.trim().replace(/\/+$/, "");
  if (!trimmed) throw new Error("Base URL 不能为空");
  return trimmed.endsWith("/v1") ? trimmed : `${trimmed}/v1`;
}

export async function startChatStream(request: StreamChatRequest): Promise<void> {
  if (!("__TAURI_INTERNALS__" in window)) {
    throw new Error("流式 Provider 只能在 Tauri 桌面应用中调用");
  }

  await invoke("stream_chat", {
    request: {
      ...request,
      provider: {
        ...request.provider,
        baseUrl: normalizeOpenAIBaseUrl(request.provider.baseUrl),
      },
    },
  });
}
