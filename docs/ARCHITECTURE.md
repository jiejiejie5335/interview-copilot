# Architecture

## Current MVP

```text
macOS system audio
  -> Swift Core Audio Process Tap helper
  -> mono Float32 PCM
  -> Rust 16 kHz resampler + energy VAD
  -> whisper.cpp multilingual small (language=auto)
  -> transcript-final Tauri event
  -> zh / en / mixed question detector
  -> interview prompt builder
  -> Rust OpenAI-compatible provider
  -> DeepSeek SSE stream
  -> Tauri llm-stream event
  -> React answer coach
```

Audio levels are calculated from the same PCM stream and emitted separately for the React meter. The API key crosses the Tauri IPC boundary for a single request and is held in memory only. Network requests originate in Rust, so the webview does not need broad outbound CSP access.

## STT boundary

The audio helper emits raw PCM while Rust creates utterance-sized 16 kHz segments. The current transcription service uses this conceptual provider boundary:

```text
trait Transcriber
  transcribe(chunk, languageHint = auto) -> partial/final transcript
```

Routing policy:

- `auto` is always the UI default; users do not need to choose Chinese or English in advance.
- Current engine: whisper.cpp multilingual small with automatic language detection.
- Planned Chinese accuracy option: sherpa-onnx/FunASR.
- Final transcripts pass through the existing `zh | en | mixed | unknown` detector.

## Planned boundaries

- `Transcriber`: current whisper.cpp worker; sherpa-onnx/FunASR later.
- `Retriever`: resume/JD chunks with local embeddings and citations.
- `ChatProvider`: current OpenAI-compatible implementation plus Ollama.
- `AudioCapture`: current macOS Process Tap; Windows WASAPI Loopback later.
