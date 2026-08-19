import { createWriteStream, existsSync, mkdirSync, renameSync, statSync } from "node:fs";
import { pipeline } from "node:stream/promises";
import { dirname, resolve } from "node:path";

const modelDirectory = resolve("models");
const modelGroups = [
  {
    name: "sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20",
    repository: "csukuangfj/sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20",
    files: [
      ["encoder-epoch-99-avg-1.int8.onnx", "encoder.int8.onnx", 181_000_000],
      ["decoder-epoch-99-avg-1.onnx", "decoder.onnx", 13_000_000],
      ["joiner-epoch-99-avg-1.int8.onnx", "joiner.int8.onnx", 3_000_000],
      ["tokens.txt", "tokens.txt", 50_000],
      ["bpe.vocab", "bpe.vocab", 10_000],
    ],
  },
  {
    name: "sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25",
    repository: "csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25",
    files: [
      ["conv_frontend.onnx", "conv_frontend.onnx", 44_000_000],
      ["encoder.int8.onnx", "encoder.int8.onnx", 181_000_000],
      ["decoder.int8.onnx", "decoder.int8.onnx", 755_000_000],
      ["tokenizer/merges.txt", "tokenizer/merges.txt", 1_600_000],
      ["tokenizer/tokenizer_config.json", "tokenizer/tokenizer_config.json", 10_000],
      ["tokenizer/vocab.json", "tokenizer/vocab.json", 2_700_000],
    ],
  },
  {
    name: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
    repository: "csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
    files: [
      ["model.int8.onnx", "model.int8.onnx", 239_000_000],
      ["tokens.txt", "tokens.txt", 300_000],
    ],
  },
];

function groupIsComplete(group) {
  const installedDirectory = resolve(modelDirectory, group.name);
  return group.files.every(([, targetName, minimumSize]) => {
    const path = resolve(installedDirectory, targetName);
    return existsSync(path) && statSync(path).size >= minimumSize;
  });
}

async function installGroup(group) {
  const installedDirectory = resolve(modelDirectory, group.name);
  mkdirSync(installedDirectory, { recursive: true });
  if (groupIsComplete(group)) return;
  console.log(`正在准备 ${group.name}…`);
  const modelBaseUrl = `https://huggingface.co/${group.repository}/resolve/main`;
  for (const [sourceName, targetName, minimumSize] of group.files) {
    const targetPath = resolve(installedDirectory, targetName);
    if (existsSync(targetPath) && statSync(targetPath).size >= minimumSize) continue;
    mkdirSync(dirname(targetPath), { recursive: true });
    const temporaryPath = `${targetPath}.download`;
    if (existsSync(temporaryPath) && statSync(temporaryPath).size >= minimumSize) {
      renameSync(temporaryPath, targetPath);
      continue;
    }
    const downloadedBytes = existsSync(temporaryPath) ? statSync(temporaryPath).size : 0;
    console.log(
      downloadedBytes > 0
        ? `  续传 ${targetName}（已有 ${Math.round(downloadedBytes / 1024 / 1024)} MiB）…`
        : `  下载 ${targetName}…`,
    );
    const response = await fetch(`${modelBaseUrl}/${sourceName}`, {
      headers: downloadedBytes > 0 ? { Range: `bytes=${downloadedBytes}-` } : {},
    });
    if (!response.ok || !response.body) {
      throw new Error(`${targetName} 下载失败：HTTP ${response.status}`);
    }
    const append = downloadedBytes > 0 && response.status === 206;
    await pipeline(response.body, createWriteStream(temporaryPath, { flags: append ? "a" : "w" }));
    if (statSync(temporaryPath).size < minimumSize) {
      throw new Error(`${targetName} 下载不完整`);
    }
    renameSync(temporaryPath, targetPath);
  }
  if (!groupIsComplete(group)) throw new Error(`${group.name} 模型文件不完整`);
}

if (process.platform !== "darwin") {
  throw new Error("当前自动安装脚本仅支持 macOS");
}

for (const group of modelGroups) await installGroup(group);

console.log("STT 已准备完成：Zipformer + SenseVoice + Qwen3-ASR 0.6B");
