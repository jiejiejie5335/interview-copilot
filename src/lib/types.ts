export type DetectedLanguage = "zh" | "en" | "mixed" | "unknown";
export type AnswerLanguage = "zh" | "en";

export interface QuestionDetection {
  isQuestion: boolean;
  confidence: number;
  language: DetectedLanguage;
  normalizedText: string;
  reason: string;
}

export interface TranscriptEntry {
  id: string;
  speaker: "interviewer" | "candidate";
  text: string;
  language: DetectedLanguage;
  timestamp: number;
  isFinal: boolean;
  translation?: string;
  translationStatus?: "pending" | "complete" | "error";
  translationError?: string;
  translationKey?: string;
}

export type TranslationMode = "off" | "deepseek" | "ollama";

export interface TranslationConfig {
  mode: TranslationMode;
  ollamaBaseUrl: string;
  ollamaModel: string;
}

export interface ProviderConfig {
  name: string;
  baseUrl: string;
  apiKey: string;
  model: string;
  disableThinking: boolean;
}

export interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

export interface StreamChatRequest {
  requestId: string;
  provider: ProviderConfig;
  messages: ChatMessage[];
  temperature: number;
  maxTokens: number;
}

export interface LlmStreamEvent {
  requestId: string;
  delta: string;
  done: boolean;
  error?: string;
}

export interface AudioLevelEvent {
  rms: number;
  peak: number;
  sampleRate: number;
  speaker: "interviewer" | "candidate";
}

export interface TranscriptFinalEvent {
  id: string;
  text: string;
  speaker: "interviewer" | "candidate";
  isFinal: boolean;
}

export type TranscriptPartialEvent = TranscriptFinalEvent;

export interface SttStatusEvent {
  state: "loading" | "listening" | "transcribing" | "error";
  message: string;
}

export type SttMode = "localFast" | "localAccurate" | "tencentCloud";

export interface TencentAsrConfig {
  appId: string;
  secretId: string;
  secretKey: string;
}

export interface SttConfig {
  mode: SttMode;
  hotwords: string;
  tencent: TencentAsrConfig;
}
