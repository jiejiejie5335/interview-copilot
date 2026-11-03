import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import { detectQuestion } from "./lib/questionDetector";
import { startChatStream } from "./lib/provider";
import { buildInterviewMessages } from "./lib/prompts";
import { translateToChinese } from "./lib/translation";
import type {
  AnswerLanguage,
  AudioSourceSpeaker,
  AudioLevelEvent,
  CaptureStarted,
  CompanionServerInfo,
  LlmStreamEvent,
  ProviderConfig,
  RuntimeInfo,
  SttStatusEvent,
  SttConfig,
  TranscriptEntry,
  TranscriptFinalEvent,
  TranscriptPartialEvent,
  TranslationConfig,
} from "./lib/types";

type ManualInterviewPhase = "question" | "answer";

const DEFAULT_RUNTIME_INFO: RuntimeInfo = {
  platform: "macos",
  mobile: false,
  audioCapture: "dual",
};

const QUESTION_TAIL_DELAY_MS = 450;
const QUESTION_APPEND_ROLLBACK_MS = 3_500;

const DEFAULT_PROVIDER: ProviderConfig = {
  name: "DeepSeek",
  baseUrl: "https://api.deepseek.com",
  apiKey: "",
  model: "deepseek-v4-flash",
  disableThinking: true,
};

const DEFAULT_HOTWORDS = [
  "Golang=高粱|高亮|勾烂|狗浪|国王|够浪|GO",
  "GMP=鸡一|鸡安屁|鸡艾姆屁|积木P|节目P|G M P",
  "goroutine=固若听|狗肉听",
  "MySQL",
  "Redis",
  "Kafka",
  "Docker",
  "Kubernetes",
].join("\n");

const DEFAULT_STT_CONFIG: SttConfig = {
  mode: "localAccurate",
  hotwords: DEFAULT_HOTWORDS,
  tencent: {
    appId: "",
    secretId: "",
    secretKey: "",
  },
};

const DEFAULT_TRANSLATION_CONFIG: TranslationConfig = {
  mode: "off",
  ollamaBaseUrl: "http://127.0.0.1:11434",
  ollamaModel: "qwen2.5:3b",
};

const DEFAULT_COMPANION_INFO: CompanionServerInfo = {
  running: false,
  url: "",
  port: 0,
  localOnly: false,
};

interface SecretStatus {
  deepseekApiKey: boolean;
  tencentSecretKey: boolean;
}

type SecretKind = "deepseekApiKey" | "tencentSecretKey";

const DEFAULT_SECRET_STATUS: SecretStatus = {
  deepseekApiKey: false,
  tencentSecretKey: false,
};

const APP_PREFERENCES_KEY = "interview-copilot-public-preferences";

interface PublicPreferences {
  provider?: Partial<Omit<ProviderConfig, "apiKey">>;
  stt?: {
    mode?: SttConfig["mode"];
    hotwords?: string;
    tencent?: Partial<Omit<SttConfig["tencent"], "secretKey">>;
  };
  translation?: Partial<TranslationConfig>;
}

function loadPublicPreferences(): PublicPreferences {
  try {
    return JSON.parse(localStorage.getItem(APP_PREFERENCES_KEY) ?? "{}") as PublicPreferences;
  } catch {
    return {};
  }
}

function loadProviderPreferences(): ProviderConfig {
  const saved = loadPublicPreferences().provider;
  return {
    ...DEFAULT_PROVIDER,
    baseUrl: typeof saved?.baseUrl === "string" ? saved.baseUrl : DEFAULT_PROVIDER.baseUrl,
    model: typeof saved?.model === "string" ? saved.model : DEFAULT_PROVIDER.model,
    disableThinking:
      typeof saved?.disableThinking === "boolean"
        ? saved.disableThinking
        : DEFAULT_PROVIDER.disableThinking,
  };
}

function loadSttPreferences(): SttConfig {
  const saved = loadPublicPreferences().stt;
  const validModes: SttConfig["mode"][] = ["localFast", "localAccurate", "tencentCloud"];
  return {
    ...DEFAULT_STT_CONFIG,
    mode:
      saved?.mode && validModes.includes(saved.mode) ? saved.mode : DEFAULT_STT_CONFIG.mode,
    hotwords: typeof saved?.hotwords === "string" ? saved.hotwords : DEFAULT_STT_CONFIG.hotwords,
    tencent: {
      ...DEFAULT_STT_CONFIG.tencent,
      appId: typeof saved?.tencent?.appId === "string" ? saved.tencent.appId : "",
      secretId: typeof saved?.tencent?.secretId === "string" ? saved.tencent.secretId : "",
    },
  };
}

function loadTranslationPreferences(): TranslationConfig {
  const saved = loadPublicPreferences().translation;
  const validModes: TranslationConfig["mode"][] = ["off", "deepseek", "ollama"];
  return {
    mode:
      saved?.mode && validModes.includes(saved.mode)
        ? saved.mode
        : DEFAULT_TRANSLATION_CONFIG.mode,
    ollamaBaseUrl:
      typeof saved?.ollamaBaseUrl === "string"
        ? saved.ollamaBaseUrl
        : DEFAULT_TRANSLATION_CONFIG.ollamaBaseUrl,
    ollamaModel:
      typeof saved?.ollamaModel === "string"
        ? saved.ollamaModel
        : DEFAULT_TRANSLATION_CONFIG.ollamaModel,
  };
}

function savePublicPreferences(
  provider: ProviderConfig,
  sttConfig: SttConfig,
  translation: TranslationConfig,
) {
  const preferences: PublicPreferences = {
    provider: {
      name: provider.name,
      baseUrl: provider.baseUrl,
      model: provider.model,
      disableThinking: provider.disableThinking,
    },
    stt: {
      mode: sttConfig.mode,
      hotwords: sttConfig.hotwords,
      tencent: {
        appId: sttConfig.tencent.appId,
        secretId: sttConfig.tencent.secretId,
      },
    },
    translation,
  };
  localStorage.setItem(APP_PREFERENCES_KEY, JSON.stringify(preferences));
}

interface WindowPosition {
  x: number;
  y: number;
}

interface WindowPreferences {
  alwaysOnTop: boolean;
  locked: boolean;
  position?: WindowPosition;
}

const WINDOW_PREFERENCES_KEY = "interview-copilot-window-preferences";
const DEFAULT_WINDOW_PREFERENCES: WindowPreferences = {
  alwaysOnTop: false,
  locked: false,
};

function loadWindowPreferences(): WindowPreferences {
  try {
    const value = localStorage.getItem(WINDOW_PREFERENCES_KEY);
    if (!value) return DEFAULT_WINDOW_PREFERENCES;
    const parsed = JSON.parse(value) as Partial<WindowPreferences>;
    const position = parsed.position;
    return {
      alwaysOnTop: parsed.alwaysOnTop === true,
      locked: parsed.locked === true,
      position:
        position && Number.isFinite(position.x) && Number.isFinite(position.y)
          ? { x: position.x, y: position.y }
          : undefined,
    };
  } catch {
    return DEFAULT_WINDOW_PREFERENCES;
  }
}

