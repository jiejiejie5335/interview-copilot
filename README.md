# Interview Copilot

本地优先的 macOS 技术面试练习与经授权实时辅助工具。当前阶段聚焦：

- Tauri 2 + React 桌面壳
- macOS Core Audio Process Tap 系统音频采集（原生 Swift helper）
- 中文、英文及中英混合问题检测
- DeepSeek / OpenAI-compatible 流式回答
- 可切换 STT：SenseVoice 本地快速、Qwen3-ASR 本地高精度、腾讯云 ASR 2.0
- 可替换的 RAG 接口

> 仅应在参与者知情并允许录音、转写和 AI 辅助的场景中使用。项目不实现屏幕共享规避或应用伪装。

## 当前里程碑

第一阶段包含可运行 UI、macOS 系统音频 PCM 捕获、Zipformer 中英文流式 partial 转写、SenseVoice/Qwen3-ASR 后台 final 校正、腾讯云 ASR V2 WebSocket、技术热词别名修正、繁简体/英文/混合语问题检测和 OpenAI-compatible SSE 流式 Provider。也可以手动加入转写进行修正。简历 RAG、持久化和麦克风输入将在下一阶段接入。

音频 helper 复用了 Meetily 的 Process Tap 生命周期思路，并参考 AudioCap 改写为 Swift。这样只需要 Xcode Command Line Tools，不要求安装完整 Xcode。当前最低系统版本为 macOS 14.4。

## 开发

需要 Node.js 20+、Rust stable、Xcode Command Line Tools。

```bash
npm install
npm run setup:stt
npm run test
npm run tauri dev
```

`setup:stt` 会从官方模型仓库下载约 1.4 GiB 的中英双语 streaming Zipformer INT8、SenseVoice INT8 与 Qwen3-ASR 0.6B INT8，模型不会提交到 Git。本地模式下，Zipformer 持续输出低延迟 partial transcript，选定的最终模型在独立线程中生成 final transcript，不会阻塞音频采集。

设置里的“技术热词 / 常见误识别”支持普通热词，也支持 `标准词=误识别1|误识别2` 格式。例如 `Golang=高粱|高亮`。实时结果和最终结果都会应用别名修正；如果 SenseVoice 丢失了更多技术热词，系统会保留信息更完整的 Zipformer 结果。

### 识别模式

- `本地快速`：Zipformer partial + SenseVoice final，完全离线且内存占用较低。
- `本地高精度`：Zipformer partial + Qwen3-ASR 0.6B final，完全离线，默认模式。
- `腾讯云高精度`：腾讯云实时 ASR V2 `16k_zh_en_2.0`，使用 16 kHz PCM、科技领域和临时热词表。需要在设置中填写 AppID、SecretId、SecretKey；凭据只保存在当前前端/后端进程内存，不写入项目文件。

腾讯云模式会将捕获的系统音频实时发送到腾讯云，可能产生费用。使用前请开通对应的大模型 2.0 计费服务，并确保参与者知情同意。

项目的 Tauri 启动脚本会自动把 Rustup 的 Cargo 目录加入子进程 `PATH`，并在退出时清理它启动的 Vite/Cargo 进程，因此安装 Rust 后无需重开终端，也不会让开发端口被残留进程占用。如果希望在当前终端直接使用 `cargo` 命令，可另外执行 `source "$HOME/.cargo/env"`。

首次点击“开始监听”时，macOS 会请求“系统音频录制”权限。若拒绝，可在“系统设置 → 隐私与安全性 → 屏幕与系统音频录制”中重新授权。

DeepSeek 默认配置：

- Base URL: `https://api.deepseek.com`
- Model: `deepseek-v4-flash`
- Thinking: disabled（降低实时回答延迟）

API Key 当前只保存在前端内存中，不写入磁盘。后续版本将接入 macOS Keychain。

## 目录

- `src/`：React UI、双语问题检测、提示词和 Provider 前端边界
- `src-tauri/src/commands/provider.rs`：OpenAI-compatible / DeepSeek SSE 流式适配
- `src-tauri/audio-helper/SystemAudioTap.swift`：macOS 系统音频 Process Tap
- `src-tauri/src/audio/macos.rs`：Float32 PCM 音频 helper 生命周期
- `src-tauri/src/stt/`：sherpa-onnx 流式识别、endpoint 定稿与音频回归工具
- `docs/ARCHITECTURE.md`：组件边界和下一阶段 STT 接入点
