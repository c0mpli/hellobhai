#!/bin/bash
# Type-checks the app for Linux from a Mac without linking:
#   cargo check --target x86_64-unknown-linux-gnu
# The GTK, WebKitGTK and ALSA sys crates ask pkg-config for their libraries,
# which a Mac doesn't have for Linux, so a check-only stub that says yes to
# everything is put on PATH. Never use the stub for real builds; those run on
# Linux (CI). Needs: rustup target add x86_64-unknown-linux-gnu, Homebrew
# openssl@3 (or OPENSSL_DIRECTORY) and Xcode's clang.
set -euo pipefail

tauri_directory="$(cd "$(dirname "$0")/.." && pwd)"
stub_bin_directory="$(mktemp -d)"
trap 'rm -rf "$stub_bin_directory"' EXIT
cp "$tauri_directory/scripts/stub-pkg-config" "$stub_bin_directory/pkg-config"
chmod +x "$stub_bin_directory/pkg-config"

# openssl-sys (native-tls on Linux) reads OpenSSL's version header with a C
# preprocessor: clang aimed at Linux plus the Homebrew OpenSSL headers is enough.
openssl_directory="${OPENSSL_DIRECTORY:-$(brew --prefix openssl@3 2>/dev/null || echo /opt/homebrew/opt/openssl@3)}"

cd "$tauri_directory/src-tauri"
PATH="$stub_bin_directory:$PATH" PKG_CONFIG="$stub_bin_directory/pkg-config" PKG_CONFIG_ALLOW_CROSS=1 \
    CC_x86_64_unknown_linux_gnu=clang CFLAGS_x86_64_unknown_linux_gnu="--target=x86_64-unknown-linux-gnu" \
    X86_64_UNKNOWN_LINUX_GNU_OPENSSL_INCLUDE_DIR="$openssl_directory/include" \
    X86_64_UNKNOWN_LINUX_GNU_OPENSSL_LIB_DIR="$openssl_directory/lib" \
    cargo check --target x86_64-unknown-linux-gnu "$@"