function saveWindowPreferences(preferences: WindowPreferences) {
  localStorage.setItem(WINDOW_PREFERENCES_KEY, JSON.stringify(preferences));
}

function App() {
  const [runtimeInfo, setRuntimeInfo] = useState(DEFAULT_RUNTIME_INFO);
  const [runtimeReady, setRuntimeReady] = useState(false);
  const [listening, setListening] = useState(false);
  const [audioLevels, setAudioLevels] = useState<Record<AudioSourceSpeaker, AudioLevelEvent>>({
    interviewer: { rms: 0, peak: 0, sampleRate: 0, speaker: "interviewer" },
    candidate: { rms: 0, peak: 0, sampleRate: 0, speaker: "candidate" },
    ambient: { rms: 0, peak: 0, sampleRate: 0, speaker: "ambient" },
  });
  const [transcript, setTranscript] = useState<TranscriptEntry[]>([]);
  const [partialTranscripts, setPartialTranscripts] = useState<
    Partial<Record<TranscriptEntry["speaker"], TranscriptEntry>>
  >({});
  const deletedTranscriptIdsRef = useRef(new Set<string>());
  const transcriptStartTimesRef = useRef(new Map<string, number>());
  const translationRequestsRef = useRef(new Set<string>());
  const ambientTranscriptIdsRef = useRef(new Set<string>());
  const ambientSpeakerAssignmentsRef = useRef(
    new Map<string, TranscriptEntry["speaker"]>(),
  );
  const ambientSpeakerRef = useRef<TranscriptEntry["speaker"]>("interviewer");
  const transcriptRef = useRef<TranscriptEntry[]>([]);
  const transcriptListRef = useRef<HTMLDivElement>(null);
  const transcriptAutoFollowRef = useRef(true);
  const partialTranscriptsRef = useRef<
    Partial<Record<TranscriptEntry["speaker"], TranscriptEntry>>
  >({});
  const questionWindowStartRef = useRef(Date.now());
  const [draft, setDraft] = useState("");
  const [speaker, setSpeaker] = useState<"interviewer" | "candidate">("interviewer");
  const [provider, setProvider] = useState(loadProviderPreferences);
  const [secretStatus, setSecretStatus] = useState(DEFAULT_SECRET_STATUS);
  const [answer, setAnswer] = useState("");
  const [answerLanguage, setAnswerLanguage] = useState<AnswerLanguage | "">("");
  const [answerHints, setAnswerHints] = useState("");
  const [activeRequestId, setActiveRequestId] = useState<string | null>(null);
  const activeRequestIdRef = useRef<string | null>(null);
  const askAiRef = useRef<(questionOverride?: string) => Promise<void>>(async () => undefined);
  const [status, setStatus] = useState("准备就绪");
  const [profile, setProfile] = useState("");
  const [jobDescription, setJobDescription] = useState("");
  const [sttConfig, setSttConfig] = useState(loadSttPreferences);
  const [translationConfig, setTranslationConfig] = useState(loadTranslationPreferences);
  const [showSettings, setShowSettings] = useState(false);
  const [companionInfo, setCompanionInfo] = useState(DEFAULT_COMPANION_INFO);
  const [companionBusy, setCompanionBusy] = useState(false);
  const [windowPreferences, setWindowPreferences] = useState(loadWindowPreferences);
  const [manualPhase, setManualPhase] = useState<ManualInterviewPhase>("question");
  const [manualAppending, setManualAppending] = useState(false);
  const [manualActionBusy, setManualActionBusy] = useState(false);
  const [currentQuestion, setCurrentQuestion] = useState("");

  const isMobile = runtimeInfo.mobile;

  const latestInterviewer = useMemo(
    () => [...transcript].reverse().find((entry) => entry.speaker === "interviewer"),
    [transcript],
  );
  const partialEntries = useMemo(
    () =>
      Object.values(partialTranscripts).sort(
        (left, right) => left.timestamp - right.timestamp,
      ),
    [partialTranscripts],
  );
  const interviewerPartial = partialTranscripts.interviewer;
  const detection = useMemo(
    () =>
      detectQuestion(
        interviewerPartial?.text ??
          latestInterviewer?.text ??
          (speaker === "interviewer" ? draft : ""),
      ),
    [draft, interviewerPartial, latestInterviewer, speaker],
  );

  useEffect(() => {
    transcriptRef.current = transcript;
  }, [transcript]);

  useEffect(() => {
    const transcriptList = transcriptListRef.current;
    if (!transcriptList || !transcriptAutoFollowRef.current) return;

    const frame = requestAnimationFrame(() => {
      transcriptList.scrollTop = transcriptList.scrollHeight;
    });
    return () => cancelAnimationFrame(frame);
  }, [partialEntries, transcript]);

  useEffect(() => {
    partialTranscriptsRef.current = partialTranscripts;
  }, [partialTranscripts]);

  useEffect(() => {
    const unlistenLevel = listen<AudioLevelEvent>("audio-level", (event) => {
      const payload = event.payload;
      setAudioLevels((current) => ({ ...current, [payload.speaker]: payload }));
    });
    const unlistenLlm = listen<LlmStreamEvent>("llm-stream", (event) => {
      const payload = event.payload;
      if (payload.requestId !== activeRequestIdRef.current) return;
      if (payload.error) {
        setStatus(`调用失败：${payload.error}`);
        activeRequestIdRef.current = null;
        setActiveRequestId(null);
        return;
      }
      if (payload.delta) setAnswer((current) => current + payload.delta);
      if (payload.done) {
        setStatus("回答已生成");
        activeRequestIdRef.current = null;
        setActiveRequestId(null);
      }
    });
    const unlistenAudioError = listen<string>("audio-error", (event) => {
      setListening(false);
      setStatus(`音频错误：${event.payload}`);
      void invoke("stop_audio_capture").catch(() => undefined);
    });
    const unlistenPartial = listen<TranscriptPartialEvent>("transcript-partial", (event) => {
      const payload = event.payload;
      if (deletedTranscriptIdsRef.current.has(payload.id)) return;
      const resolvedSpeaker =
        payload.speaker === "ambient"
          ? (ambientSpeakerAssignmentsRef.current.get(payload.id) ?? ambientSpeakerRef.current)
          : payload.speaker;
      if (payload.speaker === "ambient") {
        ambientTranscriptIdsRef.current.add(payload.id);
        ambientSpeakerAssignmentsRef.current.set(payload.id, resolvedSpeaker);
      }
      const result = detectQuestion(payload.text);
      const timestamp = transcriptStartTimesRef.current.get(payload.id) ?? Date.now();
      transcriptStartTimesRef.current.set(payload.id, timestamp);
      setPartialTranscripts((current) => ({
        ...current,
        [resolvedSpeaker]: {
          id: payload.id,
          speaker: resolvedSpeaker,
          text: payload.text,
          language: result.language,
          timestamp,
          isFinal: false,
        },
      }));
      setStatus(
        resolvedSpeaker === "interviewer"
          ? "正在识别面试官…"
          : "正在记录我的回答…",
      );
    });
    const unlistenTranscript = listen<TranscriptFinalEvent>("transcript-final", (event) => {
      const payload = event.payload;
      if (deletedTranscriptIdsRef.current.has(payload.id)) return;
      const resolvedSpeaker =
        payload.speaker === "ambient"
          ? (ambientSpeakerAssignmentsRef.current.get(payload.id) ?? ambientSpeakerRef.current)
          : payload.speaker;
      if (payload.speaker === "ambient") {
        ambientTranscriptIdsRef.current.add(payload.id);
        ambientSpeakerAssignmentsRef.current.set(payload.id, resolvedSpeaker);
      }
      const result = detectQuestion(payload.text);
      const entry: TranscriptEntry = {
        id: payload.id,
        speaker: resolvedSpeaker,
        text: payload.text,
        language: result.language,
        timestamp: transcriptStartTimesRef.current.get(payload.id) ?? Date.now(),
        isFinal: payload.isFinal,
      };
      transcriptStartTimesRef.current.delete(payload.id);
      setPartialTranscripts((current) => {
        if (current[resolvedSpeaker]?.id !== payload.id) return current;
        const next = { ...current };
        delete next[resolvedSpeaker];
        return next;
      });
      setTranscript((current) => {
        const existingIndex = current.findIndex((item) => item.id === payload.id);
        const next =
          existingIndex < 0
            ? [...current, entry]
            : current.map((item, index) => (index === existingIndex ? entry : item));
        return next.sort((left, right) => left.timestamp - right.timestamp);
      });
      setStatus(
        resolvedSpeaker === "candidate"
          ? "已记录我的回答"
          : result.isQuestion
            ? "已识别面试问题"
            : "已完成一段面试官语音识别",
      );
    });
    const unlistenSttStatus = listen<SttStatusEvent>("stt-status", (event) => {
      setStatus(event.payload.message);
    });
    const unlistenCompanionGenerate = listen("companion-generate-requested", () => {
      void askAiRef.current();
    });

    return () => {
      void unlistenLevel.then((fn) => fn());
      void unlistenLlm.then((fn) => fn());
      void unlistenAudioError.then((fn) => fn());
      void unlistenPartial.then((fn) => fn());
      void unlistenTranscript.then((fn) => fn());
      void unlistenSttStatus.then((fn) => fn());
      void unlistenCompanionGenerate.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const mobilePreview = new URLSearchParams(window.location.search).has("mobilePreview");
    if (mobilePreview) {
      setRuntimeInfo({ platform: "ios", mobile: true, audioCapture: "ambient" });
      setSttConfig((current) => ({ ...current, mode: "tencentCloud" }));
      setTranslationConfig((current) =>
        current.mode === "ollama" ? { ...current, mode: "deepseek" } : current,
      );
      setWindowPreferences(DEFAULT_WINDOW_PREFERENCES);
      setRuntimeReady(true);
      return;
    }
    void invoke<RuntimeInfo>("get_runtime_info")
      .then((info) => {
        setRuntimeInfo(info);
        if (info.mobile) {
          setSttConfig((current) => ({ ...current, mode: "tencentCloud" }));
          setTranslationConfig((current) =>
            current.mode === "ollama" ? { ...current, mode: "deepseek" } : current,
          );
          setWindowPreferences(DEFAULT_WINDOW_PREFERENCES);
        }
      })
      .catch((error) => setStatus(`读取运行环境失败：${String(error)}`))
      .finally(() => setRuntimeReady(true));
  }, []);

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    void invoke<SecretStatus>("get_secret_status")
      .then(setSecretStatus)
      .catch((error) => setStatus(`读取钥匙串状态失败：${String(error)}`));
  }, []);

  useEffect(() => {
    savePublicPreferences(provider, sttConfig, translationConfig);
  }, [
    provider.baseUrl,
    provider.disableThinking,
    provider.model,
    provider.name,
    sttConfig.hotwords,
    sttConfig.mode,
    sttConfig.tencent.appId,
    sttConfig.tencent.secretId,
    translationConfig.mode,
    translationConfig.ollamaBaseUrl,
    translationConfig.ollamaModel,
  ]);

  useEffect(() => {
    if (translationConfig.mode === "off") return;
    if (
      translationConfig.mode === "deepseek" &&
      !provider.apiKey.trim() &&
      !secretStatus.deepseekApiKey
    ) {
      return;
    }
    if (
      translationConfig.mode === "ollama" &&
      (!translationConfig.ollamaBaseUrl.trim() || !translationConfig.ollamaModel.trim())
    ) {
      return;
    }

    for (const entry of transcript) {
      if (
        entry.speaker !== "interviewer" ||
        !entry.isFinal ||
        !["en", "mixed"].includes(entry.language) ||
        !entry.text.trim()
      ) {
        continue;
      }
      const engine =
        translationConfig.mode === "deepseek"
          ? `${provider.baseUrl}|${provider.model}`
          : `${translationConfig.ollamaBaseUrl}|${translationConfig.ollamaModel}`;
      const translationKey = `${entry.id}|${translationConfig.mode}|${engine}|${entry.text}`;
      if (
        translationRequestsRef.current.has(translationKey) ||
        (entry.translationKey === translationKey && entry.translationStatus === "complete")
      ) {
        continue;
      }

      translationRequestsRef.current.add(translationKey);
      setTranscript((current) =>
        current.map((item) =>
          item.id === entry.id
            ? {
                ...item,
                translation: undefined,
                translationError: undefined,
                translationKey,
                translationStatus: "pending",
              }
            : item,
        ),
      );
      void translateToChinese(entry.text, translationConfig, provider)
        .then((translation) => {
          setTranscript((current) =>
            current.map((item) =>
              item.id === entry.id && item.translationKey === translationKey
                ? {
                    ...item,
                    translation,
                    translationError: undefined,
                    translationStatus: "complete",
                  }
                : item,
            ),
          );
        })
        .catch((error) => {
          setTranscript((current) =>
            current.map((item) =>
              item.id === entry.id && item.translationKey === translationKey
                ? {
                    ...item,
                    translationError: String(error),
                    translationStatus: "error",
                  }
                : item,
            ),
          );
        });
    }
  }, [
    provider.apiKey,
    provider.baseUrl,
    provider.disableThinking,
    provider.model,
    secretStatus.deepseekApiKey,
    transcript,
    translationConfig,
  ]);

  useEffect(() => {
    if (!runtimeReady || runtimeInfo.mobile) return;
    void invoke<WindowPosition>("restore_window_preferences", {
      preferences: windowPreferences,
    })
      .then((position) => {
        const restored = { ...windowPreferences, position };
        setWindowPreferences(restored);
        saveWindowPreferences(restored);
      })
      .catch((error) => setStatus(`窗口设置失败：${String(error)}`));
    // Window preferences are restored once. Later changes use set_window_presentation.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtimeInfo.mobile, runtimeReady]);

  useEffect(() => {
    if (!runtimeReady || runtimeInfo.mobile || !("__TAURI_INTERNALS__" in window)) return;
    void invoke<CompanionServerInfo>("get_companion_server_info")
      .then(setCompanionInfo)
      .catch(() => undefined);
  }, [runtimeInfo.mobile, runtimeReady]);

  useEffect(() => {
    if (runtimeInfo.mobile) return;
    document.documentElement.classList.toggle(
      "transparent-mode",
      windowPreferences.alwaysOnTop,
    );
    return () => document.documentElement.classList.remove("transparent-mode");
  }, [runtimeInfo.mobile, windowPreferences.alwaysOnTop]);

  async function toggleListening() {
    try {
      if (listening) {
        await invoke("stop_audio_capture");
        setListening(false);
        setStatus("监听已停止");
      } else {
        if (!answerLanguage) {
          setStatus("请先在顶部选择本场面试的回答语言");
          return;
        }
        const captureConfig = isMobile
          ? { ...sttConfig, mode: "tencentCloud" as const }
          : sttConfig;
        const started = await invoke<CaptureStarted>("start_audio_capture", {
          config: captureConfig,
        });
        if (started.audioCapture === "ambient") {
          ambientSpeakerRef.current = "interviewer";
          ambientSpeakerAssignmentsRef.current.clear();
          ambientTranscriptIdsRef.current.clear();
          questionWindowStartRef.current = Date.now();
          setManualPhase("question");
          setManualAppending(false);
          setCurrentQuestion("");
        }
        setListening(true);
        setStatus(
          started.audioCapture === "ambient"
            ? "正在通过 iPhone 麦克风监听面试环境"
            : "正在监听面试官和我的麦克风",
        );
      }
    } catch (error) {
      setStatus(`音频错误：${String(error)}`);
    }
  }

  function appendTranscript(event: FormEvent) {
    event.preventDefault();
    const text = draft.trim();
    if (!text) return;
    const result = detectQuestion(text);
    setTranscript((current) => [
      ...current,
      {
        id: crypto.randomUUID(),
        speaker,
        text,
        language: result.language,
        timestamp: Date.now(),
        isFinal: true,
      },
    ]);
    setDraft("");
  }

  function deleteTranscript(id: string) {
    deletedTranscriptIdsRef.current.add(id);
    transcriptStartTimesRef.current.delete(id);
    ambientTranscriptIdsRef.current.delete(id);
    ambientSpeakerAssignmentsRef.current.delete(id);
    setTranscript((current) => current.filter((entry) => entry.id !== id));
    setPartialTranscripts((current) => {
      const next = { ...current };
      for (const entry of Object.values(next)) {
        if (entry?.id === id) delete next[entry.speaker];
      }
      return next;
    });
    setStatus("已删除这条转写");
  }

  function retryTranslation(entry: TranscriptEntry) {
    if (entry.translationKey) {
      translationRequestsRef.current.delete(entry.translationKey);
    }
    setTranscript((current) =>
      current.map((item) =>
        item.id === entry.id
          ? {
              ...item,
              translationError: undefined,
              translationStatus: undefined,
            }
          : item,
      ),
    );
  }

  function clearTranscript() {
    transcriptAutoFollowRef.current = true;
    setTranscript((current) => {
      current.forEach((entry) => deletedTranscriptIdsRef.current.add(entry.id));
      return [];
    });
    setPartialTranscripts((current) => {
      Object.values(current).forEach((entry) => {
        if (entry) deletedTranscriptIdsRef.current.add(entry.id);
      });
      return {};
    });
    transcriptStartTimesRef.current.clear();
    translationRequestsRef.current.clear();
    ambientTranscriptIdsRef.current.clear();
    ambientSpeakerAssignmentsRef.current.clear();
    questionWindowStartRef.current = Date.now();
    setCurrentQuestion("");
    setManualAppending(false);
    setStatus("转写记录已清空");
  }

  async function updateWindowPresentation(next: WindowPreferences, message: string) {
    try {
      const position = await invoke<WindowPosition>("set_window_presentation", {
        alwaysOnTop: next.alwaysOnTop,
        locked: next.locked,
      });
      const saved = { ...next, position };
      setWindowPreferences(saved);
      saveWindowPreferences(saved);
      setStatus(message);
    } catch (error) {
      setStatus(`窗口设置失败：${String(error)}`);
    }
  }

  async function saveSensitiveConfiguration() {
    const deepseekKey = provider.apiKey.trim();
    const tencentKey = sttConfig.tencent.secretKey.trim();
    if (!deepseekKey && !tencentKey) {
      setStatus("请输入需要保存或替换的密钥");
      return;
    }

    try {
      const operations: Promise<unknown>[] = [];
      if (deepseekKey) {
        operations.push(invoke("save_secret", { kind: "deepseekApiKey", value: deepseekKey }));
      }
      if (tencentKey) {
        operations.push(invoke("save_secret", { kind: "tencentSecretKey", value: tencentKey }));
      }
      await Promise.all(operations);
      setSecretStatus((current) => ({
        deepseekApiKey: current.deepseekApiKey || Boolean(deepseekKey),
        tencentSecretKey: current.tencentSecretKey || Boolean(tencentKey),
      }));
      setProvider((current) => ({ ...current, apiKey: "" }));
      setSttConfig((current) => ({
        ...current,
        tencent: { ...current.tencent, secretKey: "" },
      }));
      setStatus("密钥已安全保存到系统钥匙串");
    } catch (error) {
      setStatus(`保存密钥失败：${String(error)}`);
    }
  }

  async function startCompanionSync() {
    setCompanionBusy(true);
    try {
      const token = crypto.randomUUID().replaceAll("-", "");
      const info = await invoke<CompanionServerInfo>("start_companion_server", { token });
      setCompanionInfo(info);
      setStatus(
        info.localOnly
          ? "同步服务已启动，但没有检测到局域网地址"
          : "手机同步已开启，请在同一 Wi-Fi 的设备上打开链接",
      );
    } catch (error) {
      setStatus(`启动手机同步失败：${String(error)}`);
    } finally {
      setCompanionBusy(false);
    }
  }

  async function stopCompanionSync() {
    setCompanionBusy(true);
    try {
      const info = await invoke<CompanionServerInfo>("stop_companion_server");
      setCompanionInfo(info);
      setStatus("手机同步已关闭");
    } catch (error) {
      setStatus(`关闭手机同步失败：${String(error)}`);
    } finally {
      setCompanionBusy(false);
    }
  }

  async function copyCompanionUrl() {
    try {
      await navigator.clipboard.writeText(companionInfo.url);
      setStatus("手机同步地址已复制");
    } catch (error) {
      setStatus(`复制地址失败：${String(error)}`);
    }
  }

  async function clearStoredSecret(kind: SecretKind) {
    const label = kind === "deepseekApiKey" ? "DeepSeek API Key" : "腾讯云 SecretKey";
    if (!window.confirm(`确定从系统钥匙串删除 ${label}？`)) return;
    try {
      await invoke("delete_secret", { kind });
      setSecretStatus((current) => ({ ...current, [kind]: false }));
      setStatus(`${label} 已从系统钥匙串删除`);
    } catch (error) {
      setStatus(`删除密钥失败：${String(error)}`);
    }
  }

  function collectQuestionTextSince(startedAt: number): string {
    const entriesById = new Map<string, TranscriptEntry>();
    for (const entry of [
      ...transcriptRef.current,
      ...Object.values(partialTranscriptsRef.current).filter(
        (entry): entry is TranscriptEntry => Boolean(entry),
      ),
    ]) {
      if (entry.speaker !== "interviewer" || entry.timestamp < startedAt) continue;
      const existing = entriesById.get(entry.id);
      if (!existing || entry.isFinal) entriesById.set(entry.id, entry);
    }
    return [...entriesById.values()]
      .sort((left, right) => left.timestamp - right.timestamp)
      .map((entry) => entry.text.trim())
      .filter(Boolean)
      .join(" ")
      .trim();
  }

  function startQuestionAppend() {
    if (!listening || !currentQuestion || manualActionBusy) return;
    const rollbackAt = Date.now() - QUESTION_APPEND_ROLLBACK_MS;
    ambientSpeakerRef.current = "interviewer";
    questionWindowStartRef.current = rollbackAt;

    for (const entry of [
      ...transcriptRef.current,
      ...Object.values(partialTranscriptsRef.current).filter(
        (item): item is TranscriptEntry => Boolean(item),
      ),
    ]) {
      if (entry.timestamp >= rollbackAt && ambientTranscriptIdsRef.current.has(entry.id)) {
        ambientSpeakerAssignmentsRef.current.set(entry.id, "interviewer");
      }
    }

    setTranscript((current) => {
      const next = current.map((entry) =>
        entry.timestamp >= rollbackAt && ambientTranscriptIdsRef.current.has(entry.id)
          ? { ...entry, speaker: "interviewer" as const }
          : entry,
      );
      transcriptRef.current = next;
      return next;
    });
    setPartialTranscripts((current) => {
      const next: Partial<Record<TranscriptEntry["speaker"], TranscriptEntry>> = {};
      for (const entry of Object.values(current)) {
        if (!entry) continue;
        const reassigned =
          entry.timestamp >= rollbackAt && ambientTranscriptIdsRef.current.has(entry.id)
            ? { ...entry, speaker: "interviewer" as const }
            : entry;
        next[reassigned.speaker] = reassigned;
      }
      partialTranscriptsRef.current = next;
      return next;
    });
    setManualPhase("question");
    setManualAppending(true);
    setStatus("正在追加面试官的问题；补充结束后点击“问题结束”");
  }

  function finishCandidateAnswer() {
    if (!listening || manualActionBusy) return;
    ambientSpeakerRef.current = "interviewer";
    questionWindowStartRef.current = Date.now();
    setManualPhase("question");
    setManualAppending(false);
    setCurrentQuestion("");
    setStatus("回答已结束，正在监听下一道问题");
  }

  async function finishQuestion() {
    if (!listening || manualActionBusy) return;
    setManualActionBusy(true);
    setStatus("正在收取问题尾音…");
    await new Promise<void>((resolve) => window.setTimeout(resolve, QUESTION_TAIL_DELAY_MS));

    const addedText = collectQuestionTextSince(questionWindowStartRef.current);
    const question = manualAppending
      ? [currentQuestion, addedText && `补充：${addedText}`].filter(Boolean).join("\n")
      : addedText;
    if (!question.trim()) {
      setManualActionBusy(false);
      setStatus("还没有识别到本轮面试官问题，请继续监听后再点击");
      return;
    }

    setCurrentQuestion(question);
    setManualAppending(false);
    setManualPhase("answer");
    ambientSpeakerRef.current = "candidate";
    setManualActionBusy(false);
    await askAi(question);
  }

  async function askAi(questionOverride?: string) {
    if (activeRequestIdRef.current) {
      setStatus("回答正在生成，请稍候");
      return;
    }
    const question =
      questionOverride?.trim() ||
      currentQuestion.trim() ||
      interviewerPartial?.text.trim() ||
      latestInterviewer?.text.trim() ||
      (speaker === "interviewer" ? draft.trim() : "");
    if (!question) {
      setStatus("请先加入一条面试官转写");
      return;
    }
    if (!answerLanguage) {
      setStatus("请先在顶部选择中文回答或 English answer");
      return;
    }
    if (!provider.apiKey.trim() && !secretStatus.deepseekApiKey) {
      setShowSettings(true);
      setStatus("请先填写 DeepSeek API Key");
      return;
    }

    const requestId = crypto.randomUUID();
    setAnswer("");
    activeRequestIdRef.current = requestId;
    setActiveRequestId(requestId);
    setStatus("正在生成回答…");

    try {
      await startChatStream({
        requestId,
        provider,
        question,
        answerLanguage,
        messages: buildInterviewMessages(
          question,
          answerLanguage,
          transcriptRef.current,
          profile,
          jobDescription,
          answerHints.trim(),
        ),
        temperature: 0.3,
        maxTokens: 800,
      });
    } catch (error) {
      activeRequestIdRef.current = null;
      setActiveRequestId(null);
      setStatus(`调用失败：${String(error)}`);
    }
  }

  askAiRef.current = askAi;

  const meterWidths = {
    interviewer: `${Math.min(100, Math.max(2, audioLevels.interviewer.rms * 260))}%`,
    candidate: `${Math.min(100, Math.max(2, audioLevels.candidate.rms * 260))}%`,
    ambient: `${Math.min(100, Math.max(2, audioLevels.ambient.rms * 260))}%`,
  };

  return (
    <main
      className={`app-shell ${windowPreferences.alwaysOnTop ? "focus-mode" : ""} ${
        isMobile ? "mobile-runtime" : ""
      }`}
    >
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark">IC</span>
          <div>
            <h1>Interview Copilot</h1>
            <p>
              中英实时面试教练 · {isMobile ? "iPhone 环境监听" : "macOS 双通道"}
            </p>
          </div>
        </div>
        <div className="topbar-actions">
          <span className={`status-dot ${listening ? "live" : ""}`} />
          <span className="status-text">{status}</span>
          <label className="quick-language">
            <span>回答语言</span>
            <select
              value={answerLanguage}
              disabled={listening}
              aria-label="选择本场面试的回答语言"
              onChange={(event) => {
                const value = event.target.value as AnswerLanguage | "";
                setAnswerLanguage(value);
                setStatus(
                  value
                    ? `回答语言已确认：${value === "zh" ? "中文" : "English"}`
                    : "请选择回答语言",
                );
              }}
            >
              <option value="">请先选择</option>
              <option value="zh">中文回答</option>
              <option value="en">English answer</option>
            </select>
          </label>
          {!isMobile && (
            <>
              <button
                className={`ghost-button ${windowPreferences.alwaysOnTop ? "selected" : ""}`}
                title="保持在其他窗口上方，并进入仅保留文字的透视模式"
                onClick={() =>
                  updateWindowPresentation(
                    { ...windowPreferences, alwaysOnTop: !windowPreferences.alwaysOnTop },
                    windowPreferences.alwaysOnTop
                      ? "已取消置顶并恢复界面背景"
                      : "窗口已置顶并进入透视模式",
                  )
                }
              >
                {windowPreferences.alwaysOnTop ? "已置顶 · 透视" : "置顶透视"}
              </button>
              <button
                className={`ghost-button ${windowPreferences.locked ? "selected" : ""}`}
                title="记住当前位置并阻止窗口被意外拖动"
                onClick={() =>
                  updateWindowPresentation(
                    { ...windowPreferences, locked: !windowPreferences.locked },
                    windowPreferences.locked ? "窗口位置已解锁" : "窗口位置已锁定并保存",
                  )
                }
              >
                {windowPreferences.locked ? "解除锁定" : "锁定位置"}
              </button>
            </>
          )}
          <button className="ghost-button" onClick={() => setShowSettings((value) => !value)}>
            设置
          </button>
        </div>
      </header>

      {showSettings && (
        <section className="settings-panel">
          <div className="section-heading">
            <div>
              <span className="eyebrow">SETTINGS</span>
              <h2>语音识别与回答 Provider</h2>
            </div>
            <span className="privacy-note">敏感密钥保存在系统钥匙串，不提供明文查看</span>
            {isMobile && (
              <button className="ghost-button" onClick={() => setShowSettings(false)}>
                完成
              </button>
            )}
          </div>
          {!isMobile && (
            <div className={`companion-settings ${companionInfo.running ? "active" : ""}`}>
              <div className="companion-heading">
                <div>
                  <span className="eyebrow">COMPANION OUTPUT</span>
                  <h3>同步回答到手机</h3>
                </div>
                <button
                  className={companionInfo.running ? "listen-button active" : "ask-button"}
                  disabled={companionBusy}
                  onClick={() =>
                    companionInfo.running
                      ? void stopCompanionSync()
                      : void startCompanionSync()
                  }
                >
                  {companionBusy
                    ? "处理中…"
                    : companionInfo.running
                      ? "关闭同步"
                      : "开启同步"}
                </button>
              </div>
              {companionInfo.running ? (
                <>
                  <p className="companion-note">
                    让 iPhone 与 Mac 连接同一个 Wi-Fi，再用 Safari 打开下面的临时地址。AI
                    回答会逐步同步，不经过第三方转发服务。
                  </p>
                  <div className="companion-url-row">
                    <code>{companionInfo.url}</code>
                    <button className="ghost-button" onClick={() => void copyCompanionUrl()}>
                      复制地址
                    </button>
                  </div>
                  {companionInfo.localOnly && (
                    <small className="companion-warning">
                      当前只检测到本机地址。请确认 Mac 已连接 Wi-Fi 后重新开启同步。
                    </small>
                  )}
                </>
              ) : (
                <p className="companion-note">
                  默认关闭。开启后只发送当前面试问题和 AI 回答文本，不发送麦克风音频、API
                  Key 或完整简历。
                </p>
              )}
            </div>
          )}
          <div className="settings-grid">
            <label className="wide-field">
              语音识别模式
              <select
                value={sttConfig.mode}
                disabled={listening || isMobile}
                onChange={(event) =>
                  setSttConfig({ ...sttConfig, mode: event.target.value as SttConfig["mode"] })
                }
              >
                {!isMobile && (
                  <>
                    <option value="localFast">本地快速 · Zipformer + SenseVoice</option>
                    <option value="localAccurate">本地高精度 · Zipformer + Qwen3-ASR 0.6B</option>
                  </>
                )}
                <option value="tencentCloud">腾讯云高精度 · ASR 大模型 2.0</option>
              </select>
              <small className="field-note">
                {sttConfig.mode === "localFast" && "完全离线，资源占用最低。"}
                {sttConfig.mode === "localAccurate" && "完全离线，准确率优先，会占用更多内存。"}
                {sttConfig.mode === "tencentCloud" && "实时上传音频到腾讯云，依赖网络并可能产生费用。"}
                {isMobile && " iPhone 首版固定使用腾讯云识别环境声音。"}
              </small>
            </label>
            <label>
              Base URL
              <input
                value={provider.baseUrl}
                onChange={(event) => setProvider({ ...provider, baseUrl: event.target.value })}
              />
            </label>
            <label>
              Model
              <input
                value={provider.model}
                onChange={(event) => setProvider({ ...provider, model: event.target.value })}
              />
            </label>
            <label className="wide-field">
              API Key
              <input
                type="password"
                value={provider.apiKey}
                placeholder={
                  secretStatus.deepseekApiKey
                    ? "已保存；输入新值可替换"
                    : "sk-..."
                }
                autoComplete="off"
                onChange={(event) => setProvider({ ...provider, apiKey: event.target.value })}
              />
              <small className={`key-status ${secretStatus.deepseekApiKey ? "saved" : ""}`}>
                {secretStatus.deepseekApiKey ? "已安全保存" : "尚未保存"}
              </small>
            </label>
            <label className="check-field">
              <input
                type="checkbox"
                checked={provider.disableThinking}
                onChange={(event) =>
                  setProvider({ ...provider, disableThinking: event.target.checked })
                }
              />
              关闭思考模式以降低延迟
            </label>
          </div>
          <div className="translation-settings">
            <label className="check-field translation-toggle">
              <input
                type="checkbox"
                checked={translationConfig.mode !== "off"}
                onChange={(event) =>
                  setTranslationConfig({
                    ...translationConfig,
                    mode: event.target.checked ? "deepseek" : "off",
                  })
                }
              />
              开启英语面试实时中文翻译
            </label>
            {translationConfig.mode !== "off" && (
              <>
                <label>
                  翻译方式
                  <select
                    value={translationConfig.mode}
                    onChange={(event) =>
                      setTranslationConfig({
                        ...translationConfig,
                        mode: event.target.value as Exclude<
                          TranslationConfig["mode"],
                          "off"
                        >,
                      })
                    }
                  >
                    <option value="deepseek">DeepSeek 云端翻译</option>
                    {!isMobile && <option value="ollama">Ollama 本地翻译</option>}
                  </select>
                </label>
                {translationConfig.mode === "deepseek" ? (
                  <p className="translation-note">
                    使用上方同一个 DeepSeek 模型和 API Key，仅发送面试官的英文转写文本，不发送音频。
                  </p>
                ) : (
                  <>
                    <label>
                      Ollama 地址
                      <input
                        value={translationConfig.ollamaBaseUrl}
                        placeholder="http://127.0.0.1:11434"
                        onChange={(event) =>
                          setTranslationConfig({
                            ...translationConfig,
                            ollamaBaseUrl: event.target.value,
                          })
                        }
                      />
                    </label>
                    <label>
                      Ollama 模型
                      <input
                        value={translationConfig.ollamaModel}
                        placeholder="qwen2.5:3b"
                        onChange={(event) =>
                          setTranslationConfig({
                            ...translationConfig,
                            ollamaModel: event.target.value,
                          })
                        }
                      />
                    </label>
                    <p className="translation-note">
                      完全在本机翻译。首次使用前运行：ollama pull {translationConfig.ollamaModel || "qwen2.5:3b"}
                    </p>
                  </>
                )}
              </>
            )}
          </div>
          {sttConfig.mode === "tencentCloud" && (
            <div className="settings-grid cloud-settings">
              <label>
                腾讯云 AppID
                <input
                  value={sttConfig.tencent.appId}
                  autoComplete="off"
                  onChange={(event) =>
                    setSttConfig({
                      ...sttConfig,
                      tencent: { ...sttConfig.tencent, appId: event.target.value },
                    })
                  }
                />
              </label>
              <label>
                SecretId
                <input
                  value={sttConfig.tencent.secretId}
                  autoComplete="off"
                  onChange={(event) =>
                    setSttConfig({
                      ...sttConfig,
                      tencent: { ...sttConfig.tencent, secretId: event.target.value },
                    })
                  }
                />
              </label>
              <label className="wide-field">
                SecretKey
                <input
                  type="password"
                  value={sttConfig.tencent.secretKey}
                  placeholder={
                    secretStatus.tencentSecretKey
                      ? "已保存；输入新值可替换"
                      : "请输入 SecretKey"
                  }
                  autoComplete="off"
                  onChange={(event) =>
                    setSttConfig({
                      ...sttConfig,
                      tencent: { ...sttConfig.tencent, secretKey: event.target.value },
                    })
                  }
                />
                <small className={`key-status ${secretStatus.tencentSecretKey ? "saved" : ""}`}>
                  {secretStatus.tencentSecretKey ? "已安全保存" : "尚未保存"}
                </small>
              </label>
            </div>
          )}
          <div className="secret-actions">
            <button
              className="ask-button"
              disabled={!provider.apiKey.trim() && !sttConfig.tencent.secretKey.trim()}
              onClick={saveSensitiveConfiguration}
            >
              安全保存新密钥
            </button>
            {secretStatus.deepseekApiKey && (
              <button className="clear-button" onClick={() => clearStoredSecret("deepseekApiKey")}>
                删除 DeepSeek 密钥
              </button>
            )}
            {secretStatus.tencentSecretKey && (
              <button className="clear-button" onClick={() => clearStoredSecret("tencentSecretKey")}>
                删除腾讯密钥
              </button>
            )}
          </div>
          <div className="context-grid">
            <label>
              个人经历 / 简历（可选）
              <textarea
                value={profile}
                onChange={(event) => setProfile(event.target.value)}
                placeholder="只填写真实经历，避免模型虚构"
              />
            </label>
            <label>
              岗位描述（可选）
              <textarea
                value={jobDescription}
                onChange={(event) => setJobDescription(event.target.value)}
                placeholder="粘贴 JD 以提高回答相关性"
              />
            </label>
            <label>
              技术热词 / 常见误识别（每行一个）
              <textarea
                value={sttConfig.hotwords}
                onChange={(event) => setSttConfig({ ...sttConfig, hotwords: event.target.value })}
                placeholder={"Golang=高粱|高亮|勾烂\nGMP=鸡一|节目P\ngoroutine"}
              />
            </label>
          </div>
        </section>
      )}

      <section className="workspace-grid">
        <article className="panel transcript-panel">
          <div className="panel-header">
            <div>
              <span className="eyebrow">LIVE TRANSCRIPT</span>
              <h2>实时转写</h2>
            </div>
            <div className="panel-actions">
              <button
                className="clear-button"
                disabled={transcript.length === 0 && partialEntries.length === 0}
                onClick={clearTranscript}
              >
                清空
              </button>
              <button
                className={`listen-button ${listening ? "active" : ""}`}
                onClick={toggleListening}
              >
                {listening ? "停止监听" : "开始监听"}
              </button>
            </div>
          </div>

          <div className="audio-sources">
            {isMobile ? (
              <div className="audio-source ambient-source">
                <div>
                  <strong>
                    环境麦克风 · {manualPhase === "question" ? "面试官" : "我的回答"}
                  </strong>
                  <small>
                    {audioLevels.ambient.sampleRate
                      ? `${audioLevels.ambient.sampleRate} Hz · peak ${audioLevels.ambient.peak.toFixed(2)}`
                      : "等待 iPhone 麦克风权限"}
                  </small>
                </div>
                <div className="audio-meter" aria-label="环境音频电平">
                  <span style={{ width: meterWidths.ambient }} />
                </div>
              </div>
            ) : (
              <>
                <div className="audio-source">
                  <div>
                    <strong>面试官 · 系统声音</strong>
                    <small>
                      {audioLevels.interviewer.sampleRate
                        ? `${audioLevels.interviewer.sampleRate} Hz · peak ${audioLevels.interviewer.peak.toFixed(2)}`
                        : "等待 Core Audio Tap"}
                    </small>
                  </div>
                  <div className="audio-meter" aria-label="系统音频电平">
                    <span style={{ width: meterWidths.interviewer }} />
                  </div>
                </div>
                <div className="audio-source candidate-source">
                  <div>
                    <strong>我 · 麦克风</strong>
                    <small>
                      {audioLevels.candidate.sampleRate
                        ? `${audioLevels.candidate.sampleRate} Hz · peak ${audioLevels.candidate.peak.toFixed(2)}`
                        : "等待麦克风权限"}
                    </small>
                  </div>
                  <div className="audio-meter" aria-label="麦克风输入电平">
                    <span style={{ width: meterWidths.candidate }} />
                  </div>
                </div>
              </>
            )}
          </div>

          <div
            className="transcript-list"
            ref={transcriptListRef}
            onScroll={(event) => {
              const element = event.currentTarget;
              const distanceFromBottom =
                element.scrollHeight - element.scrollTop - element.clientHeight;
              transcriptAutoFollowRef.current = distanceFromBottom < 72;
            }}
          >
            {transcript.length === 0 && partialEntries.length === 0 ? (
              <div className="empty-state">
                <p>
                  {isMobile
                    ? "请将 iPhone 横放在电脑扬声器附近。程序持续监听环境声，由底部按钮标记面试官问题与我的回答。"
                    : "系统声音会标记为“面试官”，麦克风会标记为“我”。建议佩戴耳机，避免扬声器声音再次进入麦克风。"}
                </p>
              </div>
            ) : (
              <>
                {transcript.map((entry) => (
                  <div className={`transcript-row ${entry.speaker}`} key={entry.id}>
                    <span>{entry.speaker === "interviewer" ? "面试官" : "我"}</span>
                    <div className="transcript-copy">
                      <p>{entry.text}</p>
                      {translationConfig.mode !== "off" &&
                        entry.speaker === "interviewer" &&
                        ["en", "mixed"].includes(entry.language) && (
                          <div className={`translation ${entry.translationStatus ?? ""}`}>
                            {entry.translationStatus === "pending" && <small>正在翻译为中文…</small>}
                            {entry.translationStatus === "complete" && entry.translation && (
                              <p>{entry.translation}</p>
                            )}
                            {entry.translationStatus === "error" && (
                              <div className="translation-error">
                                <small>{entry.translationError}</small>
                                <button onClick={() => retryTranslation(entry)}>重试</button>
                              </div>
                            )}
                            {!entry.translationStatus &&
                              translationConfig.mode === "deepseek" &&
                              !provider.apiKey.trim() &&
                              !secretStatus.deepseekApiKey && (
                                <small>保存 DeepSeek API Key 后自动翻译</small>
                              )}
                          </div>
                        )}
                    </div>
                    <small>{entry.language}</small>
                    <button
                      className="row-delete-button"
                      aria-label={`删除转写：${entry.text}`}
                      title="删除这条转写"
                      onClick={() => deleteTranscript(entry.id)}
                    >
                      删除
                    </button>
                  </div>
                ))}
                {partialEntries.map((partialTranscript) => (
                  <div
                    className={`transcript-row ${partialTranscript.speaker} partial`}
                    key={partialTranscript.id}
                  >
                    <span>{partialTranscript.speaker === "interviewer" ? "面试官" : "我"}</span>
                    <p>{partialTranscript.text}</p>
                    <small>识别中</small>
                    <button
                      className="row-delete-button"
                      aria-label={`删除正在识别的转写：${partialTranscript.text}`}
                      title="删除这段识别，并忽略它后续到达的结果"
                      onClick={() => deleteTranscript(partialTranscript.id)}
                    >
                      删除
                    </button>
                  </div>
                ))}
              </>
            )}
          </div>

          <form className="transcript-composer" onSubmit={appendTranscript}>
            <select
              value={speaker}
              onChange={(event) => setSpeaker(event.target.value as typeof speaker)}
            >
              <option value="interviewer">面试官</option>
              <option value="candidate">我</option>
            </select>
            <input
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              placeholder="输入中文、English 或中英混合转写…"
            />
            <button type="submit">加入</button>
          </form>
        </article>

        <article className="panel answer-panel">
          <div className="panel-header">
            <div>
              <span className="eyebrow">ANSWER COACH</span>
              <h2>回答建议</h2>
            </div>
            <button
              className="ask-button"
              disabled={Boolean(activeRequestId)}
              onClick={() => void askAi()}
            >
              {activeRequestId ? "生成中…" : "生成建议"}
            </button>
          </div>

          <div className="detection-card">
            <div>
              <span>问题检测</span>
              <strong>{detection.isQuestion ? "检测到问题" : "等待完整问题"}</strong>
            </div>
            <div>
              <span>语言</span>
              <strong>{detection.language}</strong>
            </div>
            <div>
              <span>置信度</span>
              <strong>{Math.round(detection.confidence * 100)}%</strong>
            </div>
            <div>
              <span>回答语言</span>
              <strong className={answerLanguage ? "" : "language-warning"}>
                {answerLanguage === "zh"
                  ? "中文"
                  : answerLanguage === "en"
                    ? "English"
                    : "未确认"}
              </strong>
            </div>
          </div>

          <div className="answer-hints">
            <div className="answer-hints-heading">
              <label htmlFor="answer-hints">本题即时提示</label>
              <button
                className="clear-button"
                disabled={!answerHints}
                onClick={() => setAnswerHints("")}
              >
                清空
              </button>
            </div>
            <textarea
              id="answer-hints"
              value={answerHints}
              rows={2}
              onChange={(event) => setAnswerHints(event.target.value)}
              placeholder="碎片词即可，例如：强调 GMP；结合支付项目；先讲结论再说取舍"
            />
            <small>只影响本题回答方向，不会作为面试官转写，也不会被当作真实经历。</small>
          </div>

          <div className="answer-content">
            {answer ? (
              <p>{answer}</p>
            ) : (
              <div className="answer-placeholder">
                <span>AI</span>
                <h3>短、准、可开口表达</h3>
                <p>回答会严格使用你在面试前选择的语言，并结合本题即时提示。</p>
              </div>
            )}
          </div>
        </article>
      </section>

      {isMobile && (
        <section className={`manual-controls ${manualPhase} ${manualAppending ? "appending" : ""}`}>
          <div className="manual-state">
            <span>{manualPhase === "question" ? "正在听题" : "正在记录我的回答"}</span>
            <strong>
              {manualAppending
                ? "面试官正在补充问题"
                : manualPhase === "question"
                  ? "说完后点击“问题结束”"
                  : "若面试官补充请点左侧；答完请点右侧"}
            </strong>
          </div>
          <button
            className="append-question-button"
            disabled={!listening || !currentQuestion || manualActionBusy || manualAppending}
            onClick={startQuestionAppend}
          >
            <span>问题追加</span>
            <small>回溯最近 3.5 秒</small>
          </button>
          <button
            className="phase-action-button"
            disabled={!listening || manualActionBusy}
            onClick={() => {
              if (manualPhase === "question") void finishQuestion();
              else finishCandidateAnswer();
            }}
          >
            <span>
              {manualActionBusy
                ? "正在收尾…"
                : manualPhase === "question"
                  ? "问题结束"
                  : "回答结束"}
            </span>
            <small>{manualPhase === "question" ? "生成回答建议" : "开始听下一题"}</small>
          </button>
        </section>
      )}
    </main>
  );
}

export default App;
