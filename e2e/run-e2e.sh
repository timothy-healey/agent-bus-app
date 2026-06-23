#!/usr/bin/env bash
# S4: build the debug Tauri app, then run the WebDriver E2E suite.
# Linux / Windows (WSL / git-bash) ONLY — tauri-driver does NOT support macOS.
set -euo pipefail

cd "$(dirname "$0")"

OS="$(uname -s 2>/dev/null || echo unknown)"
case "$OS" in
  Darwin)
    echo "ERROR: tauri-driver does not support macOS (WKWebView has no WebDriver)." >&2
    echo "Run this on Linux (webkit2gtk-driver) or Windows (msedgedriver). See README.md." >&2
    exit 2
    ;;
esac

if ! command -v tauri-driver >/dev/null 2>&1; then
  echo "ERROR: tauri-driver not found. Install: cargo install tauri-driver --locked" >&2
  echo "Plus platform WebDriver deps (Linux: webkit2gtk-driver, xvfb). See README.md." >&2
  exit 3
fi

echo "==> Building the debug Tauri binary (src-tauri/) ..."
( cd ../src-tauri && cargo build )

echo "==> Installing e2e deps (if needed) ..."
if [ ! -d node_modules ]; then
  if command -v bun >/dev/null 2>&1; then bun install; else npm install; fi
fi

echo "==> Running WebdriverIO E2E ..."
# On a headless Linux CI, wrap with xvfb-run:  xvfb-run -a ./run-e2e.sh
if command -v bun >/dev/null 2>&1; then
  bun run test:e2e
else
  npm run test:e2e
fi
