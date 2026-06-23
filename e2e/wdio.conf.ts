import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import path from "node:path";
import os from "node:os";
import { fileURLToPath } from "node:url";

// __dirname is not defined under ESM; derive it from import.meta.url.
const __dirname = path.dirname(fileURLToPath(import.meta.url));

// The built DEBUG binary. `cargo build` (no --release) under src-tauri/ emits it
// here. productName "agent-bus-app" (src-tauri/app/tauri.conf.json).
const BIN_NAME = os.platform() === "win32" ? "agent-bus-app.exe" : "agent-bus-app";
const APP_BINARY = path.resolve(
  __dirname,
  "..",
  "src-tauri",
  "target",
  "debug",
  BIN_NAME,
);

let tauriDriver: ChildProcess | undefined;

export const config: WebdriverIO.Config = {
  runner: "local",
  tsConfigPath: path.resolve(__dirname, "tsconfig.json"),

  specs: ["./specs/**/*.e2e.ts"],
  maxInstances: 1, // a desktop app: one window, serial specs.

  // tauri-driver listens on 4444 and proxies to the native WebDriver
  // (Linux: WebKitWebDriver; Windows: Edge WebDriver).
  hostname: "127.0.0.1",
  port: 4444,
  path: "/",

  capabilities: [
    {
      // tauri-driver reads tauri:options to launch the app binary.
      "tauri:options": {
        application: APP_BINARY,
      },
      // Required placeholder; tauri-driver fills in the real browser cap per-platform.
      browserName: "wry",
    } as WebdriverIO.Capabilities,
  ],

  logLevel: "info",
  bail: 0,
  waitforTimeout: 10_000,
  connectionRetryTimeout: 120_000,
  connectionRetryCount: 3,

  framework: "mocha",
  reporters: ["spec"],
  mochaOpts: {
    ui: "bdd",
    timeout: 120_000, // first launch + template seed can be slow on cold CI.
  },

  // Spawn tauri-driver as the WebDriver intermediary before the session.
  onPrepare: () => {
    // HONESTY GUARD (vet F4): tauri-driver has no macOS backend — WKWebView
    // exposes no WebDriver automation surface. Refuse loudly rather than
    // produce a confusing session failure.
    if (os.platform() === "darwin") {
      throw new Error(
        "S4: tauri-driver does not support macOS (WKWebView has no WebDriver). " +
          "Run this harness on Linux (WebKitWebDriver) or Windows (Edge WebDriver). " +
          "See e2e/README.md.",
      );
    }
    // Fail early with a clear message if tauri-driver isn't installed.
    const probe = spawnSync("tauri-driver", ["--help"], { stdio: "ignore" });
    if (probe.error) {
      throw new Error(
        "S4: `tauri-driver` not found on PATH. Install it with " +
          "`cargo install tauri-driver --locked` plus the platform WebDriver deps " +
          "(Linux: webkit2gtk-driver + xvfb; Windows: msedgedriver). See e2e/README.md.",
      );
    }
    tauriDriver = spawn("tauri-driver", [], {
      stdio: [null, process.stdout, process.stderr],
    });
  },

  onComplete: () => {
    tauriDriver?.kill();
  },
};
