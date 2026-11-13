import { spawn } from "node:child_process";
import { resolve } from "node:path";

if (process.platform !== "darwin") {
  console.error("此命令仅用于 macOS。");
  process.exit(1);
}

const projectRoot = resolve(import.meta.dirname, "..");
const appPath = resolve(projectRoot, "src-tauri", "target", "release", "bundle", "macos", "Interview Copilot.app");

function run(command, args) {
  return new Promise((resolveRun, reject) => {
    const child = spawn(command, args, { cwd: projectRoot, stdio: "inherit" });
    child.once("error", reject);
    child.once("exit", (code, signal) => {
      if (code === 0) resolveRun();
      else reject(new Error(`${command} 退出：${signal ?? code}`));
    });
  });
}

try {
  await run(process.execPath, [resolve(projectRoot, "scripts", "run-tauri.mjs"), "build", "--bundles", "app"]);
  // Tauri can leave the bundle with only its embedded executable signed.
  // Sign the complete .app so Keychain sees one consistent app identity.
  await run("codesign", ["--force", "--deep", "--sign", "-", appPath]);
  await run("codesign", ["--verify", "--deep", "--strict", appPath]);
  console.log(`已构建并验证：${appPath}`);
} catch (error) {
  console.error(`macOS 构建失败：${error.message}`);
  process.exitCode = 1;
}
