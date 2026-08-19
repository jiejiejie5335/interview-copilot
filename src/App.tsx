import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import { detectQuestion } from "./lib/questionDetector";
import { startChatStream } from "./lib/provider";
import { buildInterviewMessages } from "./lib/prompts";
import type {
  AnswerLanguage,
  AudioLevelEvent,
  LlmStreamEvent,
  ProviderConfig,
  SttStatusEvent,
  SttConfig,
  TranscriptEntry,
  TranscriptFinalEvent,
  TranscriptPartialEvent,
} from "./lib/types";

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

function savePublicPreferences(provider: ProviderConfig, sttConfig: SttConfig) {
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
  const [listening, setListening] = useState(false);
  const [audioLevel, setAudioLevel] = useState<AudioLevelEvent>({
    rms: 0,
    peak: 0,
    sampleRate: 0,
  });
  const [transcript, setTranscript] = useState<TranscriptEntry[]>([]);
  const [partialTranscript, setPartialTranscript] = useState<TranscriptEntry | null>(null);
  const deletedTranscriptIdsRef = useRef(new Set<string>());
  const [draft, setDraft] = useState("");
  const [speaker, setSpeaker] = useState<"interviewer" | "candidate">("interviewer");
  const [provider, setProvider] = useState(loadProviderPreferences);
  const [secretStatus, setSecretStatus] = useState(DEFAULT_SECRET_STATUS);
  const [answer, setAnswer] = useState("");
  const [answerLanguage, setAnswerLanguage] = useState<AnswerLanguage | "">("");
  const [answerHints, setAnswerHints] = useState("");
  const [activeRequestId, setActiveRequestId] = useState<string | null>(null);
  const activeRequestIdRef = useRef<string | null>(null);
  const [status, setStatus] = useState("准备就绪");
  const [profile, setProfile] = useState("");
  const [jobDescription, setJobDescription] = useState("");
  const [sttConfig, setSttConfig] = useState(loadSttPreferences);
  const [showSettings, setShowSettings] = useState(false);
  const [windowPreferences, setWindowPreferences] = useState(loadWindowPreferences);

  const latestInterviewer = useMemo(
    () => [...transcript].reverse().find((entry) => entry.speaker === "interviewer"),
    [transcript],
  );
  const detection = useMemo(
    () => detectQuestion(partialTranscript?.text ?? latestInterviewer?.text ?? draft),
    [draft, latestInterviewer, partialTranscript],
  );

  useEffect(() => {
    const unlistenLevel = listen<AudioLevelEvent>("audio-level", (event) => {
      setAudioLevel(event.payload);
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
    });
    const unlistenPartial = listen<TranscriptPartialEvent>("transcript-partial", (event) => {
      const payload = event.payload;
      if (deletedTranscriptIdsRef.current.has(payload.id)) return;
      const result = detectQuestion(payload.text);
      setPartialTranscript({
        id: payload.id,
        speaker: payload.speaker,
        text: payload.text,
        language: result.language,
        timestamp: Date.now(),
        isFinal: false,
      });
      setStatus("正在实时识别…");
    });
    const unlistenTranscript = listen<TranscriptFinalEvent>("transcript-final", (event) => {
      const payload = event.payload;
      if (deletedTranscriptIdsRef.current.has(payload.id)) return;
      const result = detectQuestion(payload.text);
      const entry: TranscriptEntry = {
        id: payload.id,
        speaker: payload.speaker,
        text: payload.text,
        language: result.language,
        timestamp: Date.now(),
        isFinal: payload.isFinal,
      };
      setPartialTranscript((current) => (current?.id === payload.id ? null : current));
      setTranscript((current) => {
        const existingIndex = current.findIndex((item) => item.id === payload.id);
        if (existingIndex < 0) return [...current, entry];
        return current.map((item, index) => (index === existingIndex ? entry : item));
      });
      setStatus(result.isQuestion ? "已识别面试问题" : "已完成一段语音识别");
    });
    const unlistenSttStatus = listen<SttStatusEvent>("stt-status", (event) => {
      setStatus(event.payload.message);
    });

    return () => {
      void unlistenLevel.then((fn) => fn());
      void unlistenLlm.then((fn) => fn());
      void unlistenAudioError.then((fn) => fn());
      void unlistenPartial.then((fn) => fn());
      void unlistenTranscript.then((fn) => fn());
      void unlistenSttStatus.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    void invoke<SecretStatus>("get_secret_status")
      .then(setSecretStatus)
      .catch((error) => setStatus(`读取钥匙串状态失败：${String(error)}`));
  }, []);

  useEffect(() => {
    savePublicPreferences(provider, sttConfig);
  }, [
    provider.baseUrl,
    provider.disableThinking,
    provider.model,
    provider.name,
    sttConfig.hotwords,
    sttConfig.mode,
    sttConfig.tencent.appId,
    sttConfig.tencent.secretId,
  ]);

  useEffect(() => {
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
  }, []);

  useEffect(() => {
    document.documentElement.classList.toggle(
      "transparent-mode",
      windowPreferences.alwaysOnTop,
    );
    return () => document.documentElement.classList.remove("transparent-mode");
  }, [windowPreferences.alwaysOnTop]);

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
        await invoke("start_audio_capture", { config: sttConfig });
        setListening(true);
        setStatus("正在监听并识别系统音频");
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
    setTranscript((current) => current.filter((entry) => entry.id !== id));
    setPartialTranscript((current) => (current?.id === id ? null : current));
    setStatus("已删除这条转写");
  }

  function clearTranscript() {
    setTranscript((current) => {
      current.forEach((entry) => deletedTranscriptIdsRef.current.add(entry.id));
      return [];
    });
    setPartialTranscript((current) => {
      if (current) deletedTranscriptIdsRef.current.add(current.id);
      return null;
    });
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
      setStatus("密钥已安全保存到 macOS 钥匙串");
    } catch (error) {
      setStatus(`保存密钥失败：${String(error)}`);
    }
  }

  async function clearStoredSecret(kind: SecretKind) {
    const label = kind === "deepseekApiKey" ? "DeepSeek API Key" : "腾讯云 SecretKey";
    if (!window.confirm(`确定从 macOS 钥匙串删除 ${label}？`)) return;
    try {
      await invoke("delete_secret", { kind });
      setSecretStatus((current) => ({ ...current, [kind]: false }));
      setStatus(`${label} 已从 macOS 钥匙串删除`);
    } catch (error) {
      setStatus(`删除密钥失败：${String(error)}`);
    }
  }

  async function askAi() {
    const question =
      partialTranscript?.text.trim() || latestInterviewer?.text.trim() || draft.trim();
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
        messages: buildInterviewMessages(
          question,
          answerLanguage,
          transcript,
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

  const meterWidth = `${Math.min(100, Math.max(2, audioLevel.rms * 260))}%`;

  return (
    <main className={`app-shell ${windowPreferences.alwaysOnTop ? "focus-mode" : ""}`}>
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark">IC</span>
          <div>
            <h1>Interview Copilot</h1>
            <p>中英实时面试教练 · macOS prototype</p>
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
            <span className="privacy-note">敏感密钥加密保存在 macOS 钥匙串，不提供明文查看</span>
          </div>
          <div className="settings-grid">
            <label className="wide-field">
              语音识别模式
              <select
                value={sttConfig.mode}
                disabled={listening}
                onChange={(event) =>
                  setSttConfig({ ...sttConfig, mode: event.target.value as SttConfig["mode"] })
                }
              >
                <option value="localFast">本地快速 · Zipformer + SenseVoice</option>
                <option value="localAccurate">本地高精度 · Zipformer + Qwen3-ASR 0.6B</option>
                <option value="tencentCloud">腾讯云高精度 · ASR 大模型 2.0</option>
              </select>
              <small className="field-note">
                {sttConfig.mode === "localFast" && "完全离线，资源占用最低。"}
                {sttConfig.mode === "localAccurate" && "完全离线，准确率优先，会占用更多内存。"}
                {sttConfig.mode === "tencentCloud" && "实时上传音频到腾讯云，依赖网络并可能产生费用。"}
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
                disabled={transcript.length === 0 && !partialTranscript}
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

          <div className="audio-meter" aria-label="Audio input level">
            <span style={{ width: meterWidth }} />
          </div>
          <p className="meter-caption">
            {audioLevel.sampleRate
              ? `${audioLevel.sampleRate} Hz · peak ${audioLevel.peak.toFixed(2)}`
              : "等待 Core Audio Tap"}
          </p>

          <div className="transcript-list">
            {transcript.length === 0 && !partialTranscript ? (
              <div className="empty-state">
                <p>播放面试录音后，识别结果会边说边出现，短暂停顿后自动定稿。也可以手动输入进行修正。</p>
              </div>
            ) : (
              <>
                {transcript.map((entry) => (
                  <div className={`transcript-row ${entry.speaker}`} key={entry.id}>
                    <span>{entry.speaker === "interviewer" ? "面试官" : "我"}</span>
                    <p>{entry.text}</p>
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
                {partialTranscript && (
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
                )}
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
            <button className="ask-button" disabled={Boolean(activeRequestId)} onClick={askAi}>
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
    </main>
  );
}

export default App;
