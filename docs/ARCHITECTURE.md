# Architecture

## Current MVP

```text
macOS system audio -> Core Audio Process Tap -> interviewer PCM -> interviewer STT
default microphone -> AVAudioEngine input tap -> candidate PCM -> candidate STT
                                                       |
                                                       v
                                   timestamped role-aware transcript
                                                       |
                         interviewer question detector + recent dialogue context
                                                       |
                              Rust OpenAI-compatible provider -> DeepSeek SSE
                                                       |
                                              React answer coach

final interviewer English/mixed transcript
  -> translation switch
     -> DeepSeek non-streaming chat completion
     -> Ollama local /api/chat
  -> Chinese reference shown below the preserved source text
```

The two mono Float32 PCM streams have independent queues, recognizers and level events. This preserves speaker attribution during brief overlap. Only interviewer text becomes the latest question; final interviewer and candidate turns are both included in the prompt so follow-up questions can be interpreted using the candidate's previous answer. Sensitive provider credentials are loaded from macOS Keychain only when needed. Network requests originate in Rust, so the webview does not need broad outbound CSP access.

Translation is deliberately separate from answer generation. Each finalized English or mixed interviewer entry has its own non-streaming translation request and status, so translation completion or failure cannot overwrite an active answer stream. DeepSeek reuses the configured provider and Keychain credential; Ollama uses its native local `POST /api/chat` endpoint.

## iPhone companion MVP

```text
laptop speakers + candidate voice
              |
              v
iPhone AVAudioEngine ambient microphone tap
              |
              v (Float32 FFI)
Rust bounded audio queue -> Tencent ASR V2 (single session)
              |
              v
manual role state: interviewer <-> candidate
              |
     question end / question append
              |
              v
DeepSeek answer stream + full dialogue context
```

The iOS path intentionally uses one microphone and one cloud recognizer. Ambient transcript IDs receive a stable role when their first partial event arrives. Manual controls change the role for future IDs; “question append” additionally rolls recent ambient IDs back to interviewer so a late tap does not lose the beginning of a supplement. The desktop path remains dual-channel and does not use this state machine.

## STT boundary

The audio helper emits raw PCM. Each source is routed to the selected provider with a fixed role:

```text
trait Transcriber
  transcribe(chunk, languageHint = auto) -> partial/final transcript
```

Routing policy:

- `auto` is always the UI default; users do not need to choose Chinese or English in advance.
- Local fast engine: streaming bilingual Zipformer plus SenseVoice final refinement.
- Local accurate engine: streaming bilingual Zipformer plus Qwen3-ASR final refinement.
- Cloud engine: two Tencent ASR V2 WebSocket sessions using `16k_zh_en_2.0`.
- Final transcripts pass through the existing `zh | en | mixed | unknown` detector.

## Planned boundaries

- `Transcriber`: current sherpa-onnx local workers and Tencent cloud worker.
- `Retriever`: resume/JD chunks with local embeddings and citations.
- `ChatProvider`: current OpenAI-compatible implementation plus Ollama.
- `AudioCapture`: current macOS Process Tap + AVAudioEngine microphone; Windows WASAPI Loopback later.
