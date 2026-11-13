# Interview Copilot

本地优先的 macOS / iPhone 技术面试练习与经授权实时辅助工具。当前阶段聚焦：

- Tauri 2 + React 桌面壳
- macOS 系统音频与麦克风双通道采集（Core Audio Tap + 主进程麦克风）
- Zoom/蓝牙设备切换容错：双通道独立运行，单路中断会自动退避重连
- 中文、英文及中英混合问题检测
- DeepSeek / OpenAI-compatible 流式回答
- 可关闭的英文转中文翻译：DeepSeek 云端或 Ollama 本地
- 可切换 STT：SenseVoice 本地快速、Qwen3-ASR 本地高精度、腾讯云 ASR 2.0
- 可替换的 RAG 接口
- iPhone 横屏环境监听与手动问题边界（`feature/ios-manual-companion`）
- 可选的局域网伴随端：将 AI 问题与回答流式同步到 iPhone Safari
- macOS 全局生成快捷键：在 Zoom 等应用位于前台时按 `⌘ + Shift + Enter`

> 仅应在参与者知情并允许录音、转写和 AI 辅助的场景中使用。项目不实现屏幕共享规避或应用伪装。

## 当前里程碑

当前版本包含可运行 UI、macOS 系统音频与默认麦克风双路 PCM 捕获、按角色合并的对话时间线、Zipformer 中英文流式 partial 转写、SenseVoice/Qwen3-ASR 后台 final 校正、腾讯云 ASR V2 WebSocket、技术热词别名修正、繁简体/英文/混合语问题检测和 OpenAI-compatible SSE 流式 Provider。系统音频标记为“面试官”，麦克风标记为“我”；只有面试官内容参与问题检测，但双方最近对话都会进入模型上下文。也可以手动加入或删除转写进行修正。

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
- `腾讯云高精度`：腾讯云实时 ASR V2 `16k_zh_en_2.0`，使用 16 kHz PCM、科技领域和临时热词表。需要在设置中填写 AppID、SecretId、SecretKey；SecretKey 安全保存在 macOS 钥匙串，不写入项目文件。

腾讯云模式会为系统声音和麦克风分别建立实时识别连接，并将两路音频发送到腾讯云，因此用量可能接近单通道的两倍。使用前请开通对应的大模型 2.0 计费服务，并确保参与者知情同意。

项目的 Tauri 启动脚本会自动把 Rustup 的 Cargo 目录加入子进程 `PATH`，并在退出时清理它启动的 Vite/Cargo 进程，因此安装 Rust 后无需重开终端，也不会让开发端口被残留进程占用。如果希望在当前终端直接使用 `cargo` 命令，可另外执行 `source "$HOME/.cargo/env"`。

首次点击“开始监听”时，macOS 会请求“系统音频录制”和“麦克风”权限。若拒绝，可在“系统设置 → 隐私与安全性”中的“屏幕与系统音频录制”及“麦克风”重新授权。面试时建议佩戴耳机，避免扬声器内容再次进入麦克风形成重复转写。

DeepSeek 默认配置：

- Base URL: `https://api.deepseek.com`
- Model: `deepseek-v4-flash`
- Thinking: disabled（降低实时回答延迟）

DeepSeek API Key 和腾讯云 SecretKey 均保存在 macOS 钥匙串中，界面只显示是否已保存，不会回显明文，也不会提交到 Git。

应用启动时只查询钥匙串条目是否存在，不读取密钥内容，因此不会因为显示“已保存”而弹出授权窗口。密钥在第一次实际调用对应服务时读取，并缓存在当前应用进程内。

日常使用时，建议首次运行 `npm run build:mac` 构建固定的 macOS 应用，以后用 `npm run start:mac` 启动同一份应用，不要每次运行 `npm run tauri dev`。首次读取已保存的密钥时，macOS 可能要求授权一次；应用文件未变化时，“始终允许”通常会保持有效。`tauri dev` 在 Rust 代码变动后会重新编译调试二进制，其临时签名随之改变，钥匙串可能再次要求密码。若之后需要频繁构建仍保留授权，应使用稳定的 Apple Development 代码签名身份；当前机器没有可用的代码签名证书。

## iPhone 横屏环境监听 MVP

iPhone 版本作为放在电脑旁边的伴随终端：电脑继续运行视频面试，iPhone 麦克风持续采集电脑扬声器和候选人的现场声音。首版固定使用腾讯云实时 ASR，不在手机上加载桌面端的大型本地模型。

底部控制区采用手动角色边界：

- `问题结束`：额外收取 450 ms 尾音，结束当前问题并立即请求回答建议；之后的环境语音标记为“我”。
- `问题追加`：将最近 3.5 秒环境转写回溯为面试官内容，并继续追加；再次点击“问题结束”后刷新回答建议。
- `回答结束`：完成候选人回答，下一段环境语音重新标记为面试官的新问题。

