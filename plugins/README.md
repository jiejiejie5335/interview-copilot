# Tauri Plugin ios-ambient-audio

Private application plugin used by Interview Copilot's iPhone companion mode.

- Requests microphone permission.
- Captures mono Float32 PCM through `AVAudioEngine`.
- Pushes buffers into the main Rust crate through `interview_audio_push_ffi`.
- Keeps the screen awake only while capture is active.

The plugin deliberately does not send network requests and does not persist raw
audio. Tencent ASR transport remains in the application's Rust layer.
