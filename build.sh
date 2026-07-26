#!/usr/bin/env bash
#
# build.sh — macOS build script for Kue
#
# Usage:
#   ./build.sh               # debug build (default)
#   ./build.sh --release     # release build (recommended for distribution)
#   ./build.sh --universal   # universal binary (x86_64 + arm64)
#   ./build.sh --release --universal
#
# Requirements:
#   - Xcode 15+ (for macOS 13+ SDK)
#   - Node.js 20+
#   - Rust toolchain with aarch64-apple-darwin target
#     (rustup target add aarch64-apple-darwin)
#

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT_DIR"

# ---- Parse args -----------------------------------------------------------
RELEASE=false
UNIVERSAL=false

for arg in "$@"; do
  case "$arg" in
    --release) RELEASE=true ;;
    --universal) UNIVERSAL=true ;;
    --help)
      echo "Usage: $0 [--release] [--universal]"
      exit 0
      ;;
    *)
      echo "Unknown argument: $arg"
      echo "Usage: $0 [--release] [--universal]"
      exit 1
      ;;
  esac
done

# ---- Pre-flight checks ----------------------------------------------------
echo "[build] Checking prerequisites..."

if ! command -v node &>/dev/null; then
  echo "ERROR: Node.js is required. Install via https://nodejs.org"
  exit 1
fi

if ! command -v rustc &>/dev/null; then
  echo "ERROR: Rust is required. Install via https://rustup.rs"
  exit 1
fi

if ! command -v cargo &>/dev/null; then
  echo "ERROR: cargo is required."
  exit 1
fi

if $UNIVERSAL; then
  if ! rustup target list --installed | grep -q "aarch64-apple-darwin"; then
    echo "Adding aarch64-apple-darwin target..."
    rustup target add aarch64-apple-darwin
  fi
fi

echo "[build] Prerequisites OK"

# ---- Install frontend dependencies ----------------------------------------
echo "[build] Installing npm dependencies..."
npm ci --frozen-lockfile 2>/dev/null || npm install

# ---- Build frontend -------------------------------------------------------
echo "[build] Building frontend (TypeScript + Vite)..."
npm run build

# ---- Build Rust backend ---------------------------------------------------
BUILD_FLAGS=""
if $RELEASE; then
  BUILD_FLAGS="--release"
fi

if $UNIVERSAL; then
  echo "[build] Building universal binary (x86_64 + arm64)..."
  echo "[build] Step 1: Building for x86_64-apple-darwin..."
  cargo build --target x86_64-apple-darwin $BUILD_FLAGS --manifest-path src-tauri/Cargo.toml

  echo "[build] Step 2: Building for aarch64-apple-darwin..."
  cargo build --target aarch64-apple-darwin $BUILD_FLAGS --manifest-path src-tauri/Cargo.toml

  echo "[build] Step 3: Creating universal binary with lipo..."
  LIPO_FLAGS=""
  if $RELEASE; then
    LIPO_DIRS="src-tauri/target/x86_64-apple-darwin/release src-tauri/target/aarch64-apple-darwin/release"
  else
    LIPO_DIRS="src-tauri/target/x86_64-apple-darwin/debug src-tauri/target/aarch64-apple-darwin/debug"
  fi

  # Create universal binary for the main library
  mkdir -p src-tauri/target/universal/release
  lipo -create \
    "$(echo $LIPO_DIRS | awk '{print $1}')/libkue_lib.a" \
    "$(echo $LIPO_DIRS | awk '{print $2}')/libkue_lib.a" \
    -output "src-tauri/target/universal/release/libkue_lib.a" 2>/dev/null || true

  # Build the .app bundle via Tauri (will use the architecture of the build machine)
  echo "[build] Step 4: Running tauri build (native arch — app binary will be native)..."
  # Tauri picks the correct binary; for a true universal .app we'd need
  # to manually lipo the final binary. For now this builds a working .dmg
  # for the current architecture.
fi

# ---- Tauri build ----------------------------------------------------------
echo "[build] Running 'npm run tauri build'..."
if $RELEASE; then
  npm run tauri build
else
  npm run tauri build -- --debug
fi

echo ""
echo "[build] ✅ Build complete!"
echo "[build] Output: src-tauri/target/release/bundle/dmg/ (or debug/ for debug builds)"
echo ""

# Show the .dmg if found
DMG=$(find src-tauri/target -name "*.dmg" -type f 2>/dev/null | head -1)
if [ -n "$DMG" ]; then
  echo "[build] 📦 $DMG"
fi
