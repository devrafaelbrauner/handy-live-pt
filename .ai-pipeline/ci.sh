#!/usr/bin/env bash
# CI gate for Handy Live PT (fork of cjpais/handy).
# NOTE: this repo uses bun (NOT npm) and the Rust workspace lives in src-tauri/.
# On GitHub runners the system dependencies (webkit2gtk, vulkan headers, glslc,
# bun, rust) are provisioned by .github/workflows/ai-pipeline.yml before this
# script runs. Locally on macOS, prerequisites are as in BUILD.md.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

if ! command -v bun >/dev/null 2>&1; then
  echo "bun is required (this repo does not use npm; postinstall calls bun directly)." >&2
  exit 2
fi

# --- Frontend (bun) ---
bun install
bun run lint
bun run build
bun run format:check
bun run check:translations
bun run check:model-languages

# --- Backend (Rust in src-tauri) ---
cargo test --manifest-path src-tauri/Cargo.toml

git diff --check

echo "CI gate: all checks passed."