音频由本地 Tauri Swift 插件中的 `AVAudioEngine` 采集，经 FFI 直接进入 Rust 的有界队列和现有腾讯云 WebSocket，不把 PCM 发送到 WebView。监听期间会阻止 iPhone 自动锁屏；停止监听后恢复系统锁屏设置。DeepSeek API Key 和腾讯云 SecretKey 使用 iOS Keychain 保存。

使用时请让电脑通过扬声器播放面试声音，并将 iPhone 横放在扬声器附近。完全封闭的耳机不会把面试官声音播放到环境中，因此不适用于这个模式。

### iOS 开发环境

iOS 构建必须安装完整 Xcode，只有 Xcode Command Line Tools 不够。首次配置：

```bash
sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
rustup target add aarch64-apple-ios x86_64-apple-ios aarch64-apple-ios-sim
brew install cocoapods xcodegen
npm install
npm run tauri ios init -- --ci
```

然后在 `src-tauri/tauri.ios.conf.json` 的 `bundle.iOS.developmentTeam` 中填写 Apple Developer Team ID，或者在当前终端设置 `APPLE_DEVELOPMENT_TEAM`。连接并信任 iPhone 后运行：

```bash
npm run tauri ios dev
```

首次开始监听时允许麦克风权限。横屏界面也可以先在桌面开发窗口预览：

```text
http://localhost:1420/?mobilePreview=1
```

当前开发机若缺少完整 Xcode、`xcodegen` 或 CocoaPods，`tauri ios init` 会停止；安装上述依赖后重新执行即可，不需要删除现有代码。

### 将 Mac 回答同步到 iPhone

桌面端设置中可以手动开启“同步回答到手机”。应用会在 Mac 本机启动一个临时 HTTP/SSE 服务并生成随机会话地址；让 iPhone 和 Mac 连接同一个 Wi-Fi，在 Safari 打开该地址，即可在生成前看到面试官实时转写，并在生成后看到本次固定问题和逐步输出的 AI 回答。手机页面底部固定提供“生成建议”按钮，点击后会让 Mac 使用手机当前显示的这一轮面试官转写发起生成；生成期间按钮会自动禁用，避免重复请求。

macOS 上也可以在 Zoom、腾讯会议或浏览器位于前台时按 `⌘ + Shift + Enter`，使用当前这一轮面试官转写生成建议。应用只响应按下动作，并在 700 毫秒内忽略重复触发；如果快捷键已被其他程序占用，回答建议区域会显示注册失败原因。

该功能默认关闭，不使用第三方转发服务器，也不会发送音频、API Key 或完整简历。同步地址只在本次应用进程内有效；关闭同步或退出应用后失效。由于局域网传输使用 HTTP，请只在可信 Wi-Fi 中开启。

如果手机无法打开地址，请先确认链接中的 IP 与 Mac 的 Wi-Fi 地址一致（通常为 `192.168.x.x` 或 `10.x.x.x`），并关闭会接管局域网路由的 VPN/代理。应用会优先选择 macOS 的 `en0` 私有地址并排除 `utun` 等 VPN 接口；修改网络后需要关闭并重新开启同步以生成新地址。

### 英语面试中文翻译

在“设置”中打开“英语面试实时中文翻译”，然后选择：

- `DeepSeek 云端翻译`：复用回答 Provider 的模型和 API Key，只发送面试官的英文转写文字，不发送音频。
- `Ollama 本地翻译`：文字不离开本机。安装并启动 Ollama 后，先准备默认模型：

```bash
ollama pull qwen2.5:3b
```

也可以在设置中填写其他 Ollama 模型。翻译只在英文或中英混合的面试官 final 转写定稿后触发，原文始终保留；翻译失败可以在对应转写下直接重试。关闭开关后不会发起新的翻译请求，也不会显示译文。

## 目录

- `src/`：React UI、双语问题检测、提示词和 Provider 前端边界
- `src-tauri/src/commands/provider.rs`：OpenAI-compatible / DeepSeek SSE 流式适配
- `src-tauri/audio-helper/SystemAudioTap.swift`：macOS 系统音频 Process Tap 与默认麦克风采集
- `src-tauri/src/audio/macos.rs`：Float32 PCM 音频 helper 生命周期
- `plugins/`：iOS `AVAudioEngine` 环境麦克风 Tauri 插件
- `src-tauri/src/stt/`：sherpa-onnx 流式识别、endpoint 定稿与音频回归工具
- `docs/ARCHITECTURE.md`：组件边界和下一阶段 STT 接入点
