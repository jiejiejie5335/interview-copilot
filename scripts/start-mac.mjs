import { spawn } from "node:child_process";
import { access } from "node:fs/promises";
import { resolve } from "node:path";

if (process.platform !== "darwin") {
  console.error("此命令仅用于 macOS。");
  process.exitCode = 1;
} else {
  const appPath = resolve("src-tauri", "target", "release", "bundle", "macos", "Interview Copilot.app");
  try {
    await access(appPath);
    const child = spawn("open", ["-a", appPath], { stdio: "inherit" });
    child.once("error", (error) => {
      console.error(`无法启动 Interview Copilot：${error.message}`);
      process.exitCode = 1;
    });
    child.once("exit", (code) => {
      process.exitCode = code ?? 1;
    });
  } catch {
    console.error("尚未构建稳定版。请先运行 npm run build:mac，再运行 npm run start:mac。");
    process.exitCode = 1;
  }
}
