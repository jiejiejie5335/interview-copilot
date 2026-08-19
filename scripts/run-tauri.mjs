import { spawn } from "node:child_process";
import { homedir } from "node:os";
import { delimiter, join, resolve } from "node:path";

const cargoHome = process.env.CARGO_HOME || join(homedir(), ".cargo");
const cargoBin = join(cargoHome, "bin");
const executable = resolve(
  "node_modules",
  ".bin",
  process.platform === "win32" ? "tauri.cmd" : "tauri",
);
const pathValue = [cargoBin, process.env.PATH].filter(Boolean).join(delimiter);
const usesProcessGroup = process.platform !== "win32";

const child = spawn(executable, process.argv.slice(2), {
  cwd: process.cwd(),
  env: { ...process.env, PATH: pathValue },
  stdio: "inherit",
  // Give Tauri and every process it starts (Vite, Cargo and the app) their own
  // process group. This lets us reliably stop the whole dev tree on macOS.
  detached: usesProcessGroup,
});

let childExited = false;

function stopChildTree(signal) {
  try {
    if (usesProcessGroup && child.pid) {
      process.kill(-child.pid, signal);
    } else if (!child.killed) {
      child.kill(signal);
    }
  } catch (error) {
    // ESRCH only means the process tree has already stopped.
    if (error?.code !== "ESRCH") {
      console.error(`无法停止 Tauri 开发进程：${error.message}`);
    }
  }
}

function handleSignal(signal) {
  if (childExited) {
    return;
  }
  stopChildTree(signal);
}

for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.once(signal, () => handleSignal(signal));
}

child.once("error", (error) => {
  console.error(`无法启动 Tauri CLI：${error.message}`);
  process.exitCode = 1;
});

child.once("exit", (code, signal) => {
  childExited = true;
  // Tauri may exit before its beforeDevCommand child. Clean up any remaining
  // Vite/Cargo process in the group so the next run can reuse port 1420.
  stopChildTree("SIGTERM");
  process.exitCode = code ?? (signal === "SIGINT" ? 130 : 1);
});
