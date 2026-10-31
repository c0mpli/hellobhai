#!/bin/bash
# Type-checks the app for Windows from a Mac (or Linux) without linking:
#   cargo check --target x86_64-pc-windows-msvc
# tauri-build compiles the Windows resource file (icon, manifest) with llvm-rc,
# which is not installed here, so a check-only stub that writes an empty .res is
# put on PATH. Never use the stub for real builds; those run on Windows (CI).
# Needs: rustup target add x86_64-pc-windows-msvc
set -euo pipefail

tauri_directory="$(cd "$(dirname "$0")/.." && pwd)"
stub_bin_directory="$(mktemp -d)"
trap 'rm -rf "$stub_bin_directory"' EXIT
cp "$tauri_directory/scripts/stub-llvm-rc" "$stub_bin_directory/llvm-rc"
chmod +x "$stub_bin_directory/llvm-rc"

cd "$tauri_directory/src-tauri"
PATH="$stub_bin_directory:$PATH" cargo check --target x86_64-pc-windows-msvc "$@"
